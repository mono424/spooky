use anyhow::{bail, Context, Result};
use base64::Engine;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct SurrealResponse {
    pub status: String,
    pub result: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AppliedMigration {
    pub version: String,
    pub name: String,
    pub applied_at: String,
    pub checksum: String,
}

/// Trait abstracting the SurrealDB operations needed by the migration system.
/// This enables testing with a mock implementation.
pub trait MigrationDB {
    fn ping(&self) -> Result<()>;
    fn ensure_migration_table(&self) -> Result<()>;
    fn execute(&self, query: &str) -> Result<Vec<SurrealResponse>>;
    fn get_applied_migrations(&self) -> Result<Vec<AppliedMigration>>;
    fn record_migration(&self, version: &str, name: &str, checksum: &str) -> Result<()>;
    fn remove_migration(&self, version: &str) -> Result<()>;
}

pub struct SurrealClient {
    url: String,
    namespace: String,
    database: String,
    auth_header: String,
}

impl SurrealClient {
    pub fn new(
        url: &str,
        namespace: &str,
        database: &str,
        username: &str,
        password: &str,
    ) -> Self {
        let credentials = format!("{}:{}", username, password);
        let auth_header = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(credentials)
        );
        Self {
            url: url.trim_end_matches('/').to_string(),
            namespace: namespace.to_string(),
            database: database.to_string(),
            auth_header,
        }
    }
}

impl MigrationDB for SurrealClient {
    fn execute(&self, query: &str) -> Result<Vec<SurrealResponse>> {
        let resp = ureq::post(&format!("{}/sql", self.url))
            .set("Accept", "application/json")
            .set("surreal-ns", &self.namespace)
            .set("surreal-db", &self.database)
            .set("Authorization", &self.auth_header)
            .send_string(query)
            .context("Failed to connect to SurrealDB")?;

        let body: Vec<SurrealResponse> = resp
            .into_json()
            .context("Failed to parse SurrealDB response")?;

        for r in &body {
            if r.status == "ERR" {
                let msg = r
                    .result
                    .as_ref()
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown error");
                bail!("SurrealDB error: {}", msg);
            }
        }

        Ok(body)
    }

    fn ping(&self) -> Result<()> {
        self.execute("INFO FOR DB;")
            .context("Failed to ping SurrealDB")?;
        Ok(())
    }

    fn ensure_migration_table(&self) -> Result<()> {
        let schema = include_str!("migration_tables.surql");
        self.execute(schema)
            .context("Failed to create migration tracking table")?;
        Ok(())
    }

    fn get_applied_migrations(&self) -> Result<Vec<AppliedMigration>> {
        let responses = self
            .execute("SELECT version, name, applied_at, checksum FROM _spooky_migrations ORDER BY version ASC;")
            .context("Failed to query applied migrations")?;

        let result = responses
            .into_iter()
            .next()
            .and_then(|r| r.result)
            .unwrap_or(serde_json::Value::Array(vec![]));

        let migrations: Vec<AppliedMigration> =
            serde_json::from_value(result).context("Failed to deserialize applied migrations")?;

        Ok(migrations)
    }

    fn record_migration(&self, version: &str, name: &str, checksum: &str) -> Result<()> {
        let query = format!(
            "CREATE _spooky_migrations SET version = '{}', name = '{}', checksum = '{}';",
            version, name, checksum
        );
        self.execute(&query)
            .context("Failed to record migration")?;
        Ok(())
    }

    fn remove_migration(&self, version: &str) -> Result<()> {
        let query = format!(
            "DELETE _spooky_migrations WHERE version = '{}';",
            version
        );
        self.execute(&query)
            .context("Failed to remove migration record")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_constructs_basic_auth_header() {
        let client = SurrealClient::new(
            "http://localhost:8000",
            "test_ns",
            "test_db",
            "root",
            "root",
        );
        // base64("root:root") = "cm9vdDpyb290"
        assert_eq!(client.auth_header, "Basic cm9vdDpyb290");
    }

    #[test]
    fn test_new_encodes_special_chars_in_credentials() {
        let client = SurrealClient::new(
            "http://localhost:8000",
            "ns",
            "db",
            "admin",
            "p@ss:w0rd!",
        );
        let expected = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode("admin:p@ss:w0rd!")
        );
        assert_eq!(client.auth_header, expected);
    }

    #[test]
    fn test_new_trims_trailing_slash_from_url() {
        let client = SurrealClient::new(
            "http://localhost:8000/",
            "ns",
            "db",
            "root",
            "root",
        );
        assert_eq!(client.url, "http://localhost:8000");
    }

    #[test]
    fn test_new_trims_multiple_trailing_slashes() {
        let client = SurrealClient::new(
            "http://localhost:8000///",
            "ns",
            "db",
            "root",
            "root",
        );
        assert_eq!(client.url, "http://localhost:8000");
    }

    #[test]
    fn test_new_preserves_url_without_trailing_slash() {
        let client = SurrealClient::new(
            "http://localhost:8000",
            "ns",
            "db",
            "root",
            "root",
        );
        assert_eq!(client.url, "http://localhost:8000");
    }

    #[test]
    fn test_new_stores_namespace_and_database() {
        let client = SurrealClient::new(
            "http://localhost:8000",
            "my_namespace",
            "my_database",
            "root",
            "root",
        );
        assert_eq!(client.namespace, "my_namespace");
        assert_eq!(client.database, "my_database");
    }

    #[test]
    fn test_surreal_response_deserialize_ok() {
        let json = r#"{"status":"OK","result":[{"id":"test:1"}]}"#;
        let resp: SurrealResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.status, "OK");
        assert!(resp.result.is_some());
    }

    #[test]
    fn test_surreal_response_deserialize_err() {
        let json = r#"{"status":"ERR","result":"Some error message"}"#;
        let resp: SurrealResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.status, "ERR");
        assert_eq!(resp.result.unwrap().as_str().unwrap(), "Some error message");
    }

    #[test]
    fn test_surreal_response_deserialize_null_result() {
        let json = r#"{"status":"OK","result":null}"#;
        let resp: SurrealResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.status, "OK");
        assert!(resp.result.is_none());
    }

    #[test]
    fn test_applied_migration_deserialize() {
        let json = r#"{
            "version": "20240101120000",
            "name": "initial_schema",
            "applied_at": "2024-01-01T12:05:00Z",
            "checksum": "abc123"
        }"#;
        let m: AppliedMigration = serde_json::from_str(json).unwrap();
        assert_eq!(m.version, "20240101120000");
        assert_eq!(m.name, "initial_schema");
        assert_eq!(m.applied_at, "2024-01-01T12:05:00Z");
        assert_eq!(m.checksum, "abc123");
    }

    #[test]
    fn test_applied_migration_deserialize_from_array() {
        let json = r#"[
            {"version":"20240101120000","name":"first","applied_at":"2024-01-01T12:00:00Z","checksum":"aaa"},
            {"version":"20240102120000","name":"second","applied_at":"2024-01-02T12:00:00Z","checksum":"bbb"}
        ]"#;
        let migrations: Vec<AppliedMigration> = serde_json::from_str(json).unwrap();
        assert_eq!(migrations.len(), 2);
        assert_eq!(migrations[0].version, "20240101120000");
        assert_eq!(migrations[1].version, "20240102120000");
    }
}
