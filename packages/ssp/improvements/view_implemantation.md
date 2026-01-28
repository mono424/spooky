# View.rs Refactoring & Optimization Plan

## Executive Summary

This document provides a comprehensive analysis of `view.rs` (~700 lines) and outlines a refactoring plan to improve code clarity, remove redundancy, and optimize performance for both single-record and batch processing.

---

## Table of Contents

1. [Current Architecture Analysis](#1-current-architecture-analysis)
2. [Identified Issues](#2-identified-issues)
3. [Proposed Architecture](#3-proposed-architecture)
4. [Detailed Refactoring Plan](#4-detailed-refactoring-plan)
5. [Implementation Phases](#5-implementation-phases)
6. [Performance Optimizations](#6-performance-optimizations)

---

## 1. Current Architecture Analysis

### 1.1 File Structure (698 lines)

```
view.rs
├── Structs (L1-48)
│   ├── QueryPlan
│   └── View
├── Main Processing (L49-223)
│   ├── new()
│   ├── process_single()      ← Wrapper around process_ingest
│   ├── process()             ← REDUNDANT, similar to process_single
│   └── process_ingest()      ← Main logic (150+ lines!)
├── Subquery Helpers (L227-413)
│   ├── collect_subquery_ids_recursive()
│   ├── collect_nested_subquery_ids()
│   ├── get_updated_cached_records()
│   ├── extract_subquery_tables()
│   ├── collect_subquery_tables()
│   └── collect_tables_from_operator()
├── Evaluation Engine (L415-589)
│   ├── eval_delta_batch()
│   ├── eval_delta()          ← DEPRECATED/DEAD CODE
│   └── eval_snapshot()
├── Row Access (L591-602)
│   ├── get_row_value()
│   └── get_row_hash()        ← DEAD CODE (hashes removed from Table)
└── Predicate Checking (L604-697)
    └── check_predicate()
```

### 1.2 Data Flow

```
┌─────────────────────────────────────────────────────────────────────┐
│                        process_ingest()                             │
├─────────────────────────────────────────────────────────────────────┤
│  1. Check if first run                                              │
│  2. Try incremental eval (eval_delta_batch)                         │
│  3. If fails → Full scan (eval_snapshot) + diff                     │
│  4. Detect updated records                                          │
│  5. Update cache                                                    │
│  6. Build additions/removals/updates lists                          │
│  7. Build result data                                               │
│  8. Build ViewUpdate via update module                              │
│  9. Check hash, return if changed                                   │
└─────────────────────────────────────────────────────────────────────┘
```

---

## 2. Identified Issues

### 2.1 Redundant/Dead Code

| Code | Location | Issue |
|------|----------|-------|
| `process()` | L59-70 | Redundant wrapper, same as `process_single` logic |
| `eval_delta()` | L456-469 | Marked deprecated, dead code |
| `get_row_hash()` | L599-602 | Dead code - `Table.hashes` was removed |
| `collect_subquery_ids_recursive()` | L227-278 | Very similar to `collect_nested_subquery_ids()` |
| `collect_subquery_tables()` | L364-387 | Similar pattern to `collect_tables_from_operator()` |

### 2.2 Performance Issues

| Issue | Location | Impact |
|-------|----------|--------|
| `process_single` calls `process_ingest` with HashMap overhead | L50-57 | Creates 2 HashMaps for single record |
| Repeated `.to_string()` allocations | L163, 168, 251, etc. | Unnecessary heap allocations |
| `result_data.clone()` | L204 | Clones entire result vector |
| `additions.clone()`, `removals.clone()`, `updates.clone()` | L196-198 | Triple clone for ViewDelta |
| No pre-allocation for subquery collectors | L234, 335 | Vec reallocation |
| HashSet created every call | L153, 173 | Could reuse or avoid |

### 2.3 Code Complexity Issues

| Issue | Location | Problem |
|-------|----------|---------|
| `process_ingest()` is 150+ lines | L72-223 | Too long, hard to maintain |
| Nested match statements | L236-277, 290-328 | Duplicated patterns |
| Inline closure `strip_prefix` | L149-151 | Should be a helper function |
| Inline closure `resolve_val` | L612-639 | Complex, should be extracted |
| Long comment blocks | L70-94 | Outdated/confusing comments |

### 2.4 API Design Issues

| Issue | Problem |
|-------|---------|
| `process_single` not optimized | Creates HashMap, delegates to batch method |
| `process` vs `process_single` | Confusing API, both do similar things |
| No true single-record fast path | Always goes through batch processing logic |

---

## 3. Proposed Architecture

### 3.1 New File Structure

```
view.rs
├── mod.rs (re-exports)
├── types.rs
│   └── QueryPlan, View struct
├── processing/
│   ├── mod.rs
│   ├── single.rs      ← Optimized process_single_impl()
│   ├── batch.rs       ← Optimized process_batch_impl()
│   └── common.rs      ← Shared logic (cache update, result building)
├── evaluation/
│   ├── mod.rs
│   ├── delta.rs       ← eval_delta_batch()
│   └── snapshot.rs    ← eval_snapshot()
├── predicate.rs       ← check_predicate() and helpers
└── helpers.rs         ← get_row_value(), strip_table_prefix(), etc.
```

**Alternative: Keep single file but reorganize sections**

```
view.rs (reorganized)
├── Types & Structs (50 lines)
├── Public API (30 lines)
│   ├── new()
│   ├── process_single()    ← TRUE single-record optimization
│   └── process_batch()     ← Renamed from process_ingest
├── Internal Processing (100 lines)
│   ├── compute_delta()
│   ├── apply_cache_update()
│   └── build_view_update()
├── Evaluation Engine (150 lines)
│   ├── eval_delta_batch()
│   └── eval_snapshot()
├── Predicate Checking (100 lines)
│   └── check_predicate()
└── Helpers (50 lines)
    ├── get_row_value()
    └── strip_table_prefix()
```

### 3.2 New API Design

```rust
impl View {
    /// Create a new view
    pub fn new(plan: QueryPlan, params: Option<Value>, format: Option<ViewResultFormat>) -> Self;
    
    /// Process a single record change - OPTIMIZED for single record
    /// Does NOT create intermediate HashMaps when possible
    pub fn process_single(&mut self, delta: &Delta, db: &Database) -> Option<ViewUpdate>;
    
    /// Process batch of changes - optimized for multiple records/tables  
    pub fn process_batch(&mut self, deltas: &FastMap<String, ZSet>, db: &Database) -> Option<ViewUpdate>;
    
    /// Initialize view from scratch (for registration)
    pub fn initialize(&mut self, db: &Database) -> Option<ViewUpdate>;
}
```

---

## 4. Detailed Refactoring Plan

### 4.1 Remove Dead Code

```rust
// DELETE these:

// L59-70: process() - redundant
pub fn process(...) -> Option<ViewUpdate> { ... }

// L456-469: eval_delta() - deprecated  
fn eval_delta(...) -> Option<ZSet> { ... }

// L599-602: get_row_hash() - Table.hashes removed
fn get_row_hash(...) -> Option<String> { ... }
```

### 4.2 Consolidate Duplicate Functions

**Before: Two similar recursive collectors**
```rust
fn collect_subquery_ids_recursive(&self, op, parent_row, db, out) { ... }
fn collect_nested_subquery_ids(&self, op, parent_row, db, out) { ... }
```

**After: Single unified collector**
```rust
fn collect_subquery_ids(
    &self,
    op: &Operator,
    parent_row: Option<&SpookyValue>,
    db: &Database,
    out: &mut Vec<SmolStr>,
    depth: usize,  // For debugging/limiting recursion
) {
    match op {
        Operator::Project { input, projections } => {
            self.collect_subquery_ids(input, parent_row, db, out, depth);
            
            for proj in projections {
                if let Projection::Subquery { plan, .. } = proj {
                    if let Some(parent) = parent_row {
                        let results = self.eval_snapshot(plan, db, Some(parent)).into_owned();
                        for (sub_id, _) in &results {
                            out.push(sub_id.clone());
                            if let Some(sub_row) = self.get_row_value(sub_id.as_str(), db) {
                                self.collect_subquery_ids(plan, Some(sub_row), db, out, depth + 1);
                            }
                        }
                    }
                }
            }
        }
        Operator::Filter { input, .. } |
        Operator::Limit { input, .. } => {
            self.collect_subquery_ids(input, parent_row, db, out, depth);
        }
        Operator::Join { left, right, .. } => {
            self.collect_subquery_ids(left, parent_row, db, out, depth);
            self.collect_subquery_ids(right, parent_row, db, out, depth);
        }
        Operator::Scan { .. } => {}
    }
}
```

### 4.3 Extract Helper Functions

```rust
// helpers.rs or at bottom of view.rs

/// Strip "table:" prefix from ZSet key to get row ID
#[inline]
fn strip_table_prefix(key: &str) -> &str {
    key.split_once(':').map(|(_, id)| id).unwrap_or(key)
}

/// Get row value from database using ZSet key format "table:id"
#[inline]  
fn get_row_value<'a>(key: &str, db: &'a Database) -> Option<&'a SpookyValue> {
    let (table_name, id) = key.split_once(':')?;
    db.tables.get(table_name)?.rows.get(id)
}

/// Resolve parameter value from predicate, handling $param references
fn resolve_predicate_value(
    value: &Value,
    context: Option<&SpookyValue>,
) -> Option<SpookyValue> {
    if let Some(obj) = value.as_object() {
        if let Some(param_path) = obj.get("$param") {
            let ctx = context?;
            let path_str = param_path.as_str().unwrap_or("");
            let effective_path = path_str.strip_prefix("parent.").unwrap_or(path_str);
            let path = Path::new(effective_path);
            resolve_nested_value(Some(ctx), &path)
                .cloned()
                .map(normalize_record_id)
        } else {
            Some(SpookyValue::from(value.clone()))
        }
    } else {
        Some(SpookyValue::from(value.clone()))
    }
}
```

### 4.4 Split process_ingest into Smaller Functions

```rust
impl View {
    pub fn process_batch(
        &mut self,
        deltas: &FastMap<String, ZSet>,
        db: &Database,
    ) -> Option<ViewUpdate> {
        let is_first_run = self.last_hash.is_empty();
        
        // Step 1: Compute view delta
        let view_delta = self.compute_view_delta(deltas, db, is_first_run);
        
        // Step 2: Check for updates to cached records
        let updated_ids = self.get_updated_cached_records(deltas);
        
        // Step 3: Early return if no changes
        if view_delta.is_empty() && !is_first_run && updated_ids.is_empty() {
            return None;
        }
        
        // Step 4: Apply delta to cache
        self.apply_cache_delta(&view_delta);
        
        // Step 5: Build and return update
        self.build_view_update(&view_delta, &updated_ids, is_first_run)
    }
    
    fn compute_view_delta(
        &self,
        deltas: &FastMap<String, ZSet>,
        db: &Database,
        is_first_run: bool,
    ) -> ZSet {
        if !is_first_run {
            if let Some(delta) = self.eval_delta_batch(&self.plan.root, deltas, db, self.params.as_ref()) {
                return delta;
            }
        }
        
        // Fallback: full scan + diff
        self.compute_full_diff(db)
    }
    
    fn compute_full_diff(&self, db: &Database) -> ZSet {
        let target = self.eval_snapshot(&self.plan.root, db, self.params.as_ref()).into_owned();
        let mut diff = FastMap::default();
        
        // New or changed records
        for (key, &new_w) in &target {
            let old_w = self.cache.get(key).copied().unwrap_or(0);
            if new_w != old_w {
                diff.insert(key.clone(), new_w - old_w);
            }
        }
        
        // Removed records
        for (key, &old_w) in &self.cache {
            if !target.contains_key(key) {
                diff.insert(key.clone(), -old_w);
            }
        }
        
        diff
    }
    
    fn apply_cache_delta(&mut self, delta: &ZSet) {
        for (key, &weight) in delta {
            let entry = self.cache.entry(key.clone()).or_insert(0);
            *entry += weight;
            if *entry == 0 {
                self.cache.remove(key);
            }
        }
    }
    
    fn build_view_update(
        &mut self,
        view_delta: &ZSet,
        updated_ids: &[String],
        is_first_run: bool,
    ) -> Option<ViewUpdate> {
        // Build additions/removals
        let (additions, removals) = self.categorize_delta(view_delta, updated_ids);
        
        // Build updates list
        let removal_set: HashSet<&str> = removals.iter().map(|s| s.as_str()).collect();
        let updates: Vec<String> = updated_ids
            .iter()
            .filter(|id| !removal_set.contains(id.as_str()))
            .cloned()
            .collect();
        
        // Build result data
        let mut result_data: Vec<String> = self.cache.keys().map(|k| k.to_string()).collect();
        result_data.sort_unstable();
        
        // Delegate to update module
        let raw_result = RawViewResult {
            query_id: self.plan.id.clone(),
            records: result_data,
            delta: if is_first_run { None } else {
                Some(ViewDelta { additions, removals, updates })
            },
        };
        
        let update = build_update(raw_result, self.format.clone());
        let hash = self.extract_hash(&update);
        
        if hash != self.last_hash {
            self.last_hash = hash;
            Some(update)
        } else {
            None
        }
    }
    
    fn categorize_delta(
        &self,
        view_delta: &ZSet,
        updated_ids: &[String],
    ) -> (Vec<String>, Vec<String>) {
        let updated_set: HashSet<&str> = updated_ids.iter().map(|s| s.as_str()).collect();
        
        let mut additions = Vec::with_capacity(view_delta.len());
        let mut removals = Vec::with_capacity(view_delta.len());
        
        for (key, &weight) in view_delta {
            if weight > 0 && !updated_set.contains(key.as_str()) {
                additions.push(key.to_string());
            } else if weight < 0 {
                removals.push(key.to_string());
            }
        }
        
        (additions, removals)
    }
    
    fn extract_hash(&self, update: &ViewUpdate) -> String {
        match update {
            ViewUpdate::Flat(m) | ViewUpdate::Tree(m) => m.result_hash.clone(),
            ViewUpdate::Streaming(_) => {
                let data: Vec<_> = self.cache.keys().map(|k| k.to_string()).collect();
                compute_flat_hash(&data)
            }
        }
    }
}
```

### 4.5 Optimized process_single

```rust
impl View {
    /// Optimized single-record processing
    /// Avoids HashMap creation when possible
    pub fn process_single(&mut self, delta: &Delta, db: &Database) -> Option<ViewUpdate> {
        let is_first_run = self.last_hash.is_empty();
        
        // Fast path: Check if this table is even relevant to our view
        if !self.plan.root.references_table(&delta.table) {
            return None;
        }
        
        // For simple Scan + Filter views, we can optimize
        if let Some(view_delta) = self.try_fast_single_eval(delta, db) {
            if view_delta.is_empty() && !is_first_run {
                // Check if it's an update to existing cached record
                if delta.weight > 0 && self.cache.contains_key(&delta.key) {
                    return self.build_single_update(&delta.key, db, is_first_run);
                }
                return None;
            }
            
            self.apply_cache_delta(&view_delta);
            return self.build_view_update(&view_delta, &[], is_first_run);
        }
        
        // Fallback to batch processing for complex queries
        let mut deltas = FastMap::default();
        let mut zset = ZSet::default();
        zset.insert(delta.key.clone(), delta.weight);
        deltas.insert(delta.table.to_string(), zset);
        self.process_batch(&deltas, db)
    }
    
    /// Try to evaluate single record without full batch overhead
    fn try_fast_single_eval(&self, delta: &Delta, db: &Database) -> Option<ZSet> {
        match &self.plan.root {
            Operator::Scan { table } if table == delta.table.as_str() => {
                // Direct scan - delta passes through
                let mut result = ZSet::default();
                result.insert(delta.key.clone(), delta.weight);
                Some(result)
            }
            Operator::Filter { input, predicate } => {
                if let Operator::Scan { table } = input.as_ref() {
                    if table == delta.table.as_str() {
                        // Scan + Filter - check predicate
                        let mut result = ZSet::default();
                        if self.check_predicate(predicate, &delta.key, db, self.params.as_ref()) {
                            result.insert(delta.key.clone(), delta.weight);
                        }
                        return Some(result);
                    }
                }
                None // Complex filter, fall back
            }
            _ => None // Complex query, fall back to batch
        }
    }
    
    fn build_single_update(
        &mut self,
        key: &SmolStr,
        db: &Database,
        is_first_run: bool,
    ) -> Option<ViewUpdate> {
        // This is an update to an existing record
        let mut result_data: Vec<String> = self.cache.keys().map(|k| k.to_string()).collect();
        result_data.sort_unstable();
        
        let raw_result = RawViewResult {
            query_id: self.plan.id.clone(),
            records: result_data,
            delta: if is_first_run { None } else {
                Some(ViewDelta {
                    additions: vec![],
                    removals: vec![],
                    updates: vec![key.to_string()],
                })
            },
        };
        
        let update = build_update(raw_result, self.format.clone());
        let hash = self.extract_hash(&update);
        
        if hash != self.last_hash {
            self.last_hash = hash;
            Some(update)
        } else {
            None
        }
    }
}
```

### 4.6 Add Helper Method to Operator

```rust
// In operators/operator.rs

impl Operator {
    /// Check if this operator tree references a specific table
    pub fn references_table(&self, table_name: &str) -> bool {
        match self {
            Operator::Scan { table } => table == table_name,
            Operator::Filter { input, .. } |
            Operator::Project { input, .. } |
            Operator::Limit { input, .. } => input.references_table(table_name),
            Operator::Join { left, right, .. } => {
                left.references_table(table_name) || right.references_table(table_name)
            }
        }
    }
}
```

---

## 5. Implementation Phases

### Phase 1: Cleanup (1 day)
- [ ] Remove `process()` method
- [ ] Remove `eval_delta()` method
- [ ] Remove `get_row_hash()` method
- [ ] Remove outdated comments (L70-94)
- [ ] Extract `strip_table_prefix()` helper
- [ ] Extract `resolve_predicate_value()` helper

### Phase 2: Consolidate (1-2 days)
- [ ] Merge `collect_subquery_ids_recursive` and `collect_nested_subquery_ids`
- [ ] Merge `collect_subquery_tables` and `collect_tables_from_operator`
- [ ] Add `Operator::references_table()` helper

### Phase 3: Split process_ingest (2 days)
- [ ] Extract `compute_view_delta()`
- [ ] Extract `compute_full_diff()`
- [ ] Extract `apply_cache_delta()`
- [ ] Extract `build_view_update()`
- [ ] Extract `categorize_delta()`
- [ ] Rename `process_ingest` to `process_batch`

### Phase 4: Optimize process_single (1-2 days)
- [ ] Implement `try_fast_single_eval()`
- [ ] Implement `build_single_update()`
- [ ] Add fast path for Scan and Scan+Filter views
- [ ] Benchmark improvements

### Phase 5: Testing & Documentation (1 day)
- [ ] Add unit tests for new functions
- [ ] Update documentation
- [ ] Performance benchmarks

---

## 6. Performance Optimizations

### 6.1 Reduce Allocations

| Current | Optimized | Savings |
|---------|-----------|---------|
| `key.to_string()` in loops | Use `SmolStr` or `&str` where possible | ~50% string allocs |
| `Vec::new()` without capacity | `Vec::with_capacity(n)` | ~30% reallocs |
| Clone for ViewDelta | Move ownership or use references | 3 clones eliminated |
| HashSet per call | Reuse or inline checks for small sets | ~20% for small deltas |

### 6.2 Fast Paths

```rust
// Single record, simple view (Scan or Scan+Filter)
process_single() → try_fast_single_eval() → direct evaluation
                                          ↓
                            ~10x faster than batch path

// Batch with incremental eval possible
process_batch() → eval_delta_batch() → incremental update
                                     ↓
                        ~5x faster than full scan
```

### 6.3 Expected Improvements

| Scenario | Current | After | Improvement |
|----------|---------|-------|-------------|
| Single record, Scan view | ~500ns | ~50ns | 10x |
| Single record, Filter view | ~800ns | ~100ns | 8x |
| Single record, complex view | ~2μs | ~2μs | - (same) |
| Batch 100 records | ~50μs | ~40μs | 1.25x |
| First run (full scan) | ~10ms | ~10ms | - (same) |

---

## 7. Summary

### Files to Modify
- `view.rs` - Main refactoring target
- `operators/operator.rs` - Add `references_table()` helper

### Code Reduction
- **Before:** ~700 lines
- **After:** ~500 lines (estimated)
- **Removed:** ~150 lines of dead/redundant code
- **Added:** ~50 lines of optimizations

### Key Changes
1. Remove 3 dead/deprecated functions
2. Consolidate 4 similar recursive functions into 2
3. Split 150-line `process_ingest` into 6 smaller functions
4. Add true single-record fast path in `process_single`
5. Rename `process_ingest` → `process_batch` for clarity

### API Changes
```rust
// Removed:
fn process(&mut self, changed_table, input_delta, db) -> Option<ViewUpdate>

// Renamed:
fn process_ingest(...) → fn process_batch(...)

// Optimized:
fn process_single(...) // Now has true fast path
```

---

*Document Version: 1.0*
*Last Updated: 2025-01-23*
*Status: Ready for Implementation*