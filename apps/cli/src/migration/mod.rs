pub mod engine;
mod legacy;
pub(crate) mod sp00ky_engine;
pub(crate) mod surrealkit;

use anyhow::Result;
use std::path::PathBuf;

pub use engine::{MigrationEngine, MigrationEnvironment, MigrationInfo, MigrationState};
pub use sp00ky_engine::{InternalSchemaConfig, RemoteFunctionsConfig};

/// All parameters needed to construct a migration engine.
pub struct MigrationContext {
    pub environment: MigrationEnvironment,
    pub project_dir: PathBuf,
    pub migrations_dir: PathBuf,
    // Connection params
    pub url: String,
    pub namespace: String,
    pub database: String,
    pub username: String,
    pub password: String,
    // surrealkit-specific (None = use legacy engine)
    pub surrealkit_binary: Option<String>,
    // Post-migration steps (optional, set by caller based on context)
    pub internal_schema: Option<InternalSchemaConfig>,
    pub remote_functions: Option<RemoteFunctionsConfig>,
}

/// Factory: select the inner engine based on config, then wrap with Sp00kyEngine decorator.
///
/// The decorator handles internal schema + remote functions after `apply()`.
/// When both `internal_schema` and `remote_functions` are `None`, the decorator
/// delegates straight through with no overhead.
pub fn create_engine(ctx: MigrationContext) -> Result<Box<dyn MigrationEngine>> {
    let inner: Box<dyn MigrationEngine> = if let Some(ref binary) = ctx.surrealkit_binary {
        Box::new(surrealkit::SurrealKitEngine::new(
            binary.clone(),
            ctx.environment,
            ctx.project_dir.clone(),
            ctx.url.clone(),
            ctx.namespace.clone(),
            ctx.database.clone(),
            ctx.username.clone(),
            ctx.password.clone(),
        )?)
    } else {
        Box::new(legacy::LegacyEngine::new(
            ctx.url.clone(),
            ctx.namespace.clone(),
            ctx.database.clone(),
            ctx.username.clone(),
            ctx.password.clone(),
            ctx.migrations_dir.clone(),
        ))
    };

    Ok(Box::new(sp00ky_engine::Sp00kyEngine::new(
        inner,
        ctx.url,
        ctx.namespace,
        ctx.database,
        ctx.username,
        ctx.password,
        ctx.internal_schema,
        ctx.remote_functions,
    )))
}
