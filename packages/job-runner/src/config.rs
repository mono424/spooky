use crate::types::{BackendInfo, JobConfig};
use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

#[derive(Debug, Deserialize)]
struct Sp00kyConfig {
    backends: HashMap<String, BackendConfig>,
}

#[derive(Debug, Deserialize)]
struct BackendConfig {
    #[serde(rename = "baseUrl")]
    base_url: Option<String>,
    method: BackendMethod,
    auth: Option<AuthConfig>,
}

#[derive(Debug, Deserialize)]
struct AuthConfig {
    #[serde(rename = "type")]
    auth_type: String,
    token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BackendMethod {
    #[serde(rename = "type")]
    method_type: String,
    table: Option<String>,
}

/// Load job configuration from sp00ky.yml with inline backend configs
pub fn load_config<P: AsRef<Path>>(sp00ky_config_path: P) -> Result<JobConfig> {
    let config_path = sp00ky_config_path.as_ref();

    // Read sp00ky.yml
    let config_str = fs::read_to_string(config_path)
        .context(format!("Failed to read sp00ky config: {:?}", config_path))?;

    let sp00ky_config: Sp00kyConfig =
        serde_yaml::from_str(&config_str).context("Failed to parse sp00ky config")?;

    let mut job_tables = HashMap::new();

    // Process each backend directly from the map
    for (backend_name, backend_config) in sp00ky_config.backends {
        // Only process outbox backends that have a base_url and table
        if backend_config.method.method_type == "outbox" {
            if let (Some(base_url), Some(table)) =
                (backend_config.base_url, backend_config.method.table)
            {
                // Extract auth token if present
                let auth_token = backend_config.auth.and_then(|auth| {
                    if auth.auth_type == "token" {
                        auth.token
                    } else {
                        None
                    }
                });

                let backend_info = BackendInfo {
                    name: backend_name,
                    base_url,
                    auth_token,
                };

                // Map table to backend info
                if job_tables.insert(table.clone(), backend_info).is_some() {
                    tracing::warn!(
                        table = %table,
                        "Table already mapped to another backend - overwriting"
                    );
                }
            }
        }
    }

    Ok(JobConfig { job_tables })
}

/// Build a JobConfig from a SurrealDB _sp00ky_config record.
/// Expected format: { "backends": [{ "name": "...", "base_url": "...", "method_type": "outbox", "table": "..." }] }
pub fn from_db_record(record: &Value) -> Result<JobConfig> {
    let backends = record
        .get("backends")
        .and_then(|v| v.as_array())
        .context("_sp00ky_config record missing 'backends' array")?;

    let mut job_tables = HashMap::new();
    for entry in backends {
        let method_type = entry.get("method_type").and_then(|v| v.as_str()).unwrap_or("");
        if method_type != "outbox" {
            continue;
        }
        let name = entry.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let base_url = entry.get("base_url").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let table = entry.get("table").and_then(|v| v.as_str()).unwrap_or("").to_string();
        if table.is_empty() || base_url.is_empty() {
            continue;
        }
        let auth_token = entry
            .get("auth_token")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        job_tables.insert(
            table,
            BackendInfo {
                name,
                base_url,
                auth_token,
            },
        );
    }
    Ok(JobConfig { job_tables })
}
