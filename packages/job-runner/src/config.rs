use crate::types::{BackendInfo, JobConfig};
use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

#[derive(Debug, Deserialize)]
struct SpookyConfig {
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

/// Load job configuration from spooky.yml with inline backend configs
pub fn load_config<P: AsRef<Path>>(spooky_config_path: P) -> Result<JobConfig> {
    let config_path = spooky_config_path.as_ref();

    // Read spooky.yml
    let config_str = fs::read_to_string(config_path)
        .context(format!("Failed to read spooky config: {:?}", config_path))?;

    let spooky_config: SpookyConfig =
        serde_yaml::from_str(&config_str).context("Failed to parse spooky config")?;

    let mut job_tables = HashMap::new();

    // Process each backend directly from the map
    for (backend_name, backend_config) in spooky_config.backends {
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
