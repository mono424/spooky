use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub mod snapshot_hash;

/// Storage mode for the SSP's per-query reference tables (`_00_query`,
/// `_00_list_ref`). Shared by the CLI (reads from `sp00ky.yml`), the SSP
/// server (runtime routing), and downstream codegen so all three agree
/// on the table-name convention.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RefMode {
    /// One shared `_00_query` / `_00_list_ref` for every user. Subject
    /// to a SurrealDB v3 LIVE-permission gap that drops cross-session
    /// INSERT notifications; kept for the eventual upstream fix.
    Single,
    /// Per-user `_00_query_user_<id>` and `_00_list_ref_user_<id>`
    /// tables, created lazily by the SSP on first registration. Each
    /// table's permission rule is hardcoded against the owning user's
    /// record id, sidestepping the LIVE-permission gap.
    Dedicated,
}

impl Default for RefMode {
    fn default() -> Self {
        RefMode::Dedicated
    }
}

impl RefMode {
    /// Stable lowercase token used in env vars and codegen output.
    pub fn as_str(&self) -> &'static str {
        match self {
            RefMode::Single => "single",
            RefMode::Dedicated => "dedicated",
        }
    }

    pub fn parse_str(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "single" => Some(RefMode::Single),
            "dedicated" => Some(RefMode::Dedicated),
            _ => None,
        }
    }
}

/// Sanitize a user record id (e.g. `"user:abc123"`) into the segment
/// that goes into a dedicated table name (e.g. `"abc123"`). Returns
/// `None` if the id is empty, missing the `user:` prefix, or contains
/// characters that aren't valid in a SurrealDB table identifier. The
/// SSP and the client mirror this function so both sides land on the
/// exact same table name.
pub fn sanitize_user_id(auth_id: &str) -> Option<String> {
    let raw = auth_id.strip_prefix("user:").unwrap_or(auth_id);
    if raw.is_empty() {
        return None;
    }
    if raw
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        Some(raw.to_string())
    } else {
        None
    }
}

/// Returns the `_00_query` table name. Always `_00_query` regardless of
/// mode: the registration row stays in the single global table so the
/// client and SSP can agree on its record id without the client needing
/// to know per-user table names at id-creation time. Only `_00_list_ref`
/// splits per user — that's where the SurrealDB LIVE permission gap
/// lives.
pub fn query_table_for(_mode: RefMode, _auth_id: &str) -> String {
    "_00_query".to_string()
}

/// Returns the `_00_list_ref` table name for a given mode + user. See
/// [`query_table_for`] for fallback semantics.
pub fn list_ref_table_for(mode: RefMode, auth_id: &str) -> String {
    match mode {
        RefMode::Single => "_00_list_ref".to_string(),
        RefMode::Dedicated => match sanitize_user_id(auth_id) {
            Some(uid) => format!("_00_list_ref_user_{}", uid),
            None => "_00_list_ref".to_string(),
        },
    }
}

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
