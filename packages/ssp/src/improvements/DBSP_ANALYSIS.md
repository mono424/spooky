# DBSP Module Analysis & Improvement Plan

## Executive Summary

Your DBSP (Database Stream Processing) module is a well-structured incremental view maintenance system built in Rust. It currently handles CREATE, UPDATE, and DELETE operations, but the batch processing is somewhat limited in how it groups and processes mixed operations across tables. This analysis covers how the system works, identifies bottlenecks, and provides concrete improvements for mixed-operation/mixed-table batching.

---

## 1. Current Architecture Overview

### Data Flow

```
User Input → ingest_batch() → table_deltas (per-table ZSets) 
    → dependency_graph lookup → affected views 
    → process_ingest() → ViewUpdate (Flat/Tree/Streaming)
```

### Key Components

| Component | File | Purpose |
|-----------|------|---------|
| `Circuit` | circuit.rs | Main orchestrator, holds DB + Views |
| `Database/Table` | circuit.rs | Storage layer with ZSets |
| `View` | view.rs | Query evaluation + caching |
| `Operator` | operators/*.rs | Query plan nodes (Scan, Filter, Join, etc.) |
| `SpookyValue` | types/spooky_value.rs | Optimized JSON alternative |
| `ZSet` | types/zset.rs | FxHashMap<SmolStr, i64> for weights |

### Current Batch Processing Flow

```rust
// circuit.rs:147-167
pub fn ingest_batch(
    &mut self,
    batch: Vec<(String, String, String, Value, String)>, // (table, op, id, record, hash)
    is_optimistic: bool,
) -> Vec<ViewUpdate> {
    // 1. Convert to SpookyValue (allocation heavy)
    let batch_spooky = batch.into_iter().map(|(t,o,i,r,h)| {
        (SmolStr::from(t), SmolStr::from(o), SmolStr::from(i), SpookyValue::from(r), h)
    }).collect();
    
    self.ingest_batch_spooky(batch_spooky, is_optimistic)
}
```

---

## 2. How Mixed Operations Currently Work

### ✅ What Works

1. **Mixed ops per record**: CREATE/UPDATE/DELETE are handled correctly per record
2. **Weight calculation**: Proper +1/-1 weights for create/delete
3. **Table deltas**: Each table gets its own delta ZSet
4. **Dependency graph**: Views are only processed if their source tables changed

### ⚠️ Current Limitations

1. **No op-type grouping**: All ops go through the same path regardless of operation type
2. **UPDATE treated as CREATE**: Updates have weight +1, same as creates (no diff)
3. **No batch-level optimization for same-table ops**: Each record processed individually
4. **Full scan fallback too aggressive**: Any subquery table change triggers full re-scan

### Code Evidence (circuit.rs:177-200)

```rust
for (table, op, id, record_spooky, hash) in batch {
    let weight: i64 = match op.as_str() {
        "CREATE" | "UPDATE" | "create" | "update" => 1,  // ⚠️ Same weight!
        "DELETE" | "delete" => -1,
        _ => 0,
    };
    // ... processes one by one
}
```

---

## 3. Performance Analysis

### Bottlenecks Identified

| Issue | Location | Impact | Priority |
|-------|----------|--------|----------|
| Repeated string conversions | circuit.rs:153-164 | O(n) allocations | HIGH |
| Clone in `process()` wrapper | view.rs:65-68 | Unnecessary delta clone | MEDIUM |
| Full scan on any subquery change | view.rs:95-99 | O(table_size) vs O(delta) | HIGH |
| HashSet creation in hot path | view.rs:199-200 | Allocation per batch | LOW |
| Sorting for dedup | circuit.rs:248-249 | O(n log n) | LOW |

### Memory Allocation Hot Spots

```rust
// view.rs:199 - Creates HashSet every batch
let updated_ids_set: std::collections::HashSet<&str> = 
    updated_record_ids.iter().map(|s| s.as_str()).collect();

// view.rs:310 - Vec allocation in first_run
let mut all_first_run_ids: Vec<String> = Vec::new();
```

---

## 4. Recommended Improvements

### 4.1 Mixed Operations Support (High Priority)

**Problem**: UPDATEs don't produce proper deltas (old value removal + new value addition)

**Solution**: Track old values for UPDATE operations

```rust
// NEW: Enhanced batch entry with optional old_record for updates
pub struct BatchEntry {
    pub table: SmolStr,
    pub op: Operation,
    pub id: SmolStr,
    pub record: SpookyValue,
    pub old_record: Option<SpookyValue>, // For UPDATEs
    pub hash: String,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Operation {
    Create,
    Update,
    Delete,
}

impl Circuit {
    pub fn ingest_batch_v2(
        &mut self,
        batch: Vec<BatchEntry>,
        is_optimistic: bool,
    ) -> Vec<ViewUpdate> {
        let mut table_deltas: FastMap<String, ZSet> = FastMap::default();
        
        for entry in batch {
            match entry.op {
                Operation::Create => {
                    // Simple: add with weight +1
                    let delta = table_deltas.entry(entry.table.to_string()).or_default();
                    *delta.entry(entry.id.clone()).or_insert(0) += 1;
                    
                    let tb = self.db.ensure_table(entry.table.as_str());
                    tb.update_row(entry.id, entry.record, entry.hash);
                }
                Operation::Update => {
                    // For proper incremental: -1 for old, +1 for new
                    // But ZSet model collapses to net change
                    // Key insight: If filter conditions haven't changed, 
                    // the record stays in view with same key
                    let tb = self.db.ensure_table(entry.table.as_str());
                    tb.update_row(entry.id.clone(), entry.record, entry.hash);
                    
                    // Mark as "touched" for version increment
                    let delta = table_deltas.entry(entry.table.to_string()).or_default();
                    *delta.entry(entry.id).or_insert(0) += 1;
                }
                Operation::Delete => {
                    let delta = table_deltas.entry(entry.table.to_string()).or_default();
                    *delta.entry(entry.id.clone()).or_insert(0) -= 1;
                    
                    let tb = self.db.ensure_table(entry.table.as_str());
                    tb.delete_row(&entry.id);
                }
            }
        }
        
        // Rest remains same...
        self.propagate_deltas(table_deltas, is_optimistic)
    }
}
```

### 4.2 Mixed Tables Optimization (High Priority)

**Problem**: Multiple tables in one batch aren't grouped efficiently

**Solution**: Pre-group by table before processing

```rust
impl Circuit {
    pub fn ingest_batch_grouped(
        &mut self,
        batch: Vec<BatchEntry>,
        is_optimistic: bool,
    ) -> Vec<ViewUpdate> {
        // Group by table for cache-friendly access
        let mut by_table: FastMap<SmolStr, Vec<BatchEntry>> = FastMap::default();
        for entry in batch {
            by_table.entry(entry.table.clone()).or_default().push(entry);
        }
        
        let mut table_deltas: FastMap<String, ZSet> = FastMap::default();
        
        // Process each table's entries together (better cache locality)
        for (table, entries) in by_table {
            let tb = self.db.ensure_table(table.as_str());
            let delta = table_deltas.entry(table.to_string()).or_default();
            
            for entry in entries {
                match entry.op {
                    Operation::Create | Operation::Update => {
                        tb.update_row(entry.id.clone(), entry.record, entry.hash);
                        *delta.entry(entry.id).or_insert(0) += 1;
                    }
                    Operation::Delete => {
                        tb.delete_row(&entry.id);
                        *delta.entry(entry.id).or_insert(0) -= 1;
                    }
                }
            }
        }
        
        self.propagate_deltas(table_deltas, is_optimistic)
    }
}
```

### 4.3 Reduce Allocations in Hot Path

```rust
// BEFORE (view.rs:199)
let updated_ids_set: std::collections::HashSet<&str> = 
    updated_record_ids.iter().map(|s| s.as_str()).collect();

// AFTER: Reuse allocation across calls
impl View {
    // Add field to View struct
    #[serde(skip)]
    scratch_set: std::collections::HashSet<SmolStr>,
    
    fn process_ingest(&mut self, ...) -> Option<ViewUpdate> {
        // Reuse scratch space
        self.scratch_set.clear();
        for id in &updated_record_ids {
            self.scratch_set.insert(SmolStr::new(id));
        }
        // Use self.scratch_set instead of creating new HashSet
    }
}
```

### 4.4 Smarter Subquery Change Detection

**Problem**: `has_changes_for_subqueries()` triggers full scan even for unrelated records

```rust
// BEFORE: Any positive weight in subquery table = full scan
if (*weight > 0 && !in_version_map) || (*weight < 0 && in_version_map) {
    return true; // Forces full scan!
}

// AFTER: Check if the changed record is actually referenced
fn has_relevant_subquery_changes(
    &self, 
    deltas: &FastMap<String, ZSet>, 
    db: &Database
) -> bool {
    let subquery_tables = self.extract_subquery_tables(&self.plan.root);
    
    for table in &subquery_tables {
        if let Some(delta) = deltas.get(table) {
            for (key, weight) in delta {
                // Only trigger full scan if:
                // 1. New record that COULD match a parent's subquery filter
                // 2. Deleted record that WAS in our version_map
                if *weight > 0 {
                    // Check if any parent record could reference this
                    if self.could_match_any_subquery_filter(key, db) {
                        return true;
                    }
                } else if *weight < 0 && self.version_map.contains_key(key.as_str()) {
                    return true;
                }
            }
        }
    }
    false
}
```

### 4.5 Parallel View Processing (Already Implemented, Verify Usage)

Your code has parallel processing but only for native targets:

```rust
#[cfg(all(feature = "parallel", not(target_arch = "wasm32")))]
let updates: Vec<ViewUpdate> = {
    use rayon::prelude::*;
    self.views
        .par_iter_mut()
        // ...
};
```

**Recommendation**: Add a threshold to avoid parallelism overhead for small batches

```rust
const PARALLEL_THRESHOLD: usize = 10; // Views

#[cfg(all(feature = "parallel", not(target_arch = "wasm32")))]
let updates: Vec<ViewUpdate> = {
    if impacted_view_indices.len() >= PARALLEL_THRESHOLD {
        use rayon::prelude::*;
        self.views.par_iter_mut()
            // parallel path
    } else {
        // sequential path (avoid thread pool overhead)
        self.views.iter_mut()
            // sequential
    }
};
```

---

## 5. API Improvements

### Current API

```rust
// Requires pre-computed hash
fn ingest_batch(
    batch: Vec<(String, String, String, Value, String)>, // table, op, id, record, hash
    is_optimistic: bool,
) -> Vec<ViewUpdate>
```

### Proposed Ergonomic API

```rust
// Builder pattern for cleaner batching
pub struct IngestBatch {
    entries: Vec<BatchEntry>,
}

impl IngestBatch {
    pub fn new() -> Self { Self { entries: Vec::new() } }
    
    pub fn create(mut self, table: &str, id: &str, record: Value) -> Self {
        self.entries.push(BatchEntry {
            table: SmolStr::new(table),
            op: Operation::Create,
            id: SmolStr::new(id),
            record: SpookyValue::from(record),
            old_record: None,
            hash: String::new(), // Computed lazily
        });
        self
    }
    
    pub fn update(mut self, table: &str, id: &str, record: Value) -> Self {
        // Similar...
        self
    }
    
    pub fn delete(mut self, table: &str, id: &str) -> Self {
        // Similar...
        self
    }
}

impl Circuit {
    pub fn ingest(&mut self, batch: IngestBatch, optimistic: bool) -> Vec<ViewUpdate> {
        // Compute hashes in parallel if needed
        let prepared = batch.prepare();
        self.ingest_batch_v2(prepared, optimistic)
    }
}

// Usage:
let updates = circuit.ingest(
    IngestBatch::new()
        .create("users", "user:1", json!({"name": "Alice"}))
        .update("posts", "post:42", json!({"title": "Updated"}))
        .delete("comments", "comment:99"),
    true
);
```

---

## 6. Clean Code Improvements

### 6.1 Extract Constants

```rust
// constants.rs
pub const OP_CREATE: &str = "CREATE";
pub const OP_UPDATE: &str = "UPDATE";  
pub const OP_DELETE: &str = "DELETE";

pub const STREAMING_SENTINEL_HASH: &str = "streaming";
```

### 6.2 Reduce Nesting in `process_ingest()`

The `process_ingest()` function in view.rs is 500+ lines. Split into smaller functions:

```rust
impl View {
    pub fn process_ingest(&mut self, deltas: &FastMap<String, ZSet>, db: &Database, is_optimistic: bool) -> Option<ViewUpdate> {
        let context = self.prepare_context(deltas, db);
        
        let view_delta = self.compute_view_delta(&context, deltas, db)?;
        let (additions, removals, updates) = self.categorize_changes(&view_delta, &context);
        
        match self.format {
            ViewResultFormat::Streaming => self.emit_streaming_update(additions, removals, updates, db, is_optimistic),
            _ => self.emit_materialized_update(additions, removals, updates, db, is_optimistic),
        }
    }
    
    fn prepare_context(&self, deltas: &FastMap<String, ZSet>, db: &Database) -> ProcessContext {
        ProcessContext {
            is_first_run: self.last_hash.is_empty(),
            is_streaming: matches!(self.format, ViewResultFormat::Streaming),
            has_subquery_changes: !self.last_hash.is_empty() && self.has_changes_for_subqueries(deltas, db),
        }
    }
    
    fn compute_view_delta(&self, ctx: &ProcessContext, deltas: &FastMap<String, ZSet>, db: &Database) -> Option<ZSet> {
        if ctx.is_first_run || ctx.has_subquery_changes {
            self.compute_full_scan_delta(db)
        } else {
            self.eval_delta_batch(&self.plan.root, deltas, db, self.params.as_ref())
        }
    }
    
    // ... more focused functions
}
```

### 6.3 Better Error Handling

```rust
// Instead of silent None returns
pub enum ViewProcessError {
    EmptyDelta,
    NoChanges,
    FilterMismatch,
}

pub fn process_ingest(&mut self, ...) -> Result<ViewUpdate, ViewProcessError> {
    // Explicit error handling
}
```

---

## 7. Summary of Changes

| Change | Complexity | Impact | Files |
|--------|------------|--------|-------|
| Mixed ops with Operation enum | Medium | High | circuit.rs, lib.rs |
| Group by table before processing | Low | Medium | circuit.rs |
| Reuse allocations (scratch sets) | Low | Medium | view.rs |
| Smarter subquery change detection | Medium | High | view.rs |
| Parallel threshold | Low | Low | circuit.rs |
| Builder API | Medium | High (DX) | service.rs, lib.rs |
| Extract 500-line function | Medium | High (maintainability) | view.rs |

---

## 8. Implementation Priority

1. **Phase 1** (Quick Wins):
   - Add Operation enum
   - Group by table before processing
   - Add parallel threshold

2. **Phase 2** (Performance):
   - Reuse scratch allocations
   - Smarter subquery detection
   - Benchmark before/after

3. **Phase 3** (API/DX):
   - Builder pattern API
   - Better error types
   - Code refactoring

---

## Appendix: Test Cases for Mixed Operations

```rust
#[test]
fn test_mixed_ops_same_table() {
    let mut circuit = Circuit::new();
    
    let updates = circuit.ingest(
        IngestBatch::new()
            .create("users", "user:1", json!({"name": "Alice"}))
            .create("users", "user:2", json!({"name": "Bob"}))
            .update("users", "user:1", json!({"name": "Alice Smith"}))
            .delete("users", "user:2"),
        true
    );
    
    // Should have user:1 with updated name, no user:2
}

#[test]
fn test_mixed_ops_multi_table() {
    let mut circuit = Circuit::new();
    // Register view that joins users + posts
    
    let updates = circuit.ingest(
        IngestBatch::new()
            .create("users", "user:1", json!({"name": "Alice"}))
            .create("posts", "post:1", json!({"author": "user:1", "title": "Hello"}))
            .update("users", "user:1", json!({"name": "Alice Smith"})),
        true
    );
    
    // View should reflect all changes in single batch
}
```
