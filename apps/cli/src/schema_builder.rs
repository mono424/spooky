use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;

use crate::backend::BackendProcessor;

pub struct SchemaBuilderConfig {
    pub input_path: PathBuf,
    pub config_path: Option<PathBuf>,
    pub mode: String,
    pub endpoint: Option<String>,
    pub secret: Option<String>,
    pub include_functions: bool,
}

/// Build ONLY the remote functions SQL (heartbeat + mode-specific functions
/// with endpoint/secret substitution).  Used by `dev.rs` to apply functions
/// separately with Docker-internal URLs.
pub fn build_remote_functions_schema(
    mode: &str,
    endpoint: &str,
    secret: &str,
) -> String {
    let mut content = String::new();

    // Set database-level params so events can reference them without hardcoding
    content.push_str(&format!(
        "DEFINE PARAM OVERWRITE $sp00ky_endpoint VALUE '{}';\n",
        endpoint
    ));
    content.push_str(&format!(
        "DEFINE PARAM OVERWRITE $sp00ky_secret VALUE '{}';\n\n",
        secret
    ));

    // Common remote functions (heartbeat)
    content.push_str(include_str!("functions_remote.surql"));

    // Mode-specific functions
    let functions_remote_singlenode = include_str!("functions_remote_singlenode.surql");
    let functions_remote_surrealism = include_str!("functions_remote_surrealism.surql");

    if mode == "singlenode" || mode == "cluster" {
        let mut singlenode_fn = functions_remote_singlenode.to_string();
        singlenode_fn = singlenode_fn.replace("{{ENDPOINT}}", endpoint);
        singlenode_fn = singlenode_fn.replace("{{SECRET}}", secret);

        content.push('\n');
        content.push_str(&singlenode_fn);
    } else if mode == "surrealism" {
        content.push('\n');
        content.push_str(functions_remote_surrealism);
    }

    content
}

/// Assembles the complete server schema from all sources.
///
/// This builds the full schema that should be present in SurrealDB:
/// user schema + backend schemas + meta tables + remote functions + buckets.
pub fn build_server_schema(config: &SchemaBuilderConfig) -> Result<String> {
    let mut content = fs::read_to_string(&config.input_path).context(format!(
        "Failed to read input schema file: {:?}",
        config.input_path
    ))?;

    // Process sp00ky config/backends
    let mut backend_processor = BackendProcessor::new();
    if let Some(config_path) = &config.config_path {
        if config_path.exists() {
            backend_processor.process(config_path)?;
            content.push('\n');
            content.push_str(&backend_processor.schema_appends);
        }
    }

    // Base meta tables
    content.push('\n');
    content.push_str(include_str!("meta_tables.surql"));

    // Remote meta tables (server-side)
    content.push('\n');
    content.push_str(include_str!("meta_tables_remote.surql"));

    // Migration tracking table
    content.push('\n');
    content.push_str(include_str!("migration_tables.surql"));

    // Bucket definitions
    if !backend_processor.bucket_schema.is_empty() {
        content.push('\n');
        content.push_str(&backend_processor.bucket_schema);
    }

    // Remote functions — only when include_functions is true
    if config.include_functions {
        let default_endpoint = if config.mode == "cluster" {
            "http://localhost:9667"
        } else {
            "http://localhost:8667"
        };
        let endpoint = config
            .endpoint
            .as_deref()
            .unwrap_or(default_endpoint);
        let secret = config.secret.as_deref().unwrap_or("");

        let functions_sql = build_remote_functions_schema(&config.mode, endpoint, secret);
        content.push('\n');
        content.push_str(&functions_sql);
    }

    // Replace unregister_view call (this transforms event handlers in user
    // schema, not function definitions — always apply it)
    if config.mode == "singlenode" || config.mode == "cluster" {
        let unregister_call = "let $result = mod::dbsp::unregister_view(<string>$before.id);";
        let unregister_http =
            "let $payload = { id: <string>$before.id };\n    let $result = http::post($sp00ky_endpoint + '/view/unregister', $payload, { \"Authorization\": \"Bearer \" + $sp00ky_secret });";
        content = content.replace(unregister_call, unregister_http);
    }

    Ok(content)
}
