//! Update formatting logic (Strategy Pattern).
//!
//! This module is responsible for taking raw view results (IDs)
//! and formatting them into the desired output structure based on ViewResultFormat.

use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use smol_str::SmolStr;

/// Output format strategy
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
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

impl ViewDelta {
    /// Create an empty delta
    #[inline]
    pub fn empty() -> Self {
        Self::default()
    }
    
    /// Create a delta with only additions
    #[inline]
    pub fn additions_only(additions: Vec<SmolStr>) -> Self {
        Self { additions, removals: vec![], updates: vec![] }
    }
    
    /// Create a delta with only removals
    #[inline]
    pub fn removals_only(removals: Vec<SmolStr>) -> Self {
        Self { additions: vec![], removals, updates: vec![] }
    }
    
    /// Create a delta with only updates (content changes)
    #[inline]
    pub fn updates_only(updates: Vec<SmolStr>) -> Self {
        Self { additions: vec![], removals: vec![], updates }
    }
    
    /// Check if delta has no changes
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.additions.is_empty() && self.removals.is_empty() && self.updates.is_empty()
    }
    
    /// Total number of changes
    #[inline]
    pub fn len(&self) -> usize {
        self.additions.len() + self.removals.len() + self.updates.len()
    }
}

/// Raw view result (format-agnostic data from View)
#[derive(Debug, Clone)]
pub struct RawViewResult {
    pub query_id: String,
    pub records: Vec<SmolStr>, // full snapshot for Flat/Tree
    pub delta: Option<ViewDelta>, // delta info for Streaming
}

/// Flat list output
#[derive(Serialize, Deserialize, Debug, Clone)]
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
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct StreamingUpdate {
    pub view_id: String,
    pub records: Vec<DeltaRecord>,
}

/// Unified output type for all formats
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "format", rename_all = "lowercase")]
pub enum ViewUpdate {
    Flat(MaterializedViewUpdate),
    Tree(MaterializedViewUpdate), // Placeholder, same as Flat for now
    Streaming(StreamingUpdate),
}

impl ViewUpdate {
    /// Extract the result hash (for change detection)
    /// Returns None for Streaming format (check has_streaming_changes instead)
    #[inline]
    pub fn hash(&self) -> Option<&str> {
        match self {
            ViewUpdate::Flat(m) | ViewUpdate::Tree(m) => Some(&m.result_hash),
            ViewUpdate::Streaming(_) => None,
        }
    }
    
    /// Check if this update has any changes (for Streaming format)
    #[inline]
    pub fn has_streaming_changes(&self) -> bool {
        match self {
            ViewUpdate::Streaming(s) => !s.records.is_empty(),
            _ => true,
        }
    }
    
    /// Get the query/view ID
    #[inline]
    pub fn query_id(&self) -> &str {
        match self {
            ViewUpdate::Flat(m) | ViewUpdate::Tree(m) => &m.query_id,
            ViewUpdate::Streaming(s) => &s.view_id,
        }
    }
    
    /// Get the number of records (for Flat/Tree) or changes (for Streaming)
    #[inline]
    pub fn record_count(&self) -> usize {
        match self {
            ViewUpdate::Flat(m) | ViewUpdate::Tree(m) => m.result_data.len(),
            ViewUpdate::Streaming(s) => s.records.len(),
        }
    }
}

/// Compute hash from flat array of record IDs.
/// 
/// IMPORTANT: Sorts by record ID before hashing to ensure deterministic output
/// regardless of insertion order.
/// 
/// # Performance
/// - Uses SmallVec for datasets ≤16 records (avoids heap allocation)
/// - Returns consistent hash for empty datasets (fast path)
/// - Uses sort_unstable() for better performance
pub fn compute_flat_hash(data: &[SmolStr]) -> String {
    if data.is_empty() {
        // Consistent hash for empty data
        return String::from("e3b0c44298fc1c14");
    }
    
    // Optimization: Use stack allocation for small datasets
    if data.len() <= 16 {
        let mut sorted: SmallVec<[&SmolStr; 16]> = data.iter().collect();
        sorted.sort_unstable();
        
        let mut hasher = blake3::Hasher::new();
        for id in sorted {
            hasher.update(id.as_bytes());
            hasher.update(&[0]); // Delimiter
        }
        return hasher.finalize().to_hex().to_string();
    }
    
    // Standard path for larger datasets
    let mut sorted_data: Vec<SmolStr> = data.to_vec();
    sorted_data.sort_unstable();

    let mut hasher = blake3::Hasher::new();
    for id in &sorted_data {
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

/// Build update with pre-computed hash (optimization for when hash is already known)
///
/// For Flat/Tree formats, this skips redundant hash computation.
/// For Streaming format, behaves identically to `build_update`.
pub fn build_update_with_hash(
    raw: RawViewResult,
    format: ViewResultFormat,
    precomputed_hash: String,
) -> ViewUpdate {
    match format {
        ViewResultFormat::Flat => {
            ViewUpdate::Flat(MaterializedViewUpdate {
                query_id: raw.query_id,
                result_hash: precomputed_hash,
                result_data: raw.records,
            })
        }
        ViewResultFormat::Tree => {
            ViewUpdate::Tree(MaterializedViewUpdate {
                query_id: raw.query_id,
                result_hash: precomputed_hash,
                result_data: raw.records,
            })
        }
        ViewResultFormat::Streaming => {
            // Streaming doesn't use hash, delegate to normal build
            build_update(raw, format)
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_view_result_format_is_copy() {
        let format = ViewResultFormat::Flat;
        let copy = format; // Should work without clone
        assert_eq!(format, copy);
    }

    #[test]
    fn test_view_delta_helpers() {
        let delta = ViewDelta::additions_only(vec![SmolStr::new("a")]);
        assert_eq!(delta.additions.len(), 1);
        assert!(delta.removals.is_empty());
        assert!(delta.updates.is_empty());
        assert!(!delta.is_empty());
        assert_eq!(delta.len(), 1);
        
        let empty = ViewDelta::empty();
        assert!(empty.is_empty());
        assert_eq!(empty.len(), 0);
    }

    #[test]
    fn test_compute_flat_hash_empty() {
        let hash = compute_flat_hash(&[]);
        assert_eq!(hash, "e3b0c44298fc1c14");
    }

    #[test]
    fn test_compute_flat_hash_deterministic() {
        let data1 = vec![SmolStr::new("b"), SmolStr::new("a"), SmolStr::new("c")];
        let data2 = vec![SmolStr::new("a"), SmolStr::new("c"), SmolStr::new("b")];
        
        // Different order should produce same hash
        assert_eq!(compute_flat_hash(&data1), compute_flat_hash(&data2));
    }

    #[test]
    fn test_compute_flat_hash_small_optimization() {
        // Test that small datasets (≤16) work correctly
        let small: Vec<SmolStr> = (0..16).map(|i| SmolStr::new(format!("id{}", i))).collect();
        let hash = compute_flat_hash(&small);
        assert!(!hash.is_empty());
    }

    #[test]
    fn test_view_update_helpers() {
        let update = ViewUpdate::Flat(MaterializedViewUpdate {
            query_id: "test".to_string(),
            result_hash: "hash123".to_string(),
            result_data: vec![SmolStr::new("a"), SmolStr::new("b")],
        });
        
        assert_eq!(update.query_id(), "test");
        assert_eq!(update.hash(), Some("hash123"));
        assert_eq!(update.record_count(), 2);
    }

    #[test]
    fn test_build_update_flat() {
        let raw = RawViewResult {
            query_id: "test".to_string(),
            records: vec![SmolStr::new("a"), SmolStr::new("b")],
            delta: None,
        };
        
        let update = build_update(raw, ViewResultFormat::Flat);
        assert!(matches!(update, ViewUpdate::Flat(_)));
    }

    #[test]
    fn test_build_update_streaming() {
        let raw = RawViewResult {
            query_id: "test".to_string(),
            records: vec![SmolStr::new("a")],
            delta: Some(ViewDelta {
                additions: vec![SmolStr::new("a")],
                removals: vec![],
                updates: vec![],
            }),
        };
        
        let update = build_update(raw, ViewResultFormat::Streaming);
        if let ViewUpdate::Streaming(s) = update {
            assert_eq!(s.records.len(), 1);
            assert_eq!(s.records[0].event, DeltaEvent::Created);
        } else {
            panic!("Expected Streaming update");
        }
    }

    #[test]
    fn test_build_update_with_hash() {
        let raw = RawViewResult {
            query_id: "test".to_string(),
            records: vec![SmolStr::new("a")],
            delta: None,
        };
        
        let precomputed = "precomputed_hash".to_string();
        let update = build_update_with_hash(raw, ViewResultFormat::Flat, precomputed.clone());
        
        if let ViewUpdate::Flat(m) = update {
            assert_eq!(m.result_hash, precomputed);
        } else {
            panic!("Expected Flat update");
        }
    }
}
