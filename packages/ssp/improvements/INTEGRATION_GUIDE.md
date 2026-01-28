# Integration Guide: DBSP Module Improvements

## Overview

The `improved_circuit.rs` and `improved_view_process.rs` files are **NOT drop-in replacements**. They are **reference implementations** showing improvement ideas. This guide explains exactly what to add to your existing files.

---

## Quick Reference

| File | What It Shows | How to Use |
|------|---------------|------------|
| `improved_circuit.rs` | Operation enum, BatchEntry, IngestBatch builder, group-by-table | **Add** new types and methods to existing `circuit.rs` |
| `improved_view_process.rs` | ProcessContext, DeltaCategories, split functions | **Refactor** existing `process_ingest()` incrementally |
| `improved_changes_test.rs` | Test cases for new functionality | **Add** as new test file `tests/improved_changes_test.rs` |

---

## Part 1: Changes to circuit.rs

### Step 1.1: Add Operation Enum (after imports)

```rust
// Add after line ~10 in circuit.rs

/// Type-safe operation enum (replaces string matching)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Operation {
    Create,
    Update,
    Delete,
}

impl Operation {
    /// Parse from string (case-insensitive)
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_ascii_uppercase().as_str() {
            "CREATE" => Some(Operation::Create),
            "UPDATE" => Some(Operation::Update),
            "DELETE" => Some(Operation::Delete),
            _ => None,
        }
    }

    /// Get the ZSet weight for this operation
    #[inline]
    pub fn weight(&self) -> i64 {
        match self {
            Operation::Create | Operation::Update => 1,
            Operation::Delete => -1,
        }
    }

    /// Check if this operation adds/updates data
    #[inline]
    pub fn is_additive(&self) -> bool {
        matches!(self, Operation::Create | Operation::Update)
    }
}
```

### Step 1.2: Add BatchEntry Struct (after Operation)

```rust
/// Type-safe batch entry
#[derive(Clone, Debug)]
pub struct BatchEntry {
    pub table: SmolStr,
    pub op: Operation,
    pub id: SmolStr,
    pub record: SpookyValue,
    pub hash: String,
}

impl BatchEntry {
    pub fn new(
        table: impl Into<SmolStr>,
        op: Operation,
        id: impl Into<SmolStr>,
        record: SpookyValue,
        hash: String,
    ) -> Self {
        Self {
            table: table.into(),
            op,
            id: id.into(),
            record,
            hash,
        }
    }

    /// Create from legacy tuple format
    pub fn from_tuple(tuple: (String, String, String, Value, String)) -> Option<Self> {
        let (table, op_str, id, record, hash) = tuple;
        let op = Operation::from_str(&op_str)?;
        Some(Self {
            table: SmolStr::from(table),
            op,
            id: SmolStr::from(id),
            record: SpookyValue::from(record),
            hash,
        })
    }
}
```

### Step 1.3: Add IngestBatch Builder (after BatchEntry)

```rust
/// Builder for ergonomic batch creation
pub struct IngestBatch {
    entries: Vec<BatchEntry>,
}

impl IngestBatch {
    pub fn new() -> Self {
        Self { entries: Vec::new() }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self { entries: Vec::with_capacity(capacity) }
    }

    /// Add a CREATE operation
    pub fn create(mut self, table: &str, id: &str, record: SpookyValue, hash: String) -> Self {
        self.entries.push(BatchEntry {
            table: SmolStr::new(table),
            op: Operation::Create,
            id: SmolStr::new(id),
            record,
            hash,
        });
        self
    }

    /// Add an UPDATE operation
    pub fn update(mut self, table: &str, id: &str, record: SpookyValue, hash: String) -> Self {
        self.entries.push(BatchEntry {
            table: SmolStr::new(table),
            op: Operation::Update,
            id: SmolStr::new(id),
            record,
            hash,
        });
        self
    }

    /// Add a DELETE operation
    pub fn delete(mut self, table: &str, id: &str) -> Self {
        self.entries.push(BatchEntry {
            table: SmolStr::new(table),
            op: Operation::Delete,
            id: SmolStr::new(id),
            record: SpookyValue::Null,
            hash: String::new(),
        });
        self
    }

    /// Add a raw entry
    pub fn entry(mut self, entry: BatchEntry) -> Self {
        self.entries.push(entry);
        self
    }

    /// Build into entries vec
    pub fn build(self) -> Vec<BatchEntry> {
        self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for IngestBatch {
    fn default() -> Self {
        Self::new()
    }
}
```

### Step 1.4: Add New Methods to Circuit impl

Add these methods inside the existing `impl Circuit { ... }` block:

```rust
impl Circuit {
    // ... keep all existing methods ...

    // ========================================================================
    // NEW: Ergonomic batch ingestion API
    // ========================================================================

    /// Ingest using the builder pattern
    pub fn ingest(&mut self, batch: IngestBatch, is_optimistic: bool) -> Vec<ViewUpdate> {
        self.ingest_entries(batch.build(), is_optimistic)
    }

    /// Ingest pre-built entries with group-by-table optimization
    pub fn ingest_entries(&mut self, entries: Vec<BatchEntry>, is_optimistic: bool) -> Vec<ViewUpdate> {
        if entries.is_empty() {
            return Vec::new();
        }

        // Group entries by table for cache-friendly processing
        let mut by_table: FastMap<SmolStr, Vec<BatchEntry>> = FastMap::default();
        for entry in entries {
            by_table.entry(entry.table.clone()).or_default().push(entry);
        }

        let mut table_deltas: FastMap<String, ZSet> = FastMap::default();

        // Process each table's entries together (better cache locality)
        for (table, table_entries) in by_table {
            let tb = self.db.ensure_table(table.as_str());
            let delta = table_deltas.entry(table.to_string()).or_default();

            for entry in table_entries {
                let weight = entry.op.weight();

                if entry.op.is_additive() {
                    tb.update_row(entry.id.clone(), entry.record, entry.hash);
                } else {
                    tb.delete_row(&entry.id);
                }

                *delta.entry(entry.id).or_insert(0) += weight;
            }
        }

        // Use existing propagation logic
        self.propagate_deltas(table_deltas, is_optimistic)
    }

    /// Internal: Propagate deltas to views (extracted for reuse)
    fn propagate_deltas(
        &mut self,
        mut table_deltas: FastMap<String, ZSet>,
        is_optimistic: bool,
    ) -> Vec<ViewUpdate> {
        // Apply deltas to DB ZSets and collect changed tables
        let mut changed_tables: Vec<String> = Vec::with_capacity(table_deltas.len());

        for (table, delta) in &mut table_deltas {
            delta.retain(|_, w| *w != 0);
            if !delta.is_empty() {
                let tb = self.db.ensure_table(table.as_str());
                tb.apply_delta(delta);
                changed_tables.push(table.clone());
            }
        }

        if changed_tables.is_empty() {
            return Vec::new();
        }

        // Lazy rebuild of dependency graph
        if self.dependency_graph.is_empty() && !self.views.is_empty() {
            self.rebuild_dependency_graph();
        }

        // Collect impacted view indices
        let mut impacted_view_indices: Vec<usize> = Vec::new();
        for table in &changed_tables {
            if let Some(indices) = self.dependency_graph.get(table) {
                impacted_view_indices.extend(indices.iter().copied());
            }
        }

        // Deduplicate
        impacted_view_indices.sort_unstable();
        impacted_view_indices.dedup();

        if impacted_view_indices.is_empty() {
            return Vec::new();
        }

        // Process views (reuse existing logic)
        let db_ref = &self.db;
        let deltas_ref = &table_deltas;

        #[cfg(all(feature = "parallel", not(target_arch = "wasm32")))]
        let updates: Vec<ViewUpdate> = {
            use rayon::prelude::*;
            self.views
                .par_iter_mut()
                .enumerate()
                .filter_map(|(i, view)| {
                    if impacted_view_indices.binary_search(&i).is_ok() {
                        view.process_ingest(deltas_ref, db_ref, is_optimistic)
                    } else {
                        None
                    }
                })
                .collect()
        };

        #[cfg(any(target_arch = "wasm32", not(feature = "parallel")))]
        let updates: Vec<ViewUpdate> = {
            let mut ups = Vec::new();
            for i in impacted_view_indices {
                if i < self.views.len() {
                    if let Some(update) = self.views[i].process_ingest(deltas_ref, db_ref, is_optimistic) {
                        ups.push(update);
                    }
                }
            }
            ups
        };

        updates
    }
}
```

### Step 1.5: Update Existing ingest_batch_spooky (Optional Optimization)

You can **optionally** update the existing method to use the new types internally:

```rust
// Replace the existing ingest_batch_spooky method body with:
pub fn ingest_batch_spooky(
    &mut self,
    batch: Vec<(SmolStr, SmolStr, SmolStr, SpookyValue, String)>,
    is_optimistic: bool,
) -> Vec<ViewUpdate> {
    // Convert to BatchEntry format
    let entries: Vec<BatchEntry> = batch
        .into_iter()
        .filter_map(|(table, op, id, record, hash)| {
            let op = Operation::from_str(op.as_str())?;
            Some(BatchEntry { table, op, id, record, hash })
        })
        .collect();

    self.ingest_entries(entries, is_optimistic)
}
```

---

## Part 2: Changes to view.rs (Incremental Refactoring)

These changes are **optional** and should be done **incrementally** with tests.

### Step 2.1: Add ProcessContext Struct (after View struct)

```rust
/// Context for view processing - computed once, used throughout
struct ProcessContext {
    is_first_run: bool,
    is_streaming: bool,
    has_subquery_changes: bool,
}

impl ProcessContext {
    fn new(view: &View, deltas: &FastMap<String, ZSet>, db: &Database) -> Self {
        let is_first_run = view.last_hash.is_empty();
        Self {
            is_first_run,
            is_streaming: matches!(view.format, ViewResultFormat::Streaming),
            has_subquery_changes: !is_first_run && view.has_changes_for_subqueries(deltas, db),
        }
    }

    fn should_full_scan(&self) -> bool {
        self.is_first_run || self.has_subquery_changes
    }
}
```

### Step 2.2: Add DeltaCategories Struct

```rust
/// Categorized changes from view delta
struct DeltaCategories {
    additions: Vec<String>,
    removals: Vec<String>,
    updates: Vec<String>,
}

impl DeltaCategories {
    fn with_capacity(cap: usize) -> Self {
        Self {
            additions: Vec::with_capacity(cap),
            removals: Vec::with_capacity(cap),
            updates: Vec::with_capacity(cap),
        }
    }

    fn is_empty(&self) -> bool {
        self.additions.is_empty() && self.removals.is_empty() && self.updates.is_empty()
    }
}
```

### Step 2.3: Refactor process_ingest (Use ProcessContext)

Update the beginning of `process_ingest()` to use ProcessContext:

```rust
pub fn process_ingest(
    &mut self,
    deltas: &FastMap<String, ZSet>,
    db: &Database,
    is_optimistic: bool,
) -> Option<ViewUpdate> {
    // NEW: Use ProcessContext instead of separate variables
    let ctx = ProcessContext::new(self, deltas, db);

    debug_log!(
        "DEBUG VIEW: id={} is_first_run={} has_subquery_changes={} is_streaming={}",
        self.plan.id,
        ctx.is_first_run,
        ctx.has_subquery_changes,
        ctx.is_streaming
    );

    let maybe_delta = if ctx.should_full_scan() {
        None
    } else {
        self.eval_delta_batch(&self.plan.root, deltas, db, self.params.as_ref())
    };

    // ... rest of existing code, replacing:
    //   is_first_run -> ctx.is_first_run
    //   is_streaming -> ctx.is_streaming
    //   has_subquery_changes -> ctx.has_subquery_changes
}
```

---

## Part 3: Add Test File

Save `improved_changes_test.rs` as `tests/improved_changes_test.rs` in your project.

Make sure it has access to the common module by checking the import:
```rust
mod common;
use common::*;
```

---

## Part 4: Update lib.rs Exports (Optional)

If you want to expose the new types publicly:

```rust
// In lib.rs, add:
pub use engine::circuit::{Operation, BatchEntry, IngestBatch};
```

---

## Summary Checklist

### Must Do (circuit.rs)
- [ ] Add `Operation` enum
- [ ] Add `BatchEntry` struct  
- [ ] Add `IngestBatch` builder
- [ ] Add `ingest()` method to Circuit
- [ ] Add `ingest_entries()` method to Circuit
- [ ] Add `propagate_deltas()` helper method

### Optional (circuit.rs)
- [ ] Update `ingest_batch_spooky()` to use new types internally

### Optional (view.rs) - Do Incrementally
- [ ] Add `ProcessContext` struct
- [ ] Add `DeltaCategories` struct
- [ ] Refactor `process_ingest()` to use ProcessContext
- [ ] Extract helper methods one at a time

### Testing
- [ ] Add `tests/improved_changes_test.rs`
- [ ] Run existing tests to verify no regressions
- [ ] Run new tests to verify new functionality

---

## Usage After Integration

```rust
// Old way (still works)
circuit.ingest_batch(vec![
    ("users".into(), "CREATE".into(), "users:1".into(), json!({...}), "hash".into()),
], true);

// New way (more ergonomic)
circuit.ingest(
    IngestBatch::new()
        .create("users", "users:1", record, hash)
        .update("posts", "posts:1", record2, hash2)
        .delete("comments", "comments:1"),
    true
);

// Or with pre-built entries
let entries = vec![
    BatchEntry::new("users", Operation::Create, "users:1", record, hash),
];
circuit.ingest_entries(entries, true);
```

---

## Files Reference

| File | Purpose |
|------|---------|
| `improved_circuit.rs` | Reference for circuit.rs additions |
| `improved_view_process.rs` | Reference for view.rs refactoring |
| `improved_changes_test.rs` | Ready-to-use test file |
| `DBSP_ANALYSIS.md` | Full analysis document |
| `VIEW_COMPARISON_ANALYSIS.md` | Detailed view.rs comparison |
| `INTEGRATION_GUIDE.md` | This file |
