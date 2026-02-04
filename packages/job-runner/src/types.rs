use serde_json::Value;
use std::collections::HashMap;

/// Info about a single backend that handles jobs
#[derive(Clone, Debug)]
pub struct BackendInfo {
    pub name: String,
    pub base_url: String,
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
    pub id: String,           // e.g., "job:abc123"
    pub base_url: String,     // e.g., "http://localhost:3000"
    pub path: String,         // e.g., "/spookify"
    pub payload: Value,
    pub retries: u32,
    pub max_retries: u32,
    pub retry_strategy: String, // "linear" or "exponential"
}

impl JobEntry {
    /// Create a JobEntry from record data
    pub fn from_record(id: String, base_url: String, record: &Value) -> Self {
        Self {
            id,
            base_url,
            path: record
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            payload: record.get("payload").cloned().unwrap_or(Value::Null),
            retries: record
                .get("retries")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32,
            max_retries: record
                .get("max_retries")
                .and_then(|v| v.as_u64())
                .unwrap_or(3) as u32,
            retry_strategy: record
                .get("retry_strategy")
                .and_then(|v| v.as_str())
                .unwrap_or("linear")
                .to_string(),
        }
    }
}
