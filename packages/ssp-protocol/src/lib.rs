use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub mod snapshot_hash;

// --- Ingest API (snake_case wire format) ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestRequest {
    pub table: String,
    pub op: String,
    pub id: String,
    pub record: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_assignee: Option<String>,
}

// --- View API (camelCase wire format via serde rename) ---

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewRegisterRequest {
    pub id: String,
    pub surql: String,
    pub client_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttl: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_active_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewUnregisterRequest {
    pub id: String,
}

// --- SSP Management API (snake_case wire format) ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SspRegistration {
    pub ssp_id: String,
    pub url: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<std::collections::HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SspRegistrationResponse {
    pub snapshot_seq: u64,
    /// Per-table content hashes (blake3, hex with `b3:` prefix) at
    /// `snapshot_seq`. The SSP must produce the same hashes after loading
    /// its circuit store; mismatch ⇒ retry-then-fatal so the supervisor
    /// re-registers from a fresh frozen snapshot.
    #[serde(default)]
    pub table_hashes: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SspHeartbeat {
    pub ssp_id: String,
    pub timestamp: u64,
    pub views: usize,
    pub cpu_usage: Option<f64>,
    pub memory_usage: Option<f64>,
    pub version: String,
}
