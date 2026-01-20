# DBSP Architecture Migration - Implementation Plan

## Overview

This plan migrates the current DBSP engine to the new 3-module architecture:
- **view.rs** → Pure delta computation (outputs `RawViewResult`)
- **metadata.rs** → Pluggable versioning strategies (NEW)
- **update.rs** → Output formatting (restored to original design)

**Priority:** Performance > Clean Code > Features

---

## Phase 1: Foundation (Non-Breaking Changes)

### 1.1 Add New Files

| Task | File | Effort |
|------|------|--------|
| ✅ Add metadata.rs | `engine/metadata.rs` | Done |
| ✅ Update update.rs | `engine/update.rs` | Done |
| ✅ Update mod.rs | `engine/mod.rs` | Done |

### 1.2 Update types/mod.rs

Remove `VersionMap` export (now in metadata.rs):

```rust
// types/mod.rs
mod path;
mod spooky_value;
mod zset;

pub use path::Path;
pub use spooky_value::SpookyValue;
pub use zset::{FastMap, RowKey, Weight, ZSet};  // Remove VersionMap
```

### 1.3 Update types/zset.rs

Remove `VersionMap` type alias (moved to metadata.rs):

```rust
// Remove this line from zset.rs:
// pub type VersionMap = FastMap<SmolStr, u64>;
```

**Estimated Time: 15 minutes**

---

## Phase 2: View.rs Refactoring

### 2.1 Update Imports

```rust
// view.rs - NEW imports
use super::metadata::{MetadataProcessor, ViewMetadataState, VersionStrategy};
use super::update::{
    build_update, RawViewResult, ViewDelta, ViewResultFormat,
    DeltaEvent, DeltaRecord, MaterializedViewUpdate, StreamingUpdate, ViewUpdate,
};

// REMOVE these re-exports (now from update.rs/metadata.rs)
// pub use super::update::{...};  // Keep only what's needed internally
```

### 2.2 Update View Struct

**Current:**
```rust
pub struct View {
    pub plan: QueryPlan,
    pub cache: ZSet,
    pub last_hash: String,
    pub params: Option<SpookyValue>,
    pub version_map: VersionMap,  // REMOVE
    pub format: ViewResultFormat,
}
```

**New:**
```rust
pub struct View {
    pub plan: QueryPlan,
    pub cache: ZSet,
    pub params: Option<SpookyValue>,
    pub format: ViewResultFormat,
    
    // NEW: Metadata state (replaces version_map + last_hash)
    #[serde(default)]
    pub metadata: ViewMetadataState,
}
```

### 2.3 Update View::new()

```rust
impl View {
    pub fn new(plan: QueryPlan, params: Option<Value>, format: Option<ViewResultFormat>) -> Self {
        let fmt = format.unwrap_or_default();
        
        // Determine version strategy based on format
        let strategy = match fmt {
            ViewResultFormat::Tree => VersionStrategy::HashBased,
            _ => VersionStrategy::Optimistic,
        };
        
        Self {
            plan,
            cache: FastMap::default(),
            params: params.map(SpookyValue::from),
            format: fmt,
            metadata: ViewMetadataState::new(strategy),
        }
    }
    
    // NEW: Constructor with explicit metadata config
    pub fn new_with_strategy(
        plan: QueryPlan,
        params: Option<Value>,
        format: Option<ViewResultFormat>,
        strategy: VersionStrategy,
    ) -> Self {
        Self {
            plan,
            cache: FastMap::default(),
            params: params.map(SpookyValue::from),
            format: format.unwrap_or_default(),
            metadata: ViewMetadataState::new(strategy),
        }
    }
}
```

### 2.4 Refactor process_ingest() - Core Change

**Strategy:** Keep the delta computation logic, but output `RawViewResult` and delegate formatting.

```rust
pub fn process_ingest(
    &mut self,
    deltas: &FastMap<String, ZSet>,
    db: &Database,
    is_optimistic: bool,
) -> Option<ViewUpdate> {
    self.process_ingest_with_meta(deltas, db, is_optimistic, None)
}

/// NEW: Process with optional explicit metadata
pub fn process_ingest_with_meta(
    &mut self,
    deltas: &FastMap<String, ZSet>,
    db: &Database,
    is_optimistic: bool,
    batch_meta: Option<&BatchMeta>,
) -> Option<ViewUpdate> {
    let is_first_run = self.metadata.is_first_run();
    let is_streaming = matches!(self.format, ViewResultFormat::Streaming);
    let has_subquery_changes = !is_first_run && self.has_changes_for_subqueries(deltas, db);

    // Step 1: Compute view delta (UNCHANGED - pure DBSP logic)
    let view_delta = self.compute_view_delta(is_first_run, has_subquery_changes, deltas, db);
    
    // Step 2: Identify updated records (UNCHANGED)
    let updated_record_ids = self.identify_updated_records(is_streaming, deltas);
    
    // Step 3: Categorize changes (UNCHANGED)
    let (additions, removals, updates) = self.categorize_changes(&view_delta, &updated_record_ids);
    
    // Step 4: Early exit check
    if additions.is_empty() && removals.is_empty() && updates.is_empty() 
        && !is_first_run && !has_subquery_changes {
        return None;
    }
    
    // Step 5: Build RawViewResult (NEW - delegates to helper)
    let raw_result = self.build_raw_result(
        &additions,
        &removals, 
        &updates,
        is_first_run,
        has_subquery_changes,
        is_optimistic,
        batch_meta,
        db,
    )?;
    
    // Step 6: Format output (NEW - delegates to update.rs)
    let update = build_update(raw_result, self.format.clone());
    
    // Step 7: Check for actual change (hash comparison for Flat/Tree)
    if self.should_emit_update(&update) {
        Some(update)
    } else {
        None
    }
}
```

### 2.5 New Helper: build_raw_result()

```rust
fn build_raw_result(
    &mut self,
    additions: &[String],
    removals: &[String],
    updates: &[String],
    is_first_run: bool,
    has_subquery_changes: bool,
    is_optimistic: bool,
    batch_meta: Option<&BatchMeta>,
    db: &Database,
) -> Option<RawViewResult> {
    let processor = MetadataProcessor::new(self.metadata.strategy.clone());
    let is_streaming = matches!(self.format, ViewResultFormat::Streaming);
    
    // Pre-allocate result
    let mut raw = RawViewResult::with_capacity(
        self.plan.id.clone(),
        additions.len() + removals.len() + updates.len(),
    );
    raw.is_first_run = is_first_run;
    
    if is_streaming {
        self.build_streaming_raw_result(
            &mut raw, additions, removals, updates,
            is_first_run, has_subquery_changes, is_optimistic,
            &processor, batch_meta, db,
        );
    } else {
        self.build_materialized_raw_result(
            &mut raw, additions, removals, updates,
            is_optimistic, &processor, batch_meta, db,
        );
    }
    
    Some(raw)
}
```

### 2.6 New Helper: build_streaming_raw_result()

```rust
fn build_streaming_raw_result(
    &mut self,
    raw: &mut RawViewResult,
    additions: &[String],
    removals: &[String],
    updates: &[String],
    is_first_run: bool,
    has_subquery_changes: bool,
    is_optimistic: bool,
    processor: &MetadataProcessor,
    batch_meta: Option<&BatchMeta>,
    db: &Database,
) {
    if is_first_run {
        // First run: collect all IDs including subqueries
        let all_ids = self.collect_all_view_ids(db);
        for id in all_ids {
            let version = self.compute_and_store_version(&id, processor, batch_meta, true, false);
            raw.records.push((id.clone(), version));
            raw.delta.add_addition(id, version);
        }
    } else if has_subquery_changes {
        // Re-evaluate all subqueries
        self.handle_subquery_changes_streaming(raw, processor, batch_meta, db);
    } else {
        // Normal streaming update
        self.handle_normal_streaming(
            raw, additions, removals, updates,
            is_optimistic, processor, batch_meta, db,
        );
    }
}
```

### 2.7 New Helper: compute_and_store_version()

```rust
/// Compute version using processor and store in metadata
#[inline]
fn compute_and_store_version(
    &mut self,
    id: &str,
    processor: &MetadataProcessor,
    batch_meta: Option<&BatchMeta>,
    is_new: bool,
    is_optimistic: bool,
) -> u64 {
    let current = self.metadata.get_version(id);
    let record_meta = batch_meta.and_then(|bm| bm.get(id));
    
    let result = if is_new {
        processor.compute_new_version(id, current, record_meta)
    } else {
        processor.compute_update_version(id, current, record_meta, is_optimistic)
    };
    
    if result.changed || is_new {
        self.metadata.set_version(id, result.version);
    }
    
    result.version
}
```

### 2.8 New Helper: should_emit_update()

```rust
/// Check if update should be emitted (hash comparison for Flat/Tree)
fn should_emit_update(&mut self, update: &ViewUpdate) -> bool {
    match update {
        ViewUpdate::Streaming(_) => {
            // Streaming always emits if there are records
            self.metadata.last_result_hash = "streaming".to_string();
            true
        }
        ViewUpdate::Flat(m) | ViewUpdate::Tree(m) => {
            if m.result_hash != self.metadata.last_result_hash {
                self.metadata.last_result_hash = m.result_hash.clone();
                true
            } else {
                false
            }
        }
    }
}
```

### 2.9 Migration Mapping

| Old Code | New Code |
|----------|----------|
| `self.version_map.get(id)` | `self.metadata.get_version(id)` |
| `self.version_map.insert(id, v)` | `self.metadata.set_version(id, v)` |
| `self.version_map.remove(id)` | `self.metadata.remove(id)` |
| `self.version_map.contains_key(id)` | `self.metadata.contains(id)` |
| `self.last_hash` | `self.metadata.last_result_hash` |
| `self.last_hash.is_empty()` | `self.metadata.is_first_run()` |

**Estimated Time: 2-3 hours**

---

## Phase 3: Circuit.rs Updates

### 3.1 Add BatchMeta Support

```rust
/// Ingest with explicit metadata
pub fn ingest_with_meta(
    &mut self,
    batch: IngestBatch,
    meta: BatchMeta,
    is_optimistic: bool,
) -> Vec<ViewUpdate> {
    self.ingest_entries_with_meta(batch.build(), Some(meta), is_optimistic)
}

/// Internal: Ingest entries with optional metadata
pub fn ingest_entries_with_meta(
    &mut self,
    entries: Vec<BatchEntry>,
    meta: Option<BatchMeta>,
    is_optimistic: bool,
) -> Vec<ViewUpdate> {
    if entries.is_empty() {
        return Vec::new();
    }

    // ... existing grouping logic ...
    
    self.propagate_deltas_with_meta(table_deltas, meta, is_optimistic)
}

fn propagate_deltas_with_meta(
    &mut self,
    table_deltas: FastMap<String, ZSet>,
    meta: Option<BatchMeta>,
    is_optimistic: bool,
) -> Vec<ViewUpdate> {
    // ... existing delta application ...
    
    self.process_impacted_views_with_meta(&impacted_indices, &table_deltas, meta.as_ref(), is_optimistic)
}

fn process_impacted_views_with_meta(
    &mut self,
    indices: &[usize],
    deltas: &FastMap<String, ZSet>,
    meta: Option<&BatchMeta>,
    is_optimistic: bool,
) -> Vec<ViewUpdate> {
    // Sequential path
    let mut updates = Vec::with_capacity(indices.len());
    for &i in indices {
        if i < self.views.len() {
            if let Some(update) = self.views[i].process_ingest_with_meta(
                deltas, &self.db, is_optimistic, meta
            ) {
                updates.push(update);
            }
        }
    }
    updates
}
```

### 3.2 Keep Backward Compatibility

```rust
// Existing methods unchanged - they call the new internal methods
pub fn ingest_entries(&mut self, entries: Vec<BatchEntry>, is_optimistic: bool) -> Vec<ViewUpdate> {
    self.ingest_entries_with_meta(entries, None, is_optimistic)
}

pub fn ingest(&mut self, batch: IngestBatch, is_optimistic: bool) -> Vec<ViewUpdate> {
    self.ingest_entries(batch.build(), is_optimistic)
}
```

**Estimated Time: 1 hour**

---

## Phase 4: Performance Optimizations

### 4.1 High Priority (Do Now)

| Optimization | Location | Impact | Effort |
|--------------|----------|--------|--------|
| **Inline hot-path methods** | metadata.rs | High | Low |
| **Pre-allocate vectors** | view.rs | Medium | Low |
| **Avoid String clones** | view.rs | Medium | Medium |

#### 4.1.1 Add #[inline] Annotations

```rust
// metadata.rs - Already done, verify these are present:
impl MetadataProcessor {
    #[inline]
    pub fn compute_new_version(...) { ... }
    
    #[inline]
    pub fn compute_update_version(...) { ... }
}

impl ViewMetadataState {
    #[inline]
    pub fn get_version(&self, id: &str) -> u64 { ... }
    
    #[inline]
    pub fn set_version(&mut self, id: impl Into<SmolStr>, version: u64) { ... }
    
    #[inline]
    pub fn contains(&self, id: &str) -> bool { ... }
}
```

#### 4.1.2 Pre-allocate in build_raw_result()

```rust
fn build_raw_result(...) -> Option<RawViewResult> {
    // Estimate capacity
    let estimated_size = additions.len() + updates.len();
    let mut raw = RawViewResult::with_capacity(self.plan.id.clone(), estimated_size);
    
    // Reserve in metadata if needed
    if additions.len() > 10 {
        self.metadata.reserve(additions.len());
    }
    
    // ...
}
```

#### 4.1.3 Use SmolStr in ViewDelta

```rust
// update.rs - Change ViewDelta to use SmolStr internally
pub struct ViewDelta {
    pub additions: Vec<(SmolStr, u64)>,  // SmolStr instead of String
    pub removals: Vec<SmolStr>,
    pub updates: Vec<(SmolStr, u64)>,
}

// Convert to String only at serialization boundary
```

### 4.2 Medium Priority (Do Later)

| Optimization | Location | Impact | Effort |
|--------------|----------|--------|--------|
| **Batch version_map updates** | view.rs | Medium | Medium |
| **Reuse HashSet allocations** | view.rs | Low | Low |
| **Parallel view processing** | circuit.rs | High | Already done |

#### 4.2.1 Batch Metadata Updates

```rust
// Instead of multiple insert calls:
for id in &additions {
    self.metadata.set_version(id, 1);
}

// Batch update:
impl ViewMetadataState {
    pub fn set_versions_batch(&mut self, items: &[(SmolStr, u64)]) {
        self.versions.reserve(items.len());
        for (id, version) in items {
            self.versions.insert(id.clone(), *version);
        }
    }
}
```

#### 4.2.2 Reuse HashSet

```rust
// Add thread-local or struct-level scratch space
struct ViewScratch {
    updated_set: HashSet<SmolStr>,
    removal_set: HashSet<SmolStr>,
}

impl View {
    // Reuse scratch space across calls
    fn get_scratch(&self) -> &mut ViewScratch { ... }
}
```

### 4.3 Low Priority (Future)

| Optimization | Location | Impact | Effort |
|--------------|----------|--------|--------|
| **SIMD hash comparison** | update.rs | Low | High |
| **Arena allocation** | view.rs | Medium | High |
| **Zero-copy delta transfer** | All | Medium | High |

---

## Phase 5: Testing

### 5.1 Unit Tests

```rust
// tests/metadata_tests.rs
#[test]
fn test_optimistic_versioning() {
    let processor = MetadataProcessor::new(VersionStrategy::Optimistic);
    // ... test version increment
}

#[test]
fn test_explicit_versioning() {
    let meta = RecordMeta::new().with_version(42);
    let processor = MetadataProcessor::new(VersionStrategy::Explicit);
    let result = processor.compute_new_version("test:1", 0, Some(&meta));
    assert_eq!(result.version, 42);
}

#[test]
fn test_hash_based_versioning() {
    // ... test hash change detection
}
```

### 5.2 Integration Tests

```rust
// tests/integration_tests.rs
#[test]
fn test_streaming_with_explicit_version() {
    let mut circuit = Circuit::new();
    circuit.register_view(plan, None, Some(ViewResultFormat::Streaming));
    
    let meta = BatchMeta::new()
        .with_strategy(VersionStrategy::Explicit)
        .add_record("user:1", RecordMeta::new().with_version(13));
    
    let updates = circuit.ingest_with_meta(
        IngestBatch::new().update("users", "user:1", data, hash),
        meta,
        false,
    );
    
    // Verify version is 13, not auto-incremented
    if let ViewUpdate::Streaming(s) = &updates[0] {
        assert_eq!(s.records[0].version, 13);
    }
}
```

### 5.3 Backward Compatibility Tests

```rust
#[test]
fn test_existing_api_unchanged() {
    let mut circuit = Circuit::new();
    
    // Old API still works
    circuit.ingest_record("users", "CREATE", "user:1", json!({}), "hash", true);
    circuit.ingest_batch(vec![...], true);
    circuit.ingest(IngestBatch::new()..., true);
}
```

**Estimated Time: 2-3 hours**

---

## Implementation Order

### Week 1: Foundation
1. ✅ Create metadata.rs
2. ✅ Update update.rs
3. ✅ Update mod.rs
4. [ ] Update types/mod.rs and types/zset.rs
5. [ ] Verify compilation

### Week 2: View Refactoring
1. [ ] Update View struct
2. [ ] Update View::new()
3. [ ] Refactor process_ingest()
4. [ ] Add helper methods
5. [ ] Update all version_map references

### Week 3: Circuit & Testing
1. [ ] Add ingest_with_meta() to circuit.rs
2. [ ] Write unit tests
3. [ ] Write integration tests
4. [ ] Verify backward compatibility

### Week 4: Performance & Polish
1. [ ] Add #[inline] annotations
2. [ ] Pre-allocate vectors
3. [ ] Profile and optimize hot paths
4. [ ] Documentation

---

## Checklist

### Phase 1: Foundation
- [ ] Add metadata.rs to engine/
- [ ] Update update.rs with RawViewResult, ViewDelta
- [ ] Update mod.rs exports
- [ ] Remove VersionMap from types/

### Phase 2: View Refactoring
- [ ] Update View struct (metadata field)
- [ ] Update View::new()
- [ ] Add new_with_strategy()
- [ ] Refactor process_ingest()
- [ ] Add process_ingest_with_meta()
- [ ] Add build_raw_result()
- [ ] Add build_streaming_raw_result()
- [ ] Add build_materialized_raw_result()
- [ ] Add compute_and_store_version()
- [ ] Add should_emit_update()
- [ ] Migrate all version_map references
- [ ] Migrate all last_hash references

### Phase 3: Circuit Updates
- [ ] Add ingest_with_meta()
- [ ] Add ingest_entries_with_meta()
- [ ] Add propagate_deltas_with_meta()
- [ ] Add process_impacted_views_with_meta()
- [ ] Verify backward compatibility

### Phase 4: Performance
- [ ] Add #[inline] to hot paths
- [ ] Pre-allocate vectors
- [ ] Consider SmolStr in ViewDelta
- [ ] Profile and measure

### Phase 5: Testing
- [ ] Unit tests for metadata.rs
- [ ] Unit tests for update.rs
- [ ] Integration tests
- [ ] Backward compatibility tests
- [ ] Performance benchmarks

---

## Risk Mitigation

| Risk | Mitigation |
|------|------------|
| Breaking existing API | All existing methods remain, new ones added |
| Performance regression | Profile before/after, #[inline] hot paths |
| Serialization issues | ViewMetadataState is serde-compatible |
| Complex migration | Incremental phases, tests at each step |

---

## Summary

**Total Estimated Time:** 1-2 weeks

**Key Benefits:**
1. Clean separation of concerns
2. Pluggable versioning strategies
3. Explicit version support (your requirement)
4. Hash-based versioning for Tree mode
5. No breaking changes to existing API
6. Performance maintained or improved
