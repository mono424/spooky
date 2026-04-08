use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;

/// Info about a single backend that handles jobs
#[derive(Clone, Debug)]
pub struct BackendInfo {
    pub name: String,
    pub base_url: String,
    pub auth_token: Option<String>,
    pub timeout: Option<u32>,
    pub timeout_overridable: bool,
}

impl BackendInfo {
    /// Compute the effective timeout for a job, considering the backend default
    /// and an optional per-job override (only used if timeout_overridable is true).
    pub fn effective_timeout(&self, job_override: Option<u32>) -> Duration {
        let base = self.timeout.unwrap_or(10);
        let seconds = if self.timeout_overridable {
            job_override.unwrap_or(base)
        } else {
            base
        };
        Duration::from_secs(seconds as u64)
    }
}

/// Configuration mapping job tables to their backends
#[derive(Clone, Default, Debug)]
pub struct JobConfig {
    /// Maps table_name -> backend info
    pub job_tables: HashMap<String, BackendInfo>,
}

/// A job entry in the queue (includes which backend to call)
#[derive(Clone, Debug)]
pub struct JobEntry {
    pub id: String,       // e.g., "job:abc123"
    pub base_url: String, // e.g., "http://localhost:3000"
    pub path: String,     // e.g., "/spookify"
    pub payload: Value,
    pub retries: u32,
    pub max_retries: u32,
    pub retry_strategy: String, // "linear" or "exponential"
    pub auth_token: Option<String>,
    pub timeout: Duration,
}

impl JobEntry {
    /// Create a JobEntry from record data
    pub fn from_record(
        id: String,
        base_url: String,
        auth_token: Option<String>,
        record: &Value,
        timeout: Duration,
    ) -> Self {
        Self {
            id,
            base_url,
            path: record
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            payload: record.get("payload").cloned().unwrap_or(Value::Null),
            retries: record.get("retries").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
            max_retries: record
                .get("max_retries")
                .and_then(|v| v.as_u64())
                .unwrap_or(3) as u32,
            retry_strategy: record
                .get("retry_strategy")
                .and_then(|v| v.as_str())
                .unwrap_or("linear")
                .to_string(),
            auth_token,
            timeout,
        }
    }
}
