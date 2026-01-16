//! Update formatting logic (Strategy Pattern).
//!
//! This module is responsible for taking raw view results (IDs + versions)
//! and formatting them into the desired output structure based on ViewResultFormat.

use serde::{Deserialize, Serialize};

/// Output format strategy
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ViewResultFormat {
    /// Flat list: [(id, version), ...] with hash
    #[default]
    Flat,
    /// Tree structure (future)
    Tree,
    /// Streaming deltas with events
    Streaming,
}

/// Raw view result (format-agnostic data from View)
#[derive(Debug, Clone)]
pub struct RawViewResult {
    pub query_id: String,
    pub records: Vec<(String, u64)>, // (id, version)
}

/// Flat list output (current version-approach format)
#[derive(Serialize, Deserialize, Debug)]
pub struct MaterializedViewUpdate {
    pub query_id: String,
    pub result_hash: String,
    pub result_data: Vec<(String, u64)>, // [(record-id, version), ...]
}

/// Delta event for streaming format
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DeltaEvent {
    Created,
    Updated,
    Deleted,
}

/// Delta record for streaming format
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DeltaRecord {
    pub id: String,
    pub event: DeltaEvent,
    pub version: u64,
}

/// Streaming update output (minimal payload)
#[derive(Serialize, Deserialize, Debug)]
pub struct StreamingUpdate {
    pub view_id: String,
    pub records: Vec<DeltaRecord>,
}

/// Unified output type for all formats
#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "format", rename_all = "lowercase")]
pub enum ViewUpdate {
    Flat(MaterializedViewUpdate),
    Tree(MaterializedViewUpdate), // Placeholder, same as Flat for now
    Streaming(StreamingUpdate),
}

/// Compute hash from flat array of [record-id, version] pairs
pub fn compute_flat_hash(data: &[(String, u64)]) -> String {
    let mut hasher = blake3::Hasher::new();
    for (id, version) in data {
        hasher.update(id.as_bytes());
        hasher.update(&version.to_le_bytes());
        hasher.update(&[0]); // Delimiter
    }
    hasher.finalize().to_hex().to_string()
}

/// Build the final update based on the desired format
pub fn build_update(raw: RawViewResult, format: ViewResultFormat) -> ViewUpdate {
    match format {
        ViewResultFormat::Flat => {
            let hash = compute_flat_hash(&raw.records);
            ViewUpdate::Flat(MaterializedViewUpdate {
                query_id: raw.query_id,
                result_hash: hash,
                result_data: raw.records,
            })
        }
        ViewResultFormat::Tree => {
            // Future: Build tree structure
            // For now, return flat format
            let hash = compute_flat_hash(&raw.records);
            ViewUpdate::Tree(MaterializedViewUpdate {
                query_id: raw.query_id,
                result_hash: hash,
                result_data: raw.records,
            })
        }
        ViewResultFormat::Streaming => {
            // Streaming format: all records are "Created" on registration
            let records = raw
                .records
                .into_iter()
                .map(|(id, version)| DeltaRecord {
                    id,
                    event: DeltaEvent::Created,
                    version,
                })
                .collect();

            ViewUpdate::Streaming(StreamingUpdate {
                view_id: raw.query_id,
                records,
            })
        }
    }
}

/// Build streaming delta update from ZSet delta
pub fn build_streaming_delta(
    query_id: String,
    delta: &[(String, i64, u64)], // (id, weight, version)
) -> StreamingUpdate {
    let records = delta
        .iter()
        .map(|(id, weight, version)| DeltaRecord {
            id: id.clone(),
            event: if *weight > 0 {
                DeltaEvent::Created
            } else {
                DeltaEvent::Deleted
            },
            version: *version,
        })
        .collect();

    StreamingUpdate {
        view_id: query_id,
        records,
    }
}
