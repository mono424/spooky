# Implementation Plan: update.rs Improvements

## Overview

The `update.rs` module handles formatting view results into different output formats (Flat, Tree, Streaming). The current implementation needs updates to align with the new `SmolStr`-based view.rs and circuit.rs, plus cleanup and performance optimizations.

---

## Table of Contents

1. [Current State Analysis](#1-current-state-analysis)
2. [Issues Found](#2-issues-found)
3. [Proposed Changes](#3-proposed-changes)
4. [Implementation Details](#4-implementation-details)
5. [Migration Checklist](#5-migration-checklist)

---

## 1. Current State Analysis

### Current update.rs Structure (157 lines)

```
update.rs
├── Types
│   ├── ViewResultFormat (enum)
│   ├── ViewDelta (struct)
│   ├── RawViewResult (struct)
│   ├── MaterializedViewUpdate (struct)
│   ├── DeltaEvent (enum)
│   ├── DeltaRecord (struct)
│   ├── StreamingUpdate (struct)
│   └── ViewUpdate (enum)
├── Functions
│   ├── compute_flat_hash()
│   ├── build_update()
│   └── build_streaming_delta()
```

### Current Usage in view.rs

```rust
// view.rs imports
use super::update::{ViewResultFormat, ViewUpdate};
use super::update::{build_update, compute_flat_hash, RawViewResult, ViewDelta};

// Usage pattern (appears ~10 times in view.rs)
let view_delta = ViewDelta {
    additions: vec![SmolStr],
    removals: vec![SmolStr],
    updates: vec![SmolStr],
};

let raw_result = RawViewResult {
    query_id: self.plan.id.clone(),
    records: result_data,  // Vec<SmolStr>
    delta: Some(view_delta),
};

let update = build_update(raw_result, self.format.clone());
```

---

## 2. Issues Found

### 2.1 ✅ Already Updated: SmolStr Migration

The current update.rs already uses `SmolStr` for:
- `ViewDelta` fields
- `RawViewResult.records`
- `MaterializedViewUpdate.result_data`
- `DeltaRecord.id`

**Status:** Already aligned with view.rs ✅

### 2.2 ⚠️ Issue: Redundant Code in view.rs

The same pattern is repeated 5+ times in view.rs:

```rust
// Pattern repeated in:
// - apply_single_create() L286-330
// - apply_single_delete() L348-392
// - process_content_update() L129-157
// - build_content_update_notification() L197-222
// - process_batch() L429-474

let view_delta_struct = if is_first_run {
    None
} else {
    Some(ViewDelta { additions, removals, updates })
};

let raw_result = RawViewResult {
    query_id: self.plan.id.clone(),
    records: result_data,
    delta: view_delta_struct,
};

let update = build_update(raw_result, self.format.clone());

let hash = match &update {
    ViewUpdate::Flat(flat) | ViewUpdate::Tree(flat) => flat.result_hash.clone(),
    ViewUpdate::Streaming(_) => compute_flat_hash(&result_data),
};

// ... hash comparison and last_hash update
```

**Recommendation:** Add helper method to update.rs or View to reduce duplication.

### 2.3 ⚠️ Issue: `format.clone()` Called Every Time

```rust
let update = build_update(raw_result, self.format.clone());
//                                     ^^^^^^^^^^^^^^ Clone every call
```

**Fix:** `ViewResultFormat` is Copy-able (it's just an enum with no data).

### 2.4 ⚠️ Issue: Hash Computed Twice for Streaming

```rust
// In view.rs - hash computed before building update
let pre_hash = if matches!(self.format, ViewResultFormat::Streaming) {
    Some(compute_flat_hash(&result_data))
} else {
    None
};

// Then in build_update, hash is NOT computed for streaming
// But view.rs needs it for last_hash comparison
```

**Problem:** Streaming format doesn't include hash in `StreamingUpdate`, but view.rs needs it for change detection.

### 2.5 ⚠️ Issue: `build_streaming_delta` Unused

```rust
/// Build streaming delta update from ZSet delta
pub fn build_streaming_delta(
    query_id: String,
    delta: &[(SmolStr, i64)],
) -> StreamingUpdate { ... }
```

This function is defined but never used. Consider removing or documenting its purpose.

### 2.6 ⚠️ Issue: Tree Format is Just Flat

```rust
ViewResultFormat::Tree => {
    let hash = compute_flat_hash(&raw.records);
    ViewUpdate::Tree(MaterializedViewUpdate {
        query_id: raw.query_id,
        result_hash: hash,
        result_data: raw.records,
    })
}
```

Tree format is identical to Flat. Either implement it or remove it.

---

## 3. Proposed Changes

### 3.1 Add `ViewResultFormat` Copy Derive

```rust
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
//                                       ^^^^ Add Copy
pub enum ViewResultFormat {
    #[default]
    Flat,
    Tree,
    Streaming,
}
```

### 3.2 Add Builder Pattern for ViewUpdate

```rust
/// Builder for creating ViewUpdate with less boilerplate
pub struct ViewUpdateBuilder {
    query_id: String,
    records: Vec<SmolStr>,
    format: ViewResultFormat,
    delta: Option<ViewDelta>,
}

impl ViewUpdateBuilder {
    pub fn new(query_id: String, records: Vec<SmolStr>, format: ViewResultFormat) -> Self {
        Self {
            query_id,
            records,
            format,
            delta: None,
        }
    }
    
    pub fn with_delta(mut self, delta: ViewDelta) -> Self {
        self.delta = Some(delta);
        self
    }
    
    pub fn with_additions(mut self, additions: Vec<SmolStr>) -> Self {
        self.delta.get_or_insert_with(ViewDelta::default).additions = additions;
        self
    }
    
    pub fn with_removals(mut self, removals: Vec<SmolStr>) -> Self {
        self.delta.get_or_insert_with(ViewDelta::default).removals = removals;
        self
    }
    
    pub fn with_updates(mut self, updates: Vec<SmolStr>) -> Self {
        self.delta.get_or_insert_with(ViewDelta::default).updates = updates;
        self
    }
    
    /// Build the ViewUpdate and return (update, hash)
    pub fn build(self) -> (ViewUpdate, String) {
        let hash = compute_flat_hash(&self.records);
        
        let raw = RawViewResult {
            query_id: self.query_id,
            records: self.records,
            delta: self.delta,
        };
        
        let update = build_update(raw, self.format);
        (update, hash)
    }
}
```

### 3.3 Add Hash to StreamingUpdate (Optional)

```rust
#[derive(Serialize, Deserialize, Debug)]
pub struct StreamingUpdate {
    pub view_id: String,
    pub records: Vec<DeltaRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_hash: Option<String>,  // For change detection
}
```

### 3.4 Add ViewUpdate Helper Methods

```rust
impl ViewUpdate {
    /// Extract the result hash (for change detection)
    pub fn hash(&self) -> Option<&str> {
        match self {
            ViewUpdate::Flat(m) | ViewUpdate::Tree(m) => Some(&m.result_hash),
            ViewUpdate::Streaming(s) => s.snapshot_hash.as_deref(),
        }
    }
    
    /// Check if this update has any changes
    pub fn has_changes(&self) -> bool {
        match self {
            ViewUpdate::Flat(_) | ViewUpdate::Tree(_) => true, // Hash comparison done externally
            ViewUpdate::Streaming(s) => !s.records.is_empty(),
        }
    }
    
    /// Get the query/view ID
    pub fn query_id(&self) -> &str {
        match self {
            ViewUpdate::Flat(m) | ViewUpdate::Tree(m) => &m.query_id,
            ViewUpdate::Streaming(s) => &s.view_id,
        }
    }
}
```

### 3.5 Add ViewDelta Helper Methods

```rust
impl ViewDelta {
    /// Create an empty delta
    pub fn empty() -> Self {
        Self::default()
    }
    
    /// Create a delta with only additions
    pub fn additions_only(additions: Vec<SmolStr>) -> Self {
        Self { additions, removals: vec![], updates: vec![] }
    }
    
    /// Create a delta with only removals
    pub fn removals_only(removals: Vec<SmolStr>) -> Self {
        Self { additions: vec![], removals, updates: vec![] }
    }
    
    /// Create a delta with only updates
    pub fn updates_only(updates: Vec<SmolStr>) -> Self {
        Self { additions: vec![], removals: vec![], updates }
    }
    
    /// Check if delta is empty
    pub fn is_empty(&self) -> bool {
        self.additions.is_empty() && self.removals.is_empty() && self.updates.is_empty()
    }
    
    /// Total number of changes
    pub fn len(&self) -> usize {
        self.additions.len() + self.removals.len() + self.updates.len()
    }
}
```

### 3.6 Optimize compute_flat_hash

```rust
/// Compute hash from flat array of record IDs.
/// IMPORTANT: Sorts by record ID before hashing to ensure deterministic output.
pub fn compute_flat_hash(data: &[SmolStr]) -> String {
    if data.is_empty() {
        // Fast path for empty data
        return "empty".to_string();
    }
    
    // Optimization: Avoid allocation for small datasets
    if data.len() <= 16 {
        let mut sorted: SmallVec<[&SmolStr; 16]> = data.iter().collect();
        sorted.sort_unstable();
        
        let mut hasher = blake3::Hasher::new();
        for id in sorted {
            hasher.update(id.as_bytes());
            hasher.update(&[0]);
        }
        return hasher.finalize().to_hex().to_string();
    }
    
    // Standard path for larger datasets
    let mut sorted_data: Vec<_> = data.to_vec();
    sorted_data.sort_unstable();

    let mut hasher = blake3::Hasher::new();
    for id in sorted_data {
        hasher.update(id.as_bytes());
        hasher.update(&[0]);
    }
    hasher.finalize().to_hex().to_string()
}
```

### 3.7 Remove or Document Unused Function

```rust
// Option A: Remove
// Delete build_streaming_delta entirely

// Option B: Document and keep for future use
/// Build streaming delta update from raw ZSet delta.
/// 
/// Note: This is a lower-level API for direct ZSet-to-streaming conversion.
/// Most code should use `build_update()` instead.
#[allow(dead_code)]
pub fn build_streaming_delta(...) -> StreamingUpdate { ... }
```

---

## 4. Implementation Details

### 4.1 Full Updated update.rs

```rust
//! Update formatting logic (Strategy Pattern).
//!
//! This module is responsible for taking raw view results (IDs)
//! and formatting them into the desired output structure based on ViewResultFormat.

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;
use smallvec::SmallVec;

// ============================================================================
// Types
// ============================================================================

/// Output format strategy
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ViewResultFormat {
    /// Flat list: [id, ...] with hash
    #[default]
    Flat,
    /// Tree structure (future - currently same as Flat)
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
    /// Full snapshot of record IDs in the view
    pub records: Vec<SmolStr>,
    /// Delta info (None for first run/full snapshot)
    pub delta: Option<ViewDelta>,
}

/// Flat/Tree list output
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MaterializedViewUpdate {
    pub query_id: String,
    pub result_hash: String,
    pub result_data: Vec<SmolStr>,
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
    Tree(MaterializedViewUpdate),
    Streaming(StreamingUpdate),
}

impl ViewUpdate {
    /// Extract the result hash (for change detection)
    /// Returns None for Streaming format (use has_changes instead)
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

// ============================================================================
// Hash Functions
// ============================================================================

/// Compute hash from flat array of record IDs.
/// 
/// IMPORTANT: Sorts by record ID before hashing to ensure deterministic output
/// regardless of insertion order.
/// 
/// # Performance
/// - Uses SmallVec for datasets ≤16 records (avoids heap allocation)
/// - Returns "empty" for empty datasets (fast path)
pub fn compute_flat_hash(data: &[SmolStr]) -> String {
    if data.is_empty() {
        return String::from("e3b0c44298fc1c14"); // Empty hash prefix (consistent)
    }
    
    // Optimization: Use stack allocation for small datasets
    if data.len() <= 16 {
        let mut sorted: SmallVec<[&SmolStr; 16]> = data.iter().collect();
        sorted.sort_unstable();
        
        let mut hasher = blake3::Hasher::new();
        for id in sorted {
            hasher.update(id.as_bytes());
            hasher.update(&[0]);
        }
        return hasher.finalize().to_hex().to_string();
    }
    
    // Standard path for larger datasets
    let mut sorted_data: Vec<SmolStr> = data.to_vec();
    sorted_data.sort_unstable();

    let mut hasher = blake3::Hasher::new();
    for id in &sorted_data {
        hasher.update(id.as_bytes());
        hasher.update(&[0]);
    }
    hasher.finalize().to_hex().to_string()
}

// ============================================================================
// Build Functions
// ============================================================================

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
            // TODO: Implement actual tree structure
            // For now, same as Flat
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
                delta_records.reserve(delta.len());
                
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
                // First run - all records are "created"
                delta_records.reserve(raw.records.len());
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

/// Build update with pre-computed hash (optimization for when hash is already known)
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
        assert!(!hash.is_empty());
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
}
```

---

## 5. Migration Checklist

### Phase 1: Non-Breaking Changes
- [ ] Add `Copy` derive to `ViewResultFormat`
- [ ] Add `Clone` derive to `MaterializedViewUpdate`, `StreamingUpdate`, `ViewUpdate`
- [ ] Add `ViewDelta` helper methods (`empty()`, `is_empty()`, `len()`, etc.)
- [ ] Add `ViewUpdate` helper methods (`hash()`, `query_id()`, `record_count()`)
- [ ] Optimize `compute_flat_hash` with SmallVec for small datasets
- [ ] Add `build_update_with_hash` function
- [ ] Add unit tests

### Phase 2: view.rs Updates (Optional)
- [ ] Replace `self.format.clone()` with `self.format` (since Copy)
- [ ] Use `ViewDelta::additions_only()` etc. where applicable
- [ ] Consider extracting repeated build pattern into helper

### Phase 3: Cleanup
- [ ] Remove or document `build_streaming_delta` if unused
- [ ] Add TODO comment for Tree format implementation

---

## 6. Expected Benefits

| Change | Benefit |
|--------|---------|
| `ViewResultFormat: Copy` | Eliminates clone overhead |
| `ViewDelta` helpers | Cleaner code in view.rs |
| `ViewUpdate` helpers | Easier hash extraction |
| SmallVec in hash | Fewer allocations for small views |
| `build_update_with_hash` | Skip redundant hash computation |

---

## 7. Cargo.toml Addition

```toml
[dependencies]
smallvec = "1.11"  # If not already present
```

---

*Document Version: 1.0*
*Status: Ready for Implementation*