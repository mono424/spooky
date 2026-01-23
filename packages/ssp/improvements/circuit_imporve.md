# Circuit.rs Implementation Analysis

## Overview

Analyzing the provided `circuit.rs` implementation for correctness, compatibility, and potential issues.

---

## ✅ Correctly Implemented

### 1. Module Organization
```rust
pub mod types { ... }  // ViewIndex, TableName, DependencyList
pub mod dto { ... }    // BatchEntry, LoadRecord
```
**Good:** Clean separation of concerns with type aliases and DTOs in submodules.

### 2. Type Aliases with SmallVec
```rust
pub type DependencyList = SmallVec<[ViewIndex; 4]>;
```
**Good:** Uses `SmallVec` for inline allocation when ≤4 views depend on a table (covers 90%+ cases).

### 3. BatchEntry & LoadRecord
```rust
impl BatchEntry {
    pub fn new(...) -> Self;
    pub fn create(...) -> Self;
    pub fn update(...) -> Self;
    pub fn delete(...) -> Self;
}
```
**Good:** Convenience constructors implemented correctly.

### 4. Table::apply_mutation()
```rust
pub fn apply_mutation(&mut self, op: Operation, key: SmolStr, data: SpookyValue) -> (SmolStr, i64) {
    let weight = op.weight();
    match op {
        Operation::Create | Operation::Update => { self.rows.insert(key.clone(), data); }
        Operation::Delete => { self.rows.remove(&key); }
    }
    let zset_key = build_zset_key(&self.name, &key);
    // ... update zset
    (zset_key, weight)
}
```
**Good:** Consolidated mutation logic, returns zset_key + weight for caller.

### 5. Database Compatibility Fix
```rust
pub struct Database {
    pub tables: FastMap<String, Table>,  // String keys for view.rs compatibility
}
```
**Good:** Correctly identified that `view.rs` uses `db.tables.get(table_name)` with `&str`, which requires `String` keys in the HashMap (since `&str` can look up `String` keys via `Borrow` trait).

### 6. build_zset_key() Helper
```rust
fn build_zset_key(table: &str, id: &str) -> SmolStr {
    let combined_len = table.len() + 1 + id.len();
    if combined_len <= 23 { /* inline */ } else { /* heap */ }
}
```
**Good:** Optimized to avoid heap allocation for short keys. Signature changed to `(&str, &str)` which is more flexible.

### 7. Parallel Batch Processing
```rust
#[cfg(all(feature = "parallel", not(target_arch = "wasm32")))]
{
    // P2.1: Ensure tables exist sequentially
    for name in by_table.keys() {
        self.db.ensure_table(name.as_str());
    }
    // P2.2: Parallel Delta Computation
    let results: Vec<(String, ZSet)> = self.db.tables.par_iter_mut()...
}
```
**Good:** Correctly handles the issue that `ensure_table` needs mutable access, so tables are created first, then parallel mutation.

### 8. swap_remove for View Unregistration
```rust
fn unregister_view_by_index(&mut self, index: usize) {
    self.views.swap_remove(index);
    // Fix up moved view's index in dependency graph
    if index < self.views.len() { ... }
    self.rebuild_dependency_graph();
}
```
**Good:** O(1) removal with `swap_remove`, though it still rebuilds the full dependency graph.

---

## ⚠️ Potential Issues

### Issue 1: Redundant Dependency Graph Rebuild

```rust
fn unregister_view_by_index(&mut self, index: usize) {
    self.views.swap_remove(index);
    
    if index < self.views.len() {
        // Manual fix-up of moved view's indices
        ...
    }
    
    self.rebuild_dependency_graph();  // ← Rebuilds everything anyway!
}
```

**Problem:** The manual fix-up code (lines 298-310) is immediately followed by a full `rebuild_dependency_graph()`, making the fix-up code dead/useless.

**Recommendation:** Either:
- Remove the manual fix-up and just call `rebuild_dependency_graph()`, OR
- Remove `rebuild_dependency_graph()` and fix the manual fix-up to be complete

```rust
// Option A: Simple (current behavior, just cleaner)
fn unregister_view_by_index(&mut self, index: usize) {
    self.views.swap_remove(index);
    self.rebuild_dependency_graph();
}

// Option B: Optimized (avoid full rebuild)
fn unregister_view_by_index(&mut self, index: usize) {
    // Remove old view's entries from dependency graph
    if let Some(view) = self.views.get(index) {
        for t in view.plan.root.referenced_tables() {
            if let Some(deps) = self.dependency_graph.get_mut(t.as_str()) {
                deps.retain(|&i| i != index);
            }
        }
    }
    
    self.views.swap_remove(index);
    
    // Fix up swapped view's indices (was at end, now at `index`)
    if index < self.views.len() {
        let old_index = self.views.len(); // The view that was swapped
        for t in self.views[index].plan.root.referenced_tables() {
            if let Some(deps) = self.dependency_graph.get_mut(t.as_str()) {
                for idx in deps.iter_mut() {
                    if *idx == old_index { *idx = index; }
                }
            }
        }
    }
}
```

---

### Issue 2: view.rs Compatibility - rows.get(id) expects SmolStr key

In `view.rs:583`:
```rust
db.tables.get(table_name)?.rows.get(id)  // id is &str
```

But in `circuit.rs`, `Table.rows` is:
```rust
pub rows: FastMap<RowKey, SpookyValue>,  // RowKey = SmolStr
```

**Analysis:** This should work because:
- `SmolStr` implements `Borrow<str>`
- `HashMap::get()` accepts `Q: Borrow<K>`, so `&str` can look up `SmolStr` keys

**Verdict:** ✅ Compatible, no issue.

---

### Issue 3: Dependency Graph Key Type Mismatch

```rust
pub dependency_graph: FastMap<TableName, DependencyList>,  // TableName = SmolStr
```

But in `propagate_deltas`:
```rust
for table in changed_tables {  // table is &TableName (SmolStr)
    if let Some(indices) = self.dependency_graph.get(table) { ... }
}
```

**Analysis:** `SmolStr` implements `Borrow<str>`, so this works.

But in `unregister_view_by_index`:
```rust
if let Some(deps) = self.dependency_graph.get_mut(t.as_str()) {  // t is String
```

**Analysis:** `dependency_graph` has `SmolStr` keys, and we're looking up with `&str`. This works because `SmolStr: Borrow<str>`.

**Verdict:** ✅ Compatible, no issue.

---

### Issue 4: ingest_single Returns After First Update

```rust
for idx in 0..view_count {
    let view_idx = self.dependency_graph.get(&table_key).unwrap()[idx];
    if let Some(view) = self.views.get_mut(view_idx) {
        if let Some(update) = view.process_single(&delta, &self.db) {
            return Some(update);  // ← Returns first update only!
        }
    }
}
```

**Question:** Is this intentional? If multiple views depend on the same table, only the first view's update is returned.

**Analysis:** Looking at the original code and the plan, `ingest_single` is meant for the 99% case where there's typically one view affected. Returning `Option<ViewUpdate>` (not `Vec`) was a deliberate choice.

**Recommendation:** This is correct for the stated design, but should be documented clearly:
```rust
/// Returns the first view update, if any. For multiple views, use `ingest_batch()`.
```

---

### Issue 5: Parallel Batch - Clone in Closure

```rust
let results: Vec<(String, ZSet)> = self.db.tables
    .par_iter_mut()
    .filter_map(|(name, table)| {
        let name_smol = SmolStr::new(name);  // Creates new SmolStr each iteration
        let entries = by_table.get(&name_smol)?;
        
        for entry in entries {
            let (zset_key, weight) = table.apply_mutation(
                entry.op, 
                entry.id.clone(),      // Clone
                entry.data.clone()     // Clone - potentially expensive!
            );
        }
        ...
    })
```

**Problem:** `entry.data.clone()` clones potentially large `SpookyValue` objects.

**Recommendation:** Consider using `std::mem::take` or restructuring to avoid clones:
```rust
// If entries can be consumed:
for entry in entries.drain(..) {
    let (zset_key, weight) = table.apply_mutation(entry.op, entry.id, entry.data);
}
```

But this requires `by_table` to be mutable in the parallel context, which is tricky. The current approach is correct but has performance cost.

---

### Issue 6: Missing `Default` Implementation for Circuit

```rust
impl Circuit {
    pub fn new() -> Self { ... }
}
// Missing: impl Default for Circuit
```

**Recommendation:** Add for consistency:
```rust
impl Default for Circuit {
    fn default() -> Self { Self::new() }
}
```

---

## 🔍 Compatibility Check with view.rs

| view.rs Usage | circuit.rs Implementation | Compatible? |
|---------------|---------------------------|-------------|
| `db.tables.get(table_name)` | `FastMap<String, Table>` | ✅ Yes |
| `table.rows.get(id)` where id is `&str` | `FastMap<SmolStr, SpookyValue>` | ✅ Yes (Borrow trait) |
| `view.process_single(&delta, &self.db)` | `Delta` unchanged | ✅ Yes |
| `view.process_ingest(deltas, db_ref)` | `FastMap<String, ZSet>` | ✅ Yes |

---

## 📋 Summary

| Category | Status | Notes |
|----------|--------|-------|
| 3 Ingestion APIs | ✅ Correct | `ingest_single`, `ingest_batch`, `init_load` |
| Type System | ✅ Correct | SmolStr, SmallVec, type aliases |
| Database Compatibility | ✅ Correct | String keys for view.rs lookup |
| Parallel Processing | ✅ Correct | Sequential table creation, parallel mutation |
| build_zset_key | ✅ Correct | Optimized for short keys |
| Dependency Graph | ⚠️ Minor | Redundant code in unregister |
| Single Ingest Return | ⚠️ Design Choice | Only returns first update |
| Clone in Parallel | ⚠️ Performance | data.clone() in parallel path |
| Default Impl | ❌ Missing | Should add for Circuit |

---

## Recommended Fixes

### Fix 1: Clean up unregister_view_by_index
```rust
fn unregister_view_by_index(&mut self, index: usize) {
    self.views.swap_remove(index);
    self.rebuild_dependency_graph();
}
```

### Fix 2: Add Default implementation
```rust
impl Default for Circuit {
    fn default() -> Self { Self::new() }
}
```

### Fix 3: Document ingest_single behavior
```rust
/// Single record ingestion - optimized for the 99% use case.
/// 
/// **Note:** If multiple views depend on the affected table, only the first
/// view's update is returned. For comprehensive updates, use `ingest_batch()`.
pub fn ingest_single(...) -> Option<ViewUpdate>
```

---

## Verdict

**The implementation is functionally correct and compatible with view.rs.** 

The issues identified are minor:
- Redundant code (doesn't affect correctness)
- Missing `Default` impl (convenience)
- Performance consideration for parallel clones (acceptable trade-off)

**Ready for use** with the minor fixes above.