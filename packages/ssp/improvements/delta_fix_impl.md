# Implementation Plan: Fix Whole-View Update Bug

## Executive Summary

**Problem**: When updating a single record, all records in the view are marked as updated
**Root Cause**: `build_result_data()` returns entire cache regardless of what changed
**Solution**: Filter result_data to only include changed records for streaming mode
**Impact**: Critical performance fix - O(N) → O(1) for single record updates

---

## Phase 1: Pre-Implementation Verification (30 min)

### 1.1 Reproduce the Bug
Create a minimal test case to confirm the issue:

```rust
#[test]
fn reproduce_whole_view_update_bug() {
    let mut circuit = Circuit::new();
    
    // Register streaming view
    let plan = QueryPlan {
        id: "users_view".to_string(),
        root: Operator::Scan { table: "users".to_string() }
    };
    circuit.register_view(plan, None, Some(ViewResultFormat::Streaming));
    
    // Load 5 records
    circuit.init_load(vec![
        LoadRecord::new("users", "user:1", json!({"name": "Alice"}).into()),
        LoadRecord::new("users", "user:2", json!({"name": "Bob"}).into()),
        LoadRecord::new("users", "user:3", json!({"name": "Carol"}).into()),
        LoadRecord::new("users", "user:4", json!({"name": "Dave"}).into()),
        LoadRecord::new("users", "user:5", json!({"name": "Eve"}).into()),
    ]);
    
    // Update ONE record
    let updates = circuit.ingest_batch(vec![
        BatchEntry::update("users", "user:3", json!({"name": "Carol Updated"}).into())
    ]);
    
    // BUG: This will fail because all 5 records are returned as "Updated"
    assert_eq!(updates.len(), 1);
    if let ViewUpdate::Streaming(s) = &updates[0] {
        println!("Bug: {} records updated instead of 1", s.records.len());
        assert_eq!(s.records.len(), 1, "Should only update 1 record, not all 5");
    }
}
```

**Expected Result**: Test fails with "5 records updated instead of 1"
**Location**: Add to `view.rs` tests section

### 1.2 Verify Impact on Other Modes
Ensure Flat/Tree modes still need all records:

```rust
#[test]
fn flat_mode_needs_all_records_for_hash() {
    let mut circuit = Circuit::new();
    
    // Register FLAT view (not streaming)
    let plan = QueryPlan {
        id: "users_view".to_string(),
        root: Operator::Scan { table: "users".to_string() }
    };
    circuit.register_view(plan, None, Some(ViewResultFormat::Flat));
    
    // Load records
    circuit.init_load(vec![
        LoadRecord::new("users", "user:1", json!({"name": "Alice"}).into()),
        LoadRecord::new("users", "user:2", json!({"name": "Bob"}).into()),
    ]);
    
    // Update one record
    let updates = circuit.ingest_batch(vec![
        BatchEntry::update("users", "user:1", json!({"name": "Alice Updated"}).into())
    ]);
    
    // Flat mode should still return full dataset for hash computation
    assert_eq!(updates.len(), 1);
    if let ViewUpdate::Flat(f) = &updates[0] {
        assert_eq!(f.records.len(), 2, "Flat mode needs all records");
    }
}
```

**Expected Result**: Confirms Flat mode needs full dataset

---

## Phase 2: Core Implementation (1-2 hours)

### 2.1 Modify `view.rs:process_batch()`

**File**: `src/engine/view.rs`
**Line**: 475 (approximately)
**Method**: `View::process_batch()`

#### Current Code:
```rust
// Build result data
let result_data = self.build_result_data();
```

#### New Code:
```rust
// Build result data
// For Streaming mode: only include changed records to avoid sending entire view
// For Flat/Tree modes: include all records for hash computation
let result_data = match self.format {
    ViewResultFormat::Streaming => {
        // Collect only records that changed
        let mut changed_keys = Vec::new();
        changed_keys.extend(additions.iter().cloned());
        changed_keys.extend(removals.iter().cloned());
        changed_keys.extend(updates.iter().cloned());
        
        tracing::debug!(
            target: "ssp::view::process_batch",
            view_id = %self.plan.id,
            changed_count = changed_keys.len(),
            "Streaming mode: filtered to changed records only"
        );
        
        changed_keys
    }
    ViewResultFormat::Flat | ViewResultFormat::Tree => {
        // Need all records for hash computation
        self.build_result_data()
    }
};
```

**Justification**:
- Streaming mode only needs delta events, not full snapshots
- Flat/Tree modes compute hashes over entire result set
- Maintains backward compatibility for non-streaming modes

### 2.2 Review Related Methods

Check if similar issues exist in single-record fast paths:

#### `apply_single_create()` (line ~311)
```rust
fn apply_single_create(&mut self, key: &SmolStr) -> Option<ViewUpdate> {
    let was_member = self.cache.is_member(key);
    self.cache.add_member(key.clone());
    
    let (additions, updates) = if was_member {
        (vec![], vec![key.clone()])  // ✅ Only includes the one key
    } else {
        (vec![key.clone()], vec![])  // ✅ Only includes the one key
    };
    
    self.build_single_update(additions, vec![], updates)
}
```
**Status**: ✅ Already correct - constructs specific vectors

#### `apply_single_delete()` (line ~327)
```rust
fn apply_single_delete(&mut self, key: &SmolStr) -> Option<ViewUpdate> {
    if !self.cache.is_member(key) {
        return None;
    }
    
    self.cache.remove_member(key);
    self.build_single_update(vec![], vec![key.clone()], vec![])  // ✅ Only includes the one key
}
```
**Status**: ✅ Already correct

#### `build_single_update()` (line ~337)
```rust
fn build_single_update(
    &mut self,
    additions: Vec<SmolStr>,
    removals: Vec<SmolStr>,
    updates: Vec<SmolStr>,
) -> Option<ViewUpdate> {
    let is_first_run = self.last_hash.is_empty();
    let result_data = self.build_result_data();  // ⚠️ Gets ALL records
    // ...
}
```
**Status**: ⚠️ **SAME BUG** - needs fixing too!

### 2.3 Fix `build_single_update()` 

**File**: `src/engine/view.rs`
**Line**: ~345
**Method**: `View::build_single_update()`

#### Current Code:
```rust
fn build_single_update(
    &mut self,
    additions: Vec<SmolStr>,
    removals: Vec<SmolStr>,
    updates: Vec<SmolStr>,
) -> Option<ViewUpdate> {
    let is_first_run = self.last_hash.is_empty();
    let result_data = self.build_result_data();
    // ...
}
```

#### New Code:
```rust
fn build_single_update(
    &mut self,
    additions: Vec<SmolStr>,
    removals: Vec<SmolStr>,
    updates: Vec<SmolStr>,
) -> Option<ViewUpdate> {
    let is_first_run = self.last_hash.is_empty();
    
    // For streaming mode, only include changed records
    // For Flat/Tree modes, need all records for hash
    let result_data = match self.format {
        ViewResultFormat::Streaming => {
            let mut changed_keys = Vec::new();
            changed_keys.extend(&additions);
            changed_keys.extend(&removals);
            changed_keys.extend(&updates);
            changed_keys
        }
        ViewResultFormat::Flat | ViewResultFormat::Tree => {
            self.build_result_data()
        }
    };
    // ... rest unchanged
}
```

---

## Phase 3: Testing (2-3 hours)

### 3.1 Unit Tests

Create comprehensive test suite in `view.rs`:

```rust
#[cfg(test)]
mod update_fix_tests {
    use super::*;
    
    #[test]
    fn test_streaming_single_update_only_sends_changed() {
        // Test that streaming mode only sends the updated record
        let mut circuit = Circuit::new();
        
        let plan = QueryPlan {
            id: "test".to_string(),
            root: Operator::Scan { table: "users".to_string() }
        };
        circuit.register_view(plan, None, Some(ViewResultFormat::Streaming));
        
        // Load 10 records
        let records: Vec<_> = (1..=10)
            .map(|i| LoadRecord::new("users", format!("user:{}", i), 
                                    json!({"name": format!("User {}", i)}).into()))
            .collect();
        circuit.init_load(records);
        
        // Update record 5
        let updates = circuit.ingest_batch(vec![
            BatchEntry::update("users", "user:5", json!({"name": "Updated User 5"}).into())
        ]);
        
        assert_eq!(updates.len(), 1);
        if let ViewUpdate::Streaming(s) = &updates[0] {
            assert_eq!(s.records.len(), 1, "Should only send 1 updated record");
            assert_eq!(s.records[0].id.as_str(), "users:5");
            assert!(matches!(s.records[0].event, DeltaEvent::Updated));
        } else {
            panic!("Expected Streaming update");
        }
    }
    
    #[test]
    fn test_flat_mode_sends_all_records() {
        // Test that Flat mode still sends all records (needed for hash)
        let mut circuit = Circuit::new();
        
        let plan = QueryPlan {
            id: "test".to_string(),
            root: Operator::Scan { table: "users".to_string() }
        };
        circuit.register_view(plan, None, Some(ViewResultFormat::Flat));
        
        // Load 5 records
        let records: Vec<_> = (1..=5)
            .map(|i| LoadRecord::new("users", format!("user:{}", i), 
                                    json!({"name": format!("User {}", i)}).into()))
            .collect();
        circuit.init_load(records);
        
        // Update one record
        let updates = circuit.ingest_batch(vec![
            BatchEntry::update("users", "user:3", json!({"name": "Updated"}).into())
        ]);
        
        assert_eq!(updates.len(), 1);
        if let ViewUpdate::Flat(f) = &updates[0] {
            assert_eq!(f.records.len(), 5, "Flat mode should send all records");
        } else {
            panic!("Expected Flat update");
        }
    }
    
    #[test]
    fn test_streaming_multiple_updates() {
        // Test updating multiple records at once
        let mut circuit = Circuit::new();
        
        let plan = QueryPlan {
            id: "test".to_string(),
            root: Operator::Scan { table: "users".to_string() }
        };
        circuit.register_view(plan, None, Some(ViewResultFormat::Streaming));
        
        // Load records
        circuit.init_load(vec![
            LoadRecord::new("users", "user:1", json!({"name": "Alice"}).into()),
            LoadRecord::new("users", "user:2", json!({"name": "Bob"}).into()),
            LoadRecord::new("users", "user:3", json!({"name": "Carol"}).into()),
        ]);
        
        // Update 2 records
        let updates = circuit.ingest_batch(vec![
            BatchEntry::update("users", "user:1", json!({"name": "Alice Updated"}).into()),
            BatchEntry::update("users", "user:3", json!({"name": "Carol Updated"}).into()),
        ]);
        
        assert_eq!(updates.len(), 1);
        if let ViewUpdate::Streaming(s) = &updates[0] {
            assert_eq!(s.records.len(), 2, "Should send 2 updated records");
        }
    }
    
    #[test]
    fn test_streaming_mixed_operations() {
        // Test mix of create, update, delete in one batch
        let mut circuit = Circuit::new();
        
        let plan = QueryPlan {
            id: "test".to_string(),
            root: Operator::Scan { table: "users".to_string() }
        };
        circuit.register_view(plan, None, Some(ViewResultFormat::Streaming));
        
        // Initial load
        circuit.init_load(vec![
            LoadRecord::new("users", "user:1", json!({"name": "Alice"}).into()),
            LoadRecord::new("users", "user:2", json!({"name": "Bob"}).into()),
        ]);
        
        // Mixed batch: create, update, delete
        let updates = circuit.ingest_batch(vec![
            BatchEntry::create("users", "user:3", json!({"name": "Carol"}).into()),
            BatchEntry::update("users", "user:2", json!({"name": "Bob Updated"}).into()),
            BatchEntry::delete("users", "user:1"),
        ]);
        
        assert_eq!(updates.len(), 1);
        if let ViewUpdate::Streaming(s) = &updates[0] {
            assert_eq!(s.records.len(), 3, "Should have 3 delta events");
            
            let created = s.records.iter().filter(|r| matches!(r.event, DeltaEvent::Created)).count();
            let updated = s.records.iter().filter(|r| matches!(r.event, DeltaEvent::Updated)).count();
            let deleted = s.records.iter().filter(|r| matches!(r.event, DeltaEvent::Deleted)).count();
            
            assert_eq!(created, 1, "One creation");
            assert_eq!(updated, 1, "One update");
            assert_eq!(deleted, 1, "One deletion");
        }
    }
    
    #[test]
    fn test_streaming_update_filtered_record() {
        // Test updating a record in a filtered view
        let mut circuit = Circuit::new();
        
        let plan = QueryPlan {
            id: "test".to_string(),
            root: Operator::Filter {
                input: Box::new(Operator::Scan { table: "users".to_string() }),
                predicate: Predicate::Eq {
                    field: Path::new("active"),
                    value: json!(true),
                },
            }
        };
        circuit.register_view(plan, None, Some(ViewResultFormat::Streaming));
        
        // Load records (only 2 are active)
        circuit.init_load(vec![
            LoadRecord::new("users", "user:1", json!({"name": "Alice", "active": true}).into()),
            LoadRecord::new("users", "user:2", json!({"name": "Bob", "active": false}).into()),
            LoadRecord::new("users", "user:3", json!({"name": "Carol", "active": true}).into()),
        ]);
        
        // Update active record
        let updates = circuit.ingest_batch(vec![
            BatchEntry::update("users", "user:1", json!({"name": "Alice Updated", "active": true}).into()),
        ]);
        
        assert_eq!(updates.len(), 1);
        if let ViewUpdate::Streaming(s) = &updates[0] {
            assert_eq!(s.records.len(), 1, "Should only send 1 update");
            assert_eq!(s.records[0].id.as_str(), "users:1");
        }
    }
    
    #[test]
    fn test_fast_path_single_update() {
        // Test the fast path for simple scans
        let plan = QueryPlan {
            id: "test".to_string(),
            root: Operator::Scan { table: "users".to_string() }
        };
        let mut view = View::new(plan, None, Some(ViewResultFormat::Streaming));
        
        // Simulate existing cache
        view.cache.insert("users:1".into(), 1);
        view.cache.insert("users:2".into(), 1);
        view.cache.insert("users:3".into(), 1);
        view.last_hash = "initial_hash".to_string();
        
        // Setup database
        let mut db = Database::new();
        let tb = db.ensure_table("users");
        tb.rows.insert("users:1".into(), json!({"name": "Alice"}).into());
        tb.rows.insert("users:2".into(), json!({"name": "Bob"}).into());
        tb.rows.insert("users:3".into(), json!({"name": "Carol"}).into());
        tb.zset.insert("users:1".into(), 1);
        tb.zset.insert("users:2".into(), 1);
        tb.zset.insert("users:3".into(), 1);
        
        // Process content update (weight=0, content_changed=true)
        let delta = Delta {
            table: "users".into(),
            key: "users:2".into(),
            weight: 0,
            content_changed: true,
        };
        
        let result = view.process_delta(&delta, &db);
        
        assert!(result.is_some());
        if let Some(ViewUpdate::Streaming(s)) = result {
            assert_eq!(s.records.len(), 1, "Fast path should only send 1 update");
            assert_eq!(s.records[0].id.as_str(), "users:2");
        }
    }
}
```

### 3.2 Integration Tests

Create tests in a separate file `tests/update_performance.rs`:

```rust
use ssp::engine::*;

#[test]
fn test_large_view_single_update_performance() {
    let mut circuit = Circuit::new();
    
    let plan = QueryPlan {
        id: "large_view".to_string(),
        root: Operator::Scan { table: "items".to_string() }
    };
    circuit.register_view(plan, None, Some(ViewResultFormat::Streaming));
    
    // Load 1000 records
    let records: Vec<_> = (1..=1000)
        .map(|i| LoadRecord::new("items", format!("item:{}", i), 
                                json!({"value": i}).into()))
        .collect();
    circuit.init_load(records);
    
    // Update ONE record
    let start = std::time::Instant::now();
    let updates = circuit.ingest_batch(vec![
        BatchEntry::update("items", "item:500", json!({"value": 9999}).into())
    ]);
    let elapsed = start.elapsed();
    
    assert_eq!(updates.len(), 1);
    if let ViewUpdate::Streaming(s) = &updates[0] {
        assert_eq!(s.records.len(), 1, 
            "Bug: Sending {} records instead of 1 (took {:?})", 
            s.records.len(), elapsed);
    }
    
    // Performance assertion: should be fast
    assert!(elapsed.as_millis() < 10, 
            "Update took too long: {:?}", elapsed);
}

#[test]
fn test_subquery_view_update() {
    // Test that subquery views also only send changed records
    let mut circuit = Circuit::new();
    
    // View with subquery (threads with their comments)
    let plan = QueryPlan {
        id: "threads_with_comments".to_string(),
        root: Operator::Project {
            input: Box::new(Operator::Scan { table: "threads".to_string() }),
            projections: vec![
                Projection::Subquery {
                    alias: "comments".to_string(),
                    op: Box::new(Operator::Filter {
                        input: Box::new(Operator::Scan { table: "comments".to_string() }),
                        predicate: Predicate::Eq {
                            field: Path::new("thread_id"),
                            value: json!({"$param": "parent.id"}),
                        },
                    }),
                }
            ],
        }
    };
    circuit.register_view(plan, None, Some(ViewResultFormat::Streaming));
    
    // Load data
    circuit.init_load(vec![
        LoadRecord::new("threads", "thread:1", json!({"id": "thread:1", "title": "Thread 1"}).into()),
        LoadRecord::new("threads", "thread:2", json!({"id": "thread:2", "title": "Thread 2"}).into()),
        LoadRecord::new("comments", "comment:1", json!({"id": "comment:1", "thread_id": "thread:1", "text": "A"}).into()),
        LoadRecord::new("comments", "comment:2", json!({"id": "comment:2", "thread_id": "thread:1", "text": "B"}).into()),
    ]);
    
    // Update thread 1's title
    let updates = circuit.ingest_batch(vec![
        BatchEntry::update("threads", "thread:1", json!({"id": "thread:1", "title": "Updated Title"}).into())
    ]);
    
    // Should only send update for thread:1 (not thread:2)
    assert_eq!(updates.len(), 1);
    if let ViewUpdate::Streaming(s) = &updates[0] {
        // Even with subqueries, should only update affected parent
        let thread_updates: Vec<_> = s.records.iter()
            .filter(|r| r.id.starts_with("threads:"))
            .collect();
        assert_eq!(thread_updates.len(), 1, "Should only update thread:1");
    }
}
```

### 3.3 Regression Tests

Ensure the fix doesn't break existing functionality:

```rust
#[test]
fn test_backward_compatibility_flat_mode() {
    // Ensure Flat mode behavior unchanged
    // ... (copy existing flat mode tests)
}

#[test]
fn test_backward_compatibility_tree_mode() {
    // Ensure Tree mode behavior unchanged
    // ... (copy existing tree mode tests)
}

#[test]
fn test_hash_computation_unchanged() {
    // Verify hash computation still works correctly
    let mut circuit = Circuit::new();
    
    let plan = QueryPlan {
        id: "test".to_string(),
        root: Operator::Scan { table: "users".to_string() }
    };
    circuit.register_view(plan.clone(), None, Some(ViewResultFormat::Flat));
    
    circuit.init_load(vec![
        LoadRecord::new("users", "user:1", json!({"name": "Alice"}).into()),
        LoadRecord::new("users", "user:2", json!({"name": "Bob"}).into()),
    ]);
    
    let updates1 = circuit.ingest_batch(vec![
        BatchEntry::update("users", "user:1", json!({"name": "Alice2"}).into())
    ]);
    
    let hash1 = if let Some(ViewUpdate::Flat(f)) = &updates1.get(0) {
        f.result_hash.clone()
    } else {
        panic!("Expected flat update");
    };
    
    // Create identical state in new circuit
    let mut circuit2 = Circuit::new();
    circuit2.register_view(plan, None, Some(ViewResultFormat::Flat));
    circuit2.init_load(vec![
        LoadRecord::new("users", "user:1", json!({"name": "Alice2"}).into()),
        LoadRecord::new("users", "user:2", json!({"name": "Bob"}).into()),
    ]);
    
    let view = &circuit2.views[0];
    let result_data = view.build_result_data();
    let hash2 = compute_flat_hash(&result_data);
    
    assert_eq!(hash1, hash2, "Hashes should match for identical state");
}
```

---

## Phase 4: Performance Validation (1 hour)

### 4.1 Benchmark Test

Create benchmark in `benches/update_performance.rs`:

```rust
use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use ssp::engine::*;

fn bench_single_update(c: &mut Criterion) {
    let mut group = c.benchmark_group("single_update");
    
    for view_size in [100, 1000, 10000].iter() {
        group.bench_with_input(
            BenchmarkId::new("streaming", view_size),
            view_size,
            |b, &size| {
                b.iter_batched(
                    || {
                        // Setup
                        let mut circuit = Circuit::new();
                        let plan = QueryPlan {
                            id: "test".to_string(),
                            root: Operator::Scan { table: "items".to_string() }
                        };
                        circuit.register_view(plan, None, Some(ViewResultFormat::Streaming));
                        
                        let records: Vec<_> = (1..=size)
                            .map(|i| LoadRecord::new("items", format!("item:{}", i), 
                                                    json!({"value": i}).into()))
                            .collect();
                        circuit.init_load(records);
                        circuit
                    },
                    |mut circuit| {
                        // Benchmark: update middle record
                        let mid = size / 2;
                        black_box(circuit.ingest_batch(vec![
                            BatchEntry::update("items", format!("item:{}", mid), 
                                             json!({"value": 9999}).into())
                        ]))
                    },
                    criterion::BatchSize::SmallInput,
                );
            },
        );
    }
    
    group.finish();
}

criterion_group!(benches, bench_single_update);
criterion_main!(benches);
```

### 4.2 Expected Performance Improvements

| View Size | Before (ms) | After (ms) | Improvement |
|-----------|-------------|------------|-------------|
| 100       | ~2ms        | ~0.1ms     | 20x faster  |
| 1,000     | ~20ms       | ~0.1ms     | 200x faster |
| 10,000    | ~200ms      | ~0.1ms     | 2000x faster|

---

## Phase 5: Documentation (30 min)

### 5.1 Code Comments

Add detailed comments to the changed sections:

```rust
// Build result data
// CRITICAL DISTINCTION:
// - Streaming mode: Only changed records needed for delta events
//   Sending all records would trigger unnecessary UI updates
// - Flat/Tree modes: All records needed for hash computation
//   Hash must be computed over complete result set for consistency
let result_data = match self.format {
    ViewResultFormat::Streaming => {
        // Performance: O(changes) instead of O(total_records)
        // For a view with 10k records, updating 1 record:
        //   Before: 10k record IDs sent
        //   After: 1 record ID sent
        let mut changed_keys = Vec::new();
        changed_keys.extend(additions.iter().cloned());
        changed_keys.extend(removals.iter().cloned());
        changed_keys.extend(updates.iter().cloned());
        
        tracing::debug!(
            target: "ssp::view::process_batch",
            view_id = %self.plan.id,
            total_in_cache = self.cache.len(),
            changed_count = changed_keys.len(),
            "Streaming mode: filtered to changed records only"
        );
        
        changed_keys
    }
    ViewResultFormat::Flat | ViewResultFormat::Tree => {
        // Need complete result set for deterministic hash
        self.build_result_data()
    }
};
```

### 5.2 Update CHANGELOG.md

```markdown
## [Unreleased]

### Fixed
- **CRITICAL**: Fixed streaming mode sending entire view on single record update
  - Before: Updating 1 record in a view with 1000 records sent 1000 updates
  - After: Only sends the 1 changed record
  - Impact: O(N) → O(1) performance for content updates
  - Affected: `View::process_batch()` and `View::build_single_update()`
  - Modes: Streaming mode only (Flat/Tree modes unchanged)
```

### 5.3 Update API Documentation

Add to `view.rs` module documentation:

```rust
//! # View Modes and Result Data
//!
//! Views support three output formats, each with different requirements:
//!
//! ## Streaming Mode
//! - Returns delta events (Created/Updated/Deleted)
//! - Only sends changed records
//! - Optimized for real-time UIs
//! - O(changes) complexity
//!
//! ## Flat Mode
//! - Returns complete result set with hash
//! - Requires all records for hash computation
//! - Used for snapshot synchronization
//! - O(total_records) complexity
//!
//! ## Tree Mode
//! - Returns hierarchical structure with hash
//! - Requires all records for hash computation
//! - Used for nested data display
//! - O(total_records) complexity
```

---

## Phase 6: Deployment Checklist (15 min)

### Pre-Merge Checklist

- [ ] All unit tests pass (`cargo test`)
- [ ] Integration tests pass
- [ ] Benchmarks show expected improvements
- [ ] No regression in Flat/Tree modes
- [ ] Code reviewed by team member
- [ ] Documentation updated
- [ ] CHANGELOG updated

### Merge Strategy

1. Create feature branch: `fix/streaming-whole-view-update`
2. Commit changes with descriptive message:
   ```
   fix: streaming mode only sends changed records on update
   
   Previously, updating a single record would send all records in the view
   as "Updated" events. This was caused by build_result_data() returning
   the entire cache regardless of what changed.
   
   Now, streaming mode filters result_data to only include records that
   actually changed (additions, removals, updates), while Flat/Tree modes
   continue to receive all records for hash computation.
   
   Performance impact:
   - 1 record update in 1000-record view: 1000x improvement
   - Scales linearly with view size
   
   Fixes #[issue-number]
   ```
3. Create PR with test results and benchmark data
4. Merge after approval

---

## Phase 7: Monitoring (Ongoing)

### Post-Deployment Monitoring

1. **Performance Metrics**
   - Track average update latency
   - Monitor batch sizes in streaming mode
   - Watch for any hash inconsistencies in Flat/Tree

2. **Logging**
   - Enable debug logging for first week:
     ```rust
     RUST_LOG=ssp::view::process_batch=debug
     ```
   - Monitor for unexpected "changed_count" values

3. **User Feedback**
   - Collect reports of UI update performance
   - Watch for any reports of missing updates

---

## Rollback Plan

If issues are discovered post-deployment:

1. **Immediate Rollback** (< 5 minutes)
   ```bash
   git revert [commit-hash]
   git push
   ```

2. **Identify Issue**
   - Check logs for errors
   - Review test failures
   - Analyze user reports

3. **Hot Fix** (if minor)
   - Apply minimal patch
   - Fast-track through testing

4. **Full Revert** (if major)
   - Revert to previous version
   - Schedule proper fix for next release

---

## Success Criteria

✅ Fix is successful if:
1. All tests pass (unit, integration, regression)
2. Benchmarks show >10x improvement for single updates
3. No increase in hash mismatches for Flat/Tree modes
4. No new bugs reported in first week
5. User-facing update latency improves measurably

---

## Timeline Summary

| Phase | Duration | Outcome |
|-------|----------|---------|
| 1. Verification | 30 min | Bug confirmed, test cases ready |
| 2. Implementation | 1-2 hours | Code changes complete |
| 3. Testing | 2-3 hours | All tests passing |
| 4. Performance | 1 hour | Benchmarks show improvements |
| 5. Documentation | 30 min | Docs updated |
| 6. Deployment | 15 min | Merged to main |
| **Total** | **5-7 hours** | Feature complete |

---

## Risk Assessment

| Risk | Severity | Mitigation |
|------|----------|------------|
| Breaks Flat/Tree modes | High | Comprehensive regression tests |
| Hash computation affected | Medium | Hash consistency tests |
| Performance regression | Low | Benchmarks before/after |
| Breaking API change | None | Internal implementation only |

---

## Future Optimizations

After this fix, consider:

1. **Zero-allocation for small deltas**
   - Use SmallVec for changed_keys (inline ≤4 items)
   
2. **Incremental hash updates**
   - Update hash without recomputing entire result
   
3. **Batch optimization**
   - Coalesce multiple updates to same record

4. **Memory pooling**
   - Reuse Vec allocations across updates