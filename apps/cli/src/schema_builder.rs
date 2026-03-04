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

    // Process spooky config/backends
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

    // Common remote functions
    let functions_remote = include_str!("functions_remote.surql");
    content.push('\n');
    content.push_str(functions_remote);

    // Mode-specific functions
    let functions_remote_singlenode = include_str!("functions_remote_singlenode.surql");
    let functions_remote_surrealism = include_str!("functions_remote_surrealism.surql");

    if config.mode == "singlenode" || config.mode == "cluster" {
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

        let mut singlenode_fn = functions_remote_singlenode.to_string();
        singlenode_fn = singlenode_fn.replace("{{ENDPOINT}}", endpoint);
        singlenode_fn = singlenode_fn.replace("{{SECRET}}", secret);

        content.push('\n');
        content.push_str(&singlenode_fn);

        // Replace unregister_view call
        let unregister_call = "let $result = mod::dbsp::unregister_view(<string>$before.id);";
        let unregister_http = format!(
            "let $payload = {{ id: <string>$before.id }};\n    let $result = http::post('{}/view/unregister', $payload, {{ \"Authorization\": \"Bearer {}\" }});",
            endpoint, secret
        );
        content = content.replace(unregister_call, &unregister_http);
    } else if config.mode == "surrealism" {
        content.push('\n');
        content.push_str(functions_remote_surrealism);
    }

    Ok(content)
}

