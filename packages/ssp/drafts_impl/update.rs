//! Output formatting module (Strategy Pattern).
//!
//! This module is responsible for taking raw view results and formatting them
//! into the desired output structure. The view.rs module produces format-agnostic
//! `RawViewResult` data, and this module transforms it based on `ViewResultFormat`.
//!
//! Architecture:
//! ```text
//! view.rs (DBSP core) --> RawViewResult --> update.rs (formatting) --> ViewUpdate
//! ```

use serde::{Deserialize, Serialize};

/// Output format strategy - determines how view results are formatted
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ViewResultFormat {
    /// Flat list: [(id, version), ...] with hash
    /// Best for: Simple lists, pagination, full state sync
    #[default]
    Flat,
    /// Tree structure with nested records
    /// Best for: Hierarchical data, document-style views
    /// Future: Will include nested structure
    Tree,
    /// Streaming deltas with events (Created/Updated/Deleted)
    /// Best for: Real-time updates, minimal payload
    Streaming,
}

// ============================================================================
// Raw View Result (format-agnostic data from view.rs)
// ============================================================================

/// Delta information captured during view processing
#[derive(Debug, Clone, Default)]
pub struct ViewDelta {
    /// Records added to the view (id, version)
    pub additions: Vec<(String, u64)>,
    /// Records removed from the view (just ids, version 0)
    pub removals: Vec<String>,
    /// Records updated in place (id, new_version)
    pub updates: Vec<(String, u64)>,
}

impl ViewDelta {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(cap: usize) -> Self {
        Self {
            additions: Vec::with_capacity(cap),
            removals: Vec::with_capacity(cap),
            updates: Vec::with_capacity(cap),
        }
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.additions.is_empty() && self.removals.is_empty() && self.updates.is_empty()
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.additions.len() + self.removals.len() + self.updates.len()
    }

    pub fn add_addition(&mut self, id: String, version: u64) {
        self.additions.push((id, version));
    }

    pub fn add_removal(&mut self, id: String) {
        self.removals.push(id);
    }

    pub fn add_update(&mut self, id: String, version: u64) {
        self.updates.push((id, version));
    }
}

/// Raw view result - format-agnostic data produced by view.rs
/// This is the bridge between view computation and output formatting.
#[derive(Debug, Clone)]
pub struct RawViewResult {
    /// Query/view identifier
    pub query_id: String,
    /// Full record list (id, version) - for Flat/Tree modes
    pub records: Vec<(String, u64)>,
    /// Delta information - for Streaming mode (and change detection)
    pub delta: ViewDelta,
    /// Whether this is the first run (initial snapshot)
    pub is_first_run: bool,
}

impl RawViewResult {
    pub fn new(query_id: String) -> Self {
        Self {
            query_id,
            records: Vec::new(),
            delta: ViewDelta::new(),
            is_first_run: false,
        }
    }

    pub fn with_capacity(query_id: String, capacity: usize) -> Self {
        Self {
            query_id,
            records: Vec::with_capacity(capacity),
            delta: ViewDelta::with_capacity(capacity),
            is_first_run: false,
        }
    }
}

// ============================================================================
// Output Types (formatted results)
// ============================================================================

/// Flat/Tree list output - full snapshot with hash
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MaterializedViewUpdate {
    pub query_id: String,
    pub result_hash: String,
    pub result_data: Vec<(String, u64)>, // [(record-id, version), ...]
}

/// Delta event type for streaming format
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DeltaEvent {
    Created,
    Updated,
    Deleted,
}

/// Individual delta record for streaming format
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DeltaRecord {
    pub id: String,
    pub event: DeltaEvent,
    pub version: u64,
}

impl DeltaRecord {
    #[inline]
    pub fn created(id: String, version: u64) -> Self {
        Self { id, event: DeltaEvent::Created, version }
    }

    #[inline]
    pub fn updated(id: String, version: u64) -> Self {
        Self { id, event: DeltaEvent::Updated, version }
    }

    #[inline]
    pub fn deleted(id: String) -> Self {
        Self { id, event: DeltaEvent::Deleted, version: 0 }
    }
}

/// Streaming update output - minimal delta payload
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct StreamingUpdate {
    pub view_id: String,
    pub records: Vec<DeltaRecord>,
}

impl StreamingUpdate {
    pub fn new(view_id: String) -> Self {
        Self {
            view_id,
            records: Vec::new(),
        }
    }

    pub fn with_capacity(view_id: String, capacity: usize) -> Self {
        Self {
            view_id,
            records: Vec::with_capacity(capacity),
        }
    }
}

/// Unified output type for all formats
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "format", rename_all = "lowercase")]
pub enum ViewUpdate {
    Flat(MaterializedViewUpdate),
    Tree(MaterializedViewUpdate), // Currently same as Flat, will diverge for tree structure
    Streaming(StreamingUpdate),
}

impl ViewUpdate {
    /// Get the view/query ID from any variant
    pub fn query_id(&self) -> &str {
        match self {
            ViewUpdate::Flat(m) | ViewUpdate::Tree(m) => &m.query_id,
            ViewUpdate::Streaming(s) => &s.view_id,
        }
    }
}

// ============================================================================
// Hash Computation
// ============================================================================

/// Compute deterministic hash from flat array of (record-id, version) pairs.
/// Sorts by record ID before hashing to ensure consistent output regardless of order.
pub fn compute_flat_hash(data: &[(String, u64)]) -> String {
    // Sort by record ID for deterministic hash
    let mut sorted_data: Vec<_> = data.to_vec();
    sorted_data.sort_unstable_by(|a, b| a.0.cmp(&b.0));

    let mut hasher = blake3::Hasher::new();
    for (id, version) in sorted_data {
        hasher.update(id.as_bytes());
        hasher.update(&version.to_le_bytes());
        hasher.update(&[0]); // Delimiter
    }
    hasher.finalize().to_hex().to_string()
}

/// Compute hash from just record IDs (for hash-based versioning)
pub fn compute_ids_hash(ids: &[String]) -> String {
    let mut sorted_ids = ids.to_vec();
    sorted_ids.sort_unstable();

    let mut hasher = blake3::Hasher::new();
    for id in sorted_ids {
        hasher.update(id.as_bytes());
        hasher.update(&[0]);
    }
    hasher.finalize().to_hex().to_string()
}

// ============================================================================
// Format Builders (Strategy Pattern Implementation)
// ============================================================================

/// Build the final ViewUpdate based on the desired format.
/// This is the main entry point for output formatting.
pub fn build_update(raw: RawViewResult, format: ViewResultFormat) -> ViewUpdate {
    match format {
        ViewResultFormat::Flat => build_flat_update(raw),
        ViewResultFormat::Tree => build_tree_update(raw),
        ViewResultFormat::Streaming => build_streaming_update(raw),
    }
}

/// Build flat list output
fn build_flat_update(raw: RawViewResult) -> ViewUpdate {
    let hash = compute_flat_hash(&raw.records);
    ViewUpdate::Flat(MaterializedViewUpdate {
        query_id: raw.query_id,
        result_hash: hash,
        result_data: raw.records,
    })
}

/// Build tree structure output
/// TODO: Implement actual tree structure when needed
fn build_tree_update(raw: RawViewResult) -> ViewUpdate {
    // For now, tree is same as flat
    // Future: Build hierarchical structure based on parent-child relationships
    let hash = compute_flat_hash(&raw.records);
    ViewUpdate::Tree(MaterializedViewUpdate {
        query_id: raw.query_id,
        result_hash: hash,
        result_data: raw.records,
    })
}

/// Build streaming delta output
fn build_streaming_update(raw: RawViewResult) -> ViewUpdate {
    let capacity = raw.delta.len();
    let mut update = StreamingUpdate::with_capacity(raw.query_id, capacity);

    if raw.is_first_run {
        // First run: all records are Created
        for (id, version) in raw.records {
            update.records.push(DeltaRecord::created(id, version));
        }
    } else {
        // Map delta to events
        for (id, version) in raw.delta.additions {
            update.records.push(DeltaRecord::created(id, version));
        }
        for id in raw.delta.removals {
            update.records.push(DeltaRecord::deleted(id));
        }
        for (id, version) in raw.delta.updates {
            update.records.push(DeltaRecord::updated(id, version));
        }
    }

    ViewUpdate::Streaming(update)
}

// ============================================================================
// Convenience Builders
// ============================================================================

/// Build streaming update directly from delta components
/// (Useful when view.rs handles streaming specially)
pub fn build_streaming_delta(
    view_id: String,
    additions: &[(String, u64)],
    removals: &[String],
    updates: &[(String, u64)],
) -> StreamingUpdate {
    let capacity = additions.len() + removals.len() + updates.len();
    let mut update = StreamingUpdate::with_capacity(view_id, capacity);

    for (id, version) in additions {
        update.records.push(DeltaRecord::created(id.clone(), *version));
    }
    for id in removals {
        update.records.push(DeltaRecord::deleted(id.clone()));
    }
    for (id, version) in updates {
        update.records.push(DeltaRecord::updated(id.clone(), *version));
    }

    update
}

/// Build empty update for a given format
pub fn build_empty_update(query_id: String, format: ViewResultFormat) -> ViewUpdate {
    match format {
        ViewResultFormat::Flat => ViewUpdate::Flat(MaterializedViewUpdate {
            query_id,
            result_hash: compute_flat_hash(&[]),
            result_data: vec![],
        }),
        ViewResultFormat::Tree => ViewUpdate::Tree(MaterializedViewUpdate {
            query_id,
            result_hash: compute_flat_hash(&[]),
            result_data: vec![],
        }),
        ViewResultFormat::Streaming => ViewUpdate::Streaming(StreamingUpdate {
            view_id: query_id,
            records: vec![],
        }),
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_flat_hash_deterministic() {
        let data1 = vec![
            ("a:1".to_string(), 1u64),
            ("b:2".to_string(), 2u64),
        ];
        let data2 = vec![
            ("b:2".to_string(), 2u64),
            ("a:1".to_string(), 1u64),
        ];
        
        // Same data in different order should produce same hash
        assert_eq!(compute_flat_hash(&data1), compute_flat_hash(&data2));
    }

    #[test]
    fn test_build_streaming_update() {
        let mut raw = RawViewResult::new("test_view".to_string());
        raw.delta.add_addition("record:1".to_string(), 1);
        raw.delta.add_update("record:2".to_string(), 5);
        raw.delta.add_removal("record:3".to_string());

        let update = build_update(raw, ViewResultFormat::Streaming);
        
        if let ViewUpdate::Streaming(s) = update {
            assert_eq!(s.view_id, "test_view");
            assert_eq!(s.records.len(), 3);
            
            assert!(s.records.iter().any(|r| r.id == "record:1" && r.event == DeltaEvent::Created));
            assert!(s.records.iter().any(|r| r.id == "record:2" && r.event == DeltaEvent::Updated));
            assert!(s.records.iter().any(|r| r.id == "record:3" && r.event == DeltaEvent::Deleted));
        } else {
            panic!("Expected Streaming variant");
        }
    }

    #[test]
    fn test_build_flat_update() {
        let mut raw = RawViewResult::new("test_view".to_string());
        raw.records = vec![
            ("a:1".to_string(), 1),
            ("b:2".to_string(), 2),
        ];

        let update = build_update(raw, ViewResultFormat::Flat);
        
        if let ViewUpdate::Flat(m) = update {
            assert_eq!(m.query_id, "test_view");
            assert_eq!(m.result_data.len(), 2);
            assert!(!m.result_hash.is_empty());
        } else {
            panic!("Expected Flat variant");
        }
    }

    #[test]
    fn test_first_run_streaming() {
        let mut raw = RawViewResult::new("test_view".to_string());
        raw.is_first_run = true;
        raw.records = vec![
            ("a:1".to_string(), 1),
            ("b:2".to_string(), 1),
        ];

        let update = build_update(raw, ViewResultFormat::Streaming);
        
        if let ViewUpdate::Streaming(s) = update {
            assert_eq!(s.records.len(), 2);
            assert!(s.records.iter().all(|r| r.event == DeltaEvent::Created));
        } else {
            panic!("Expected Streaming variant");
        }
    }
}
