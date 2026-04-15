use anyhow::{Context, Result};
use std::path::PathBuf;

use crate::migrate;
use crate::schema_builder::SchemaBuilderConfig;
use crate::surreal_client::{MigrationDB, SurrealClient};

use super::engine::{
    ApplyResult, CreateResult, MigrationEngine, MigrationInfo, MigrationState,
};

/// Legacy migration engine wrapping the existing `migrate.rs` functions.
///
/// This adapter delegates to the original migration code with zero behavioral
/// changes, translating results into the generic `MigrationEngine` types.
pub struct LegacyEngine {
    url: String,
    namespace: String,
    database: String,
    username: String,
    password: String,
    migrations_dir: PathBuf,
    builder_config: Option<SchemaBuilderConfig>,
}

impl LegacyEngine {
    pub fn new(
        url: String,
        namespace: String,
        database: String,
        username: String,
        password: String,
        migrations_dir: PathBuf,
        builder_config: Option<SchemaBuilderConfig>,
    ) -> Self {
        Self {
            url,
            namespace,
            database,
            username,
            password,
            migrations_dir,
            builder_config,
        }
    }

    fn make_client(&self) -> SurrealClient {
        SurrealClient::new(
            &self.url,
            &self.namespace,
            &self.database,
            &self.username,
            &self.password,
        )
    }
}

impl MigrationEngine for LegacyEngine {
    fn check_connection(&self) -> Result<()> {
        let client = self.make_client();
        client.ping().context("Cannot connect to SurrealDB")
    }

    fn apply(&self) -> Result<ApplyResult> {
        let client = self.make_client();
        migrate::apply(&client, &self.migrations_dir)?;
        Ok(ApplyResult {
            applied_count: 0,
            messages: vec![],
        })
    }

    fn status(&self) -> Result<Vec<MigrationInfo>> {
        let client = self.make_client();
        client.ping().context("Cannot connect to SurrealDB")?;
        client.ensure_migration_table()?;

        let applied = client.get_applied_migrations()?;
        let filesystem = migrate::scan_migrations(&self.migrations_dir)?;

        let mut infos = Vec::new();

        for fm in &filesystem {
            if let Some(am) = applied.iter().find(|a| a.version == fm.version) {
                if fm.path.exists() {
                    let disk_checksum = migrate::checksum(&fm.path)?;
                    if disk_checksum != am.checksum {
                        infos.push(MigrationInfo {
                            id: fm.version.clone(),
                            name: fm.name.clone(),
                            state: MigrationState::Drift,
                            applied_at: Some(am.applied_at.clone()),
                            detail: Some("checksum mismatch".to_string()),
                        });
                        continue;
                    }
                }
                infos.push(MigrationInfo {
                    id: fm.version.clone(),
                    name: fm.name.clone(),
                    state: MigrationState::Applied,
                    applied_at: Some(am.applied_at.clone()),
                    detail: None,
                });
            } else {
                infos.push(MigrationInfo {
                    id: fm.version.clone(),
                    name: fm.name.clone(),
                    state: MigrationState::Pending,
                    applied_at: None,
                    detail: None,
                });
            }
        }

        // Warn about applied migrations not on disk
        for am in &applied {
            if !filesystem.iter().any(|f| f.version == am.version) {
                infos.push(MigrationInfo {
                    id: am.version.clone(),
                    name: am.name.clone(),
                    state: MigrationState::Drift,
                    applied_at: Some(am.applied_at.clone()),
                    detail: Some("not present on disk".to_string()),
                });
            }
        }

        Ok(infos)
    }

    fn create(&self, name: &str) -> Result<CreateResult> {
        let conn = Some((
            self.url.as_str(),
            self.namespace.as_str(),
            self.database.as_str(),
            self.username.as_str(),
            self.password.as_str(),
        ));
        migrate::create(
            &self.migrations_dir,
            name,
            None,
            self.builder_config.as_ref(),
            conn,
        )?;
        Ok(CreateResult {
            file_path: None,
            message: format!("Migration '{}' created", name),
            has_changes: true,
        })
    }

    fn fix(&self, fix_checksums: bool) -> Result<()> {
        let client = self.make_client();
        migrate::fix(&client, &self.migrations_dir, fix_checksums)
    }
}
