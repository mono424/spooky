use crate::types::{BackendInfo, JobConfig};
use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

#[derive(Debug, Deserialize)]
struct Sp00kyConfig {
    #[serde(default)]
    apps: HashMap<String, AppConfig>,
}

#[derive(Debug, Deserialize)]
struct AppConfig {
    #[serde(rename = "type")]
    app_type: Option<String>,
    #[serde(rename = "baseUrl")]
    base_url: Option<String>,
    method: Option<AppMethod>,
    auth: Option<AuthConfig>,
    deploy: Option<DeployConfig>,
}

#[derive(Debug, Deserialize)]
struct AuthConfig {
    #[serde(rename = "type")]
    auth_type: String,
    token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AppMethod {
    #[serde(rename = "type")]
    method_type: String,
    table: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DeployConfig {
    timeout: Option<u32>,
    #[serde(rename = "timeoutOverridable")]
    timeout_overridable: Option<bool>,
}

/// Load job configuration from sp00ky.yml with inline app configs
pub fn load_config<P: AsRef<Path>>(sp00ky_config_path: P) -> Result<JobConfig> {
    let config_path = sp00ky_config_path.as_ref();

    let config_str = fs::read_to_string(config_path)
        .context(format!("Failed to read sp00ky config: {:?}", config_path))?;

    let sp00ky_config: Sp00kyConfig =
        serde_yaml::from_str(&config_str).context("Failed to parse sp00ky config")?;

    let mut job_tables = HashMap::new();

    for (app_name, app_config) in sp00ky_config.apps {
        // Only process backend apps with outbox method
        if app_config.app_type.as_deref() != Some("backend") {
            continue;
        }
        let method = match &app_config.method {
            Some(m) if m.method_type == "outbox" => m,
            _ => continue,
        };
        if let (Some(base_url), Some(table)) = (&app_config.base_url, &method.table) {
            let auth_token = app_config.auth.and_then(|auth| {
                if auth.auth_type == "token" {
                    auth.token
                } else {
                    None
                }
            });

            let timeout = app_config.deploy.as_ref().and_then(|d| d.timeout);
            let timeout_overridable = app_config.deploy.as_ref()
                .and_then(|d| d.timeout_overridable)
                .unwrap_or(false);

            let backend_info = BackendInfo {
                name: app_name,
                base_url: base_url.clone(),
                auth_token,
                timeout,
                timeout_overridable,
            };

            if job_tables.insert(table.clone(), backend_info).is_some() {
                tracing::warn!(
                    table = %table,
                    "Table already mapped to another backend - overwriting"
                );
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
        let timeout = entry.get("timeout").and_then(|v| v.as_u64()).map(|v| v as u32);
        let timeout_overridable = entry.get("timeout_overridable").and_then(|v| v.as_bool()).unwrap_or(false);
        job_tables.insert(
            table,
            BackendInfo {
                name,
                base_url,
                auth_token,
                timeout,
                timeout_overridable,
            },
        );
    }
    Ok(JobConfig { job_tables })
}
