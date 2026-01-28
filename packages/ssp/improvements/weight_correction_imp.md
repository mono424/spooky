# Implementation Plan: Weight=0 for Updates

## Overview

Implementing proper IVM semantics where:
- **Create** = weight `+1` (add to set)
- **Update** = weight `0` (content change, no membership change)
- **Delete** = weight `-1` (remove from set)

---

## Table of Contents

1. [Files to Modify](#1-files-to-modify)
2. [Phase 1: Core Types](#2-phase-1-core-types)
3. [Phase 2: Circuit Changes](#3-phase-2-circuit-changes)
4. [Phase 3: View Changes](#4-phase-3-view-changes)
5. [Phase 4: Update Module Changes](#5-phase-4-update-module-changes)
6. [Migration Guide](#6-migration-guide)
7. [Testing](#7-testing)

---

## 1. Files to Modify

| File | Changes |
|------|---------|
| `types/circuit_types.rs` | Update `Operation::weight()`, enhance `Delta` |
| `circuit.rs` | Update `ingest_single`, `ingest_batch`, propagation |
| `view.rs` | Update `process_delta`, `process_ingest`, update detection |
| `update.rs` | No changes needed |

---

## 2. Phase 1: Core Types

### 2.1 Update Operation Enum

```rust
// types/circuit_types.rs

/// Operation type for record mutations
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Operation {
    Create,
    Update,
    Delete,
}

impl Operation {
    /// Parse from string representation
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_uppercase().as_str() {
            "CREATE" => Some(Operation::Create),
            "UPDATE" => Some(Operation::Update),
            "DELETE" => Some(Operation::Delete),
            _ => None,
        }
    }

    /// Get ZSet weight for membership algebra
    /// - Create: +1 (adds to set)
    /// - Update: 0 (no membership change)
    /// - Delete: -1 (removes from set)
    #[inline]
    pub fn weight(&self) -> i64 {
        match self {
            Operation::Create => 1,
            Operation::Update => 0,  // ← CHANGED from 1
            Operation::Delete => -1,
        }
    }
    
    /// Does this operation change record content?
    #[inline]
    pub fn changes_content(&self) -> bool {
        matches!(self, Operation::Create | Operation::Update)
    }
    
    /// Does this operation change set membership?
    #[inline]
    pub fn changes_membership(&self) -> bool {
        matches!(self, Operation::Create | Operation::Delete)
    }
    
    /// Is this an addition (Create or Update)?
    #[inline]
    pub fn is_additive(&self) -> bool {
        matches!(self, Operation::Create | Operation::Update)
    }
}
```

### 2.2 Enhanced Delta Struct

```rust
// types/circuit_types.rs

/// A single ZSet delta (change) with content tracking
#[derive(Debug, Clone)]
pub struct Delta {
    pub table: SmolStr,
    pub key: SmolStr,
    pub weight: i64,
    /// True if the record content was modified (Create or Update)
    pub content_changed: bool,
}

impl Delta {
    /// Create a new delta
    pub fn new(table: SmolStr, key: SmolStr, weight: i64) -> Self {
        Self {
            table,
            key,
            weight,
            content_changed: weight >= 0, // Create/Update change content
        }
    }
    
    /// Create delta from operation (preferred constructor)
    #[inline]
    pub fn from_operation(table: SmolStr, key: SmolStr, op: Operation) -> Self {
        Self {
            table,
            key,
            weight: op.weight(),
            content_changed: op.changes_content(),
        }
    }
    
    /// Create a content-only update delta (weight=0, content_changed=true)
    #[inline]
    pub fn content_update(table: SmolStr, key: SmolStr) -> Self {
        Self {
            table,
            key,
            weight: 0,
            content_changed: true,
        }
    }
}
```

### 2.3 New Type: BatchDeltas

```rust
// types/circuit_types.rs (or circuit.rs)

/// Batch deltas with separate tracking for content updates
#[derive(Debug, Clone, Default)]
pub struct BatchDeltas {
    /// ZSet membership deltas (weight != 0)
    pub membership: FastMap<String, ZSet>,
    
    /// Keys with content changes (including weight=0 updates)
    /// Map: table -> list of updated keys
    pub content_updates: FastMap<String, Vec<SmolStr>>,
}

impl BatchDeltas {
    pub fn new() -> Self {
        Self::default()
    }
    
    /// Add a delta from an operation
    pub fn add(&mut self, table: &str, key: SmolStr, op: Operation) {
        let weight = op.weight();
        
        // Track membership changes (weight != 0)
        if weight != 0 {
            let zset = self.membership.entry(table.to_string()).or_default();
            *zset.entry(key.clone()).or_insert(0) += weight;
        }
        
        // Track content changes
        if op.changes_content() {
            self.content_updates
                .entry(table.to_string())
                .or_default()
                .push(key);
        }
    }
    
    /// Check if there are any changes
    pub fn is_empty(&self) -> bool {
        self.membership.is_empty() && self.content_updates.is_empty()
    }
    
    /// Get all tables that have changes
    pub fn changed_tables(&self) -> Vec<String> {
        let mut tables: Vec<String> = self.membership.keys().cloned().collect();
        for table in self.content_updates.keys() {
            if !tables.contains(table) {
                tables.push(table.clone());
            }
        }
        tables
    }
}
```

---

## 3. Phase 2: Circuit Changes

### 3.1 Update ingest_single

```rust
// circuit.rs

impl Circuit {
    pub fn ingest_single(
        &mut self,
        table: &str,
        op: Operation,
        id: &str,
        data: SpookyValue,
    ) -> Vec<ViewUpdate> {
        let key = SmolStr::new(id);
        
        // Apply mutation to storage
        let zset_key = {
            let tb = self.db.ensure_table(table);
            
            match op {
                Operation::Create | Operation::Update => {
                    tb.rows.insert(key.clone(), data);
                }
                Operation::Delete => {
                    tb.rows.remove(&key);
                }
            }
            
            // Update ZSet only for membership changes
            let zset_key = build_zset_key(table, &key);
            let weight = op.weight();
            
            if weight != 0 {
                let entry = tb.zset.entry(zset_key.clone()).or_insert(0);
                *entry += weight;
                if *entry == 0 {
                    tb.zset.remove(&zset_key);
                }
            }
            
            zset_key
        };

        self.ensure_dependency_graph();

        let table_key = SmolStr::new(table);
        let view_indices: SmallVec<[ViewIndex; 4]> = self
            .dependency_graph
            .get(&table_key)
            .map(|v| v.iter().copied().collect())
            .unwrap_or_default();

        if view_indices.is_empty() {
            return Vec::new();
        }

        // Create delta with content tracking
        let delta = Delta::from_operation(table_key, zset_key, op);

        // Process all affected views
        let mut updates: Vec<ViewUpdate> = Vec::with_capacity(view_indices.len());
        
        for view_idx in view_indices {
            if let Some(view) = self.views.get_mut(view_idx) {
                if let Some(update) = view.process_delta(&delta, &self.db) {
                    updates.push(update);
                }
            }
        }

        updates
    }
}
```

### 3.2 Update ingest_batch

```rust
// circuit.rs

impl Circuit {
    pub fn ingest_batch(&mut self, entries: Vec<BatchEntry>) -> Vec<ViewUpdate> {
        if entries.is_empty() {
            return Vec::new();
        }

        // Phase 1: Group by table and apply mutations
        let mut batch_deltas = BatchDeltas::new();
        let mut tables_modified: Vec<SmolStr> = Vec::new();

        // Group entries by table
        let mut by_table: FastMap<SmolStr, Vec<BatchEntry>> = FastMap::default();
        for entry in entries {
            by_table.entry(entry.table.clone()).or_default().push(entry);
        }

        // Process each table
        for (table_name, table_entries) in by_table {
            let tb = self.db.ensure_table(table_name.as_str());
            
            for entry in table_entries {
                // Apply to storage
                match entry.op {
                    Operation::Create | Operation::Update => {
                        tb.rows.insert(entry.id.clone(), entry.data);
                    }
                    Operation::Delete => {
                        tb.rows.remove(&entry.id);
                    }
                }
                
                // Track in batch deltas
                let zset_key = build_zset_key(table_name.as_str(), &entry.id);
                batch_deltas.add(table_name.as_str(), zset_key, entry.op);
            }
            
            tables_modified.push(table_name);
        }

        // Apply membership deltas to ZSets
        for (table_name, delta) in &batch_deltas.membership {
            if let Some(tb) = self.db.tables.get_mut(table_name) {
                for (key, &weight) in delta {
                    if weight != 0 {
                        let entry = tb.zset.entry(key.clone()).or_insert(0);
                        *entry += weight;
                        if *entry == 0 {
                            tb.zset.remove(key);
                        }
                    }
                }
            }
        }

        // Phase 2: Propagate to views
        self.propagate_batch_deltas(&batch_deltas, &tables_modified)
    }

    fn propagate_batch_deltas(
        &mut self,
        batch_deltas: &BatchDeltas,
        changed_tables: &[SmolStr],
    ) -> Vec<ViewUpdate> {
        self.ensure_dependency_graph();

        // Find affected views
        let mut affected_indices: Vec<ViewIndex> = Vec::new();
        for table in changed_tables {
            if let Some(indices) = self.dependency_graph.get(table.as_str()) {
                affected_indices.extend(indices.iter().copied());
            }
        }

        affected_indices.sort_unstable();
        affected_indices.dedup();

        if affected_indices.is_empty() {
            return Vec::new();
        }

        // Process views
        let mut updates = Vec::with_capacity(affected_indices.len());
        let db_ref = &self.db;

        for i in affected_indices {
            if let Some(view) = self.views.get_mut(i) {
                if let Some(update) = view.process_batch(batch_deltas, db_ref) {
                    updates.push(update);
                }
            }
        }

        updates
    }
}
```

### 3.3 Update Table::apply_mutation

```rust
// circuit.rs

impl Table {
    /// Apply a mutation and return the ZSet key
    /// Only modifies ZSet for membership changes (weight != 0)
    pub fn apply_mutation(
        &mut self, 
        op: Operation, 
        key: SmolStr, 
        data: SpookyValue
    ) -> SmolStr {
        // Update storage
        match op {
            Operation::Create | Operation::Update => {
                self.rows.insert(key.clone(), data);
            }
            Operation::Delete => {
                self.rows.remove(&key);
            }
        }

        // Build ZSet key
        let zset_key = build_zset_key(&self.name, &key);
        
        // Only update ZSet for membership changes
        let weight = op.weight();
        if weight != 0 {
            let entry = self.zset.entry(zset_key.clone()).or_insert(0);
            *entry += weight;
            if *entry == 0 {
                self.zset.remove(&zset_key);
            }
        }
        
        zset_key
    }
}
```

---

## 4. Phase 3: View Changes

### 4.1 Update process_delta (Single Record)

```rust
// view.rs

impl View {
    /// Process a single delta for this view
    pub fn process_delta(&mut self, delta: &Delta, db: &Database) -> Option<ViewUpdate> {
        // Case 1: Membership change (Create or Delete)
        if delta.weight != 0 {
            return self.process_membership_change(delta, db);
        }
        
        // Case 2: Content-only update (weight=0, content_changed=true)
        if delta.content_changed {
            return self.process_content_update(delta, db);
        }
        
        // Case 3: No change
        None
    }
    
    /// Handle Create/Delete (membership changes)
    fn process_membership_change(&mut self, delta: &Delta, db: &Database) -> Option<ViewUpdate> {
        // Try fast path for simple views
        if let Some(result) = self.try_fast_membership_change(delta, db) {
            return result;
        }
        
        // Fallback to batch processing
        let mut batch = BatchDeltas::new();
        batch.membership
            .entry(delta.table.to_string())
            .or_default()
            .insert(delta.key.clone(), delta.weight);
        
        if delta.content_changed {
            batch.content_updates
                .entry(delta.table.to_string())
                .or_default()
                .push(delta.key.clone());
        }
        
        self.process_batch(&batch, db)
    }
    
    /// Handle Update (content-only, no membership change)
    fn process_content_update(&mut self, delta: &Delta, db: &Database) -> Option<ViewUpdate> {
        // Check if this record is in our view
        if !self.cache.contains_key(&delta.key) {
            return None; // Not in view, no update needed
        }
        
        // Check if record still matches filter (might need to leave view)
        let still_matches = self.record_matches_view(&delta.key, db);
        
        if still_matches {
            // Content update, still in view
            self.build_content_update_notification(&delta.key, db)
        } else {
            // No longer matches filter - treat as removal
            let removal_delta = Delta {
                table: delta.table.clone(),
                key: delta.key.clone(),
                weight: -1,
                content_changed: false,
            };
            self.process_membership_change(&removal_delta, db)
        }
    }
    
    /// Check if a record matches this view's filters
    fn record_matches_view(&self, key: &SmolStr, db: &Database) -> bool {
        match &self.plan.root {
            Operator::Scan { table } => {
                // Simple scan - just check table exists
                key.starts_with(&format!("{}:", table))
            }
            Operator::Filter { input, predicate } => {
                if let Operator::Scan { table } = input.as_ref() {
                    if !key.starts_with(&format!("{}:", table)) {
                        return false;
                    }
                    return self.check_predicate(predicate, key, db, self.params.as_ref());
                }
                // Complex query - assume it matches (will be corrected by full diff)
                true
            }
            _ => true // Complex queries handled by full diff
        }
    }
    
    /// Build notification for content-only update
    fn build_content_update_notification(
        &mut self, 
        key: &SmolStr, 
        _db: &Database
    ) -> Option<ViewUpdate> {
        use super::update::{build_update, compute_flat_hash, RawViewResult, ViewDelta};
        
        let result_data = self.build_result_data();
        
        let view_delta = ViewDelta {
            additions: vec![],
            removals: vec![],
            updates: vec![Self::strip_table_prefix(key)],
        };
        
        let raw_result = RawViewResult {
            query_id: self.plan.id.clone(),
            records: result_data.clone(),
            delta: Some(view_delta),
        };
        
        let update = build_update(raw_result, self.format.clone());
        
        // For streaming, always emit. For flat/tree, check hash.
        match &update {
            ViewUpdate::Streaming(s) if !s.records.is_empty() => Some(update),
            ViewUpdate::Flat(_) | ViewUpdate::Tree(_) => {
                // Content changed but set didn't - still notify
                // Hash might be same if we're only tracking IDs, not content
                Some(update)
            }
            _ => None,
        }
    }
}
```

### 4.2 Update process_batch (renamed from process_ingest)

```rust
// view.rs

impl View {
    /// Process batch of deltas for this view
    pub fn process_batch(
        &mut self,
        batch: &BatchDeltas,
        db: &Database,
    ) -> Option<ViewUpdate> {
        let is_first_run = self.last_hash.is_empty();

        // Compute view delta from membership changes
        let view_delta = self.compute_view_delta(&batch.membership, db, is_first_run);
        
        // Find content updates for records in our cache
        let content_updates = self.get_content_updates_in_view(batch);
        
        // Early return if no changes
        if view_delta.is_empty() && !is_first_run && content_updates.is_empty() {
            return None;
        }

        // Apply membership delta to cache
        self.apply_cache_delta(&view_delta);

        // Categorize changes
        let (additions, removals, updates) = 
            self.categorize_changes_with_content(&view_delta, &content_updates);

        // Build and return update
        self.build_view_update_result(additions, removals, updates, is_first_run)
    }
    
    /// Get content updates that affect records in this view
    fn get_content_updates_in_view(&self, batch: &BatchDeltas) -> Vec<String> {
        let mut updates = Vec::new();
        
        for (_table, keys) in &batch.content_updates {
            for key in keys {
                if self.cache.contains_key(key.as_str()) {
                    updates.push(key.to_string());
                }
            }
        }
        
        updates
    }
    
    /// Categorize changes including content updates
    fn categorize_changes_with_content(
        &self,
        view_delta: &ZSet,
        content_updates: &[String],
    ) -> (Vec<String>, Vec<String>, Vec<String>) {
        let mut additions = Vec::with_capacity(view_delta.len());
        let mut removals = Vec::with_capacity(view_delta.len());
        
        // Build set of content updates for O(1) lookup
        let content_update_set: std::collections::HashSet<&str> = 
            content_updates.iter().map(|s| s.as_str()).collect();
        
        // Process membership changes
        for (key, &weight) in view_delta {
            if weight > 0 {
                // New record entering view
                // Don't count as addition if it's actually an update
                if !content_update_set.contains(key.as_str()) {
                    additions.push(Self::strip_table_prefix(key));
                }
            } else if weight < 0 {
                removals.push(Self::strip_table_prefix(key));
            }
        }
        
        // Build removal set for filtering updates
        let removal_set: std::collections::HashSet<&str> = 
            view_delta.iter()
                .filter(|(_, w)| **w < 0)
                .map(|(k, _)| k.as_str())
                .collect();
        
        // Updates: content changes that aren't removals
        let updates: Vec<String> = content_updates
            .iter()
            .filter(|key| !removal_set.contains(key.as_str()))
            .map(|key| Self::strip_table_prefix(key))
            .collect();
        
        (additions, removals, updates)
    }
    
    /// Build the final ViewUpdate result
    fn build_view_update_result(
        &mut self,
        additions: Vec<String>,
        removals: Vec<String>,
        updates: Vec<String>,
        is_first_run: bool,
    ) -> Option<ViewUpdate> {
        use super::update::{build_update, compute_flat_hash, RawViewResult, ViewDelta};
        
        let result_data = self.build_result_data();
        
        let view_delta_struct = if is_first_run {
            None
        } else {
            Some(ViewDelta {
                additions,
                removals,
                updates,
            })
        };
        
        let raw_result = RawViewResult {
            query_id: self.plan.id.clone(),
            records: result_data.clone(),
            delta: view_delta_struct,
        };
        
        let update = build_update(raw_result, self.format.clone());
        
        let hash = match &update {
            ViewUpdate::Flat(flat) | ViewUpdate::Tree(flat) => flat.result_hash.clone(),
            ViewUpdate::Streaming(_) => compute_flat_hash(&result_data),
        };
        
        let has_changes = match &update {
            ViewUpdate::Streaming(s) => !s.records.is_empty(),
            _ => hash != self.last_hash,
        };
        
        if has_changes {
            self.last_hash = hash;
            Some(update)
        } else {
            None
        }
    }
}
```

### 4.3 Remove Old get_updated_cached_records

The old method is replaced by `get_content_updates_in_view` which uses the new `BatchDeltas.content_updates` field.

---

## 5. Phase 4: Update Module Changes

No changes needed to `update.rs` - the `ViewDelta` struct already supports additions, removals, and updates.

---

## 6. Migration Guide

### API Changes

```rust
// No external API changes!
// All changes are internal implementation details

// These still work exactly the same:
circuit.ingest_single("users", Operation::Create, "u1", data);
circuit.ingest_single("users", Operation::Update, "u1", data);
circuit.ingest_single("users", Operation::Delete, "u1");

circuit.ingest_batch(vec![
    BatchEntry::create("users", "u1", data),
    BatchEntry::update("users", "u2", data),
    BatchEntry::delete("users", "u3"),
]);
```

### Behavioral Changes

| Scenario | Before | After |
|----------|--------|-------|
| Create u1, Update u1, Update u1 | cache=3 ❌ | cache=1 ✓ |
| Create u1, Update u1, Delete u1 | cache=1 (not removed) ❌ | cache=0 (removed) ✓ |
| Update non-existent record | cache=1 ❌ | cache=0 (ignored) ✓ |

---

## 7. Testing

### 7.1 Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_operation_weights() {
        assert_eq!(Operation::Create.weight(), 1);
        assert_eq!(Operation::Update.weight(), 0);
        assert_eq!(Operation::Delete.weight(), -1);
    }

    #[test]
    fn test_operation_content_change() {
        assert!(Operation::Create.changes_content());
        assert!(Operation::Update.changes_content());
        assert!(!Operation::Delete.changes_content());
    }

    #[test]
    fn test_operation_membership_change() {
        assert!(Operation::Create.changes_membership());
        assert!(!Operation::Update.changes_membership());
        assert!(Operation::Delete.changes_membership());
    }

    #[test]
    fn test_delta_from_operation() {
        let delta = Delta::from_operation(
            SmolStr::new("users"),
            SmolStr::new("users:u1"),
            Operation::Update,
        );
        assert_eq!(delta.weight, 0);
        assert!(delta.content_changed);
    }

    #[test]
    fn test_batch_deltas_tracking() {
        let mut batch = BatchDeltas::new();
        
        batch.add("users", SmolStr::new("users:u1"), Operation::Create);
        batch.add("users", SmolStr::new("users:u2"), Operation::Update);
        batch.add("users", SmolStr::new("users:u3"), Operation::Delete);
        
        // Membership should have u1 (+1) and u3 (-1), but not u2 (0)
        let users_membership = batch.membership.get("users").unwrap();
        assert_eq!(users_membership.get("users:u1"), Some(&1));
        assert_eq!(users_membership.get("users:u2"), None); // weight=0, not tracked
        assert_eq!(users_membership.get("users:u3"), Some(&-1));
        
        // Content updates should have u1 and u2, but not u3
        let users_content = batch.content_updates.get("users").unwrap();
        assert!(users_content.contains(&SmolStr::new("users:u1")));
        assert!(users_content.contains(&SmolStr::new("users:u2")));
        assert!(!users_content.contains(&SmolStr::new("users:u3")));
    }

    #[test]
    fn test_cache_correctness_multiple_updates() {
        let mut circuit = Circuit::new();
        
        // Create
        circuit.ingest_single("users", Operation::Create, "u1", data());
        assert_eq!(get_cache_weight(&circuit, "users:u1"), 1);
        
        // Update (should NOT change weight)
        circuit.ingest_single("users", Operation::Update, "u1", data());
        assert_eq!(get_cache_weight(&circuit, "users:u1"), 1);
        
        // Another update
        circuit.ingest_single("users", Operation::Update, "u1", data());
        assert_eq!(get_cache_weight(&circuit, "users:u1"), 1);
        
        // Delete (should remove)
        circuit.ingest_single("users", Operation::Delete, "u1", data());
        assert_eq!(get_cache_weight(&circuit, "users:u1"), 0);
    }

    #[test]
    fn test_filter_transition_on_update() {
        let mut circuit = Circuit::new();
        
        // Register view: active users only
        let plan = QueryPlan {
            id: "active_users".into(),
            root: Operator::Filter {
                input: Box::new(Operator::Scan { table: "users".into() }),
                predicate: Predicate::Eq {
                    field: Path::new("status"),
                    value: json!("active"),
                },
            },
        };
        circuit.register_view(plan, None, None);
        
        // Create active user
        circuit.ingest_single("users", Operation::Create, "u1", json!({"status": "active"}));
        assert!(view_contains(&circuit, "active_users", "u1"));
        
        // Update to inactive - should leave view
        circuit.ingest_single("users", Operation::Update, "u1", json!({"status": "inactive"}));
        assert!(!view_contains(&circuit, "active_users", "u1"));
    }
}
```

### 7.2 Integration Tests

```rust
#[test]
fn test_full_workflow_with_updates() {
    let mut circuit = Circuit::new();
    
    // Setup view
    circuit.register_view(users_list_plan(), None, None);
    
    // Batch with mixed operations
    let updates = circuit.ingest_batch(vec![
        BatchEntry::create("users", "u1", json!({"name": "Alice"})),
        BatchEntry::create("users", "u2", json!({"name": "Bob"})),
        BatchEntry::update("users", "u1", json!({"name": "Alice Updated"})),
        BatchEntry::delete("users", "u2"),
    ]);
    
    // Should have one view update
    assert_eq!(updates.len(), 1);
    
    // View should contain only u1
    let update = &updates[0];
    match update {
        ViewUpdate::Flat(m) => {
            assert_eq!(m.result_data.len(), 1);
            assert!(m.result_data.contains(&"u1".to_string()));
        }
        _ => panic!("Expected Flat update"),
    }
}
```

---

## 8. Implementation Checklist

### Phase 1: Core Types (Day 1)
- [ ] Update `Operation::weight()` to return 0 for Update
- [ ] Add `Operation::changes_content()` method
- [ ] Add `Operation::changes_membership()` method
- [ ] Add `content_changed` field to `Delta`
- [ ] Add `Delta::from_operation()` constructor
- [ ] Create `BatchDeltas` struct
- [ ] Add unit tests for new types

### Phase 2: Circuit Changes (Day 2)
- [ ] Update `ingest_single` to use `Delta::from_operation`
- [ ] Update `ingest_batch` to use `BatchDeltas`
- [ ] Update `Table::apply_mutation` for weight=0
- [ ] Update `propagate_batch_deltas`
- [ ] Add integration tests

### Phase 3: View Changes (Day 2-3)
- [ ] Update `process_delta` to handle weight=0
- [ ] Add `process_membership_change` method
- [ ] Add `process_content_update` method
- [ ] Add `record_matches_view` method
- [ ] Rename `process_ingest` → `process_batch`
- [ ] Update `process_batch` to use `BatchDeltas`
- [ ] Add `get_content_updates_in_view` method
- [ ] Update `categorize_changes_with_content`
- [ ] Remove old `get_updated_cached_records`
- [ ] Add view tests

### Phase 4: Final Testing (Day 4)
- [ ] Run all existing tests
- [ ] Add cache correctness tests
- [ ] Add filter transition tests
- [ ] Performance benchmarks
- [ ] Documentation update

---

## 9. Expected Benefits

| Benefit | Impact |
|---------|--------|
| Correct cache weights | No more corrupted state after multiple updates |
| Proper delete behavior | Records always removed when deleted |
| Filter transitions work | Updates that change filter eligibility handled correctly |
| True IVM semantics | Standard differential dataflow behavior |
| Better debugging | Weights always make sense (0 or 1) |

---

*Document Version: 1.0*
*Estimated Time: 3-4 days*
*Status: Ready for Implementation*