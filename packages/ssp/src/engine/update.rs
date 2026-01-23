//! Update formatting logic (Strategy Pattern).
//!
//! This module is responsible for taking raw view results (IDs)
//! and formatting them into the desired output structure based on ViewResultFormat.

use serde::{Deserialize, Serialize};

use smol_str::SmolStr;

/// Output format strategy
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ViewResultFormat {
    /// Flat list: [id, ...] with hash
    #[default]
    Flat,
    /// Tree structure (future)
    Tree,
    /// Streaming deltas with events
    Streaming,
}

/// Captured delta information from view processing
#[derive(Debug, Clone, Default)]
pub struct ViewDelta {
    /// Records added to the view (weight > 0)
    pub additions: Vec<SmolStr>,
    /// Records removed from the view (weight < 0)
    pub removals: Vec<SmolStr>,
    /// Records updated in place (content changed, still in view)
    pub updates: Vec<SmolStr>,
}

/// Raw view result (format-agnostic data from View)
#[derive(Debug, Clone)]
pub struct RawViewResult {
    pub query_id: String,
    pub records: Vec<SmolStr>, // full snapshot for Flat/Tree
    pub delta: Option<ViewDelta>, // delta info for Streaming
}

/// Flat list output
#[derive(Serialize, Deserialize, Debug)]
pub struct MaterializedViewUpdate {
    pub query_id: String,
    pub result_hash: String,
    pub result_data: Vec<SmolStr>, // [record-id, ...]
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
    pub id: SmolStr,
    pub event: DeltaEvent,
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

/// Compute hash from flat array of record IDs.
/// IMPORTANT: Sorts by record ID before hashing to ensure deterministic output
/// regardless of insertion order.
pub fn compute_flat_hash(data: &[SmolStr]) -> String {
    let mut sorted_data: Vec<_> = data.to_vec();
    sorted_data.sort();

    let mut hasher = blake3::Hasher::new();
    for id in sorted_data {
        hasher.update(id.as_bytes());
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
            let hash = compute_flat_hash(&raw.records);
            ViewUpdate::Tree(MaterializedViewUpdate {
                query_id: raw.query_id,
                result_hash: hash,
                result_data: raw.records,
            })
        }
        ViewResultFormat::Streaming => {
            let mut delta_records = Vec::new();

            if let Some(delta) = raw.delta {
                for id in delta.additions {
                    delta_records.push(DeltaRecord {
                        id,
                        event: DeltaEvent::Created,
                    });
                }

                for id in delta.removals {
                    delta_records.push(DeltaRecord {
                        id,
                        event: DeltaEvent::Deleted,
                    });
                }

                for id in delta.updates {
                    delta_records.push(DeltaRecord {
                        id,
                        event: DeltaEvent::Updated,
                    });
                }
            } else {
                for id in raw.records {
                    delta_records.push(DeltaRecord {
                        id,
                        event: DeltaEvent::Created,
                    });
                }
            }

            ViewUpdate::Streaming(StreamingUpdate {
                view_id: raw.query_id,
                records: delta_records,
            })
        }
    }
}

/// Build streaming delta update from ZSet delta
pub fn build_streaming_delta(
    query_id: String,
    delta: &[(SmolStr, i64)], // (id, weight)
) -> StreamingUpdate {
    let records = delta
        .iter()
        .map(|(id, weight)| DeltaRecord {
            id: id.clone(),
            event: if *weight > 0 {
                DeltaEvent::Created
            } else {
                DeltaEvent::Deleted
            },
        })
        .collect();

    StreamingUpdate {
        view_id: query_id,
        records,
    }
}
