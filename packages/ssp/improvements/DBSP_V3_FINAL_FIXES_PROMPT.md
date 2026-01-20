# DBSP Engine V3 - Final Polish Prompt

## Context

The V3 engine has successfully implemented most of the major improvements. This prompt addresses the remaining issues to make it production-ready.

## Priority 1: MUST FIX (Performance/Correctness)

### 1.1 Fix O(n²) Bug in build_materialized_raw_result (view.rs:500-510)

**Current (O(n²)):**
```rust
for id in all_ids {
    let is_update = changes.updates.iter().any(|u| u.as_str() == id.as_str());  // O(n) per iteration!
    let is_new = additions_set.contains(id.as_str());
    // ...
}
```

**Fix (O(n)):**
```rust
fn build_materialized_raw_result(
    &mut self,
    raw: &mut RawViewResult,
    changes: &CategorizedChanges,
    is_optimistic: bool,
    processor: &MetadataProcessor,
    ctx: &ProcessContext,
    db: &Database,
) {
    // Build full snapshot
    let result_ids: Vec<String> = self.cache.keys().map(|k| k.to_string()).collect();
    let mut all_ids = Vec::new();
    for id in &result_ids {
        all_ids.push(id.clone());
        if let Some(parent_row) = self.get_row_value(id, db) {
            self.collect_subquery_ids_recursive(&self.plan.root, parent_row, db, &mut all_ids);
        }
    }
    all_ids.sort_unstable();
    all_ids.dedup();
    
    // Pre-build HashSets for O(1) lookups
    let additions_set: std::collections::HashSet<&str> = 
        changes.additions.iter().map(|id| id.as_str()).collect();
    let updates_set: std::collections::HashSet<&str> = 
        changes.updates.iter().map(|id| id.as_str()).collect();

    for id in all_ids {
        let is_update = updates_set.contains(id.as_str());  // O(1)
        let is_new = additions_set.contains(id.as_str());   // O(1)
        
        let version = self.compute_and_store_version(&id, processor, ctx, is_new, is_optimistic && is_update);
        raw.records.push((id, version));
    }
}
```

### 1.2 Fix Backward Compatibility for register_view (circuit.rs:558-599)

**Current (breaking):**
```rust
pub fn register_view(
    &mut self,
    plan: QueryPlan,
    params: Option<Value>,
    format: Option<ViewResultFormat>,
    strategy: Option<VersionStrategy>,  // ❌ Added 4th param
) -> Option<ViewUpdate>
```

**Fix:**
```rust
/// Register a view (backward compatible)
pub fn register_view(
    &mut self,
    plan: QueryPlan,
    params: Option<Value>,
    format: Option<ViewResultFormat>,
) -> Option<ViewUpdate> {
    self.register_view_with_strategy(plan, params, format, None)
}

/// Register a view with explicit version strategy
pub fn register_view_with_strategy(
    &mut self,
    plan: QueryPlan,
    params: Option<Value>,
    format: Option<ViewResultFormat>,
    strategy: Option<VersionStrategy>,
) -> Option<ViewUpdate> {
    if let Some(pos) = self.views.iter().position(|v| v.plan.id == plan.id) {
        self.views.remove(pos);
        self.rebuild_dependency_graph();
    }

    let mut view = View::new_with_strategy(
        plan, 
        params, 
        format.clone(), 
        strategy.unwrap_or_else(|| match format {
            Some(ViewResultFormat::Tree) => VersionStrategy::HashBased,
            _ => VersionStrategy::Optimistic,
        })
    );

    let empty_deltas: FastMap<String, ZSet> = FastMap::default();
    let initial_update = view.process_ingest(&empty_deltas, &self.db, true);

    let view_idx = self.views.len();
    self.views.push(view);

    if let Some(v) = self.views.last() {
        let tables = v.plan.root.referenced_tables();
        for t in tables {
            self.dependency_graph.entry(t).or_default().push(view_idx);
        }
    }

    initial_update
}
```

## Priority 2: SHOULD FIX (Code Quality)

### 2.1 Remove Duplicate Comments

**view.rs lines 169-172:**
```rust
// REMOVE lines 171-172 (duplicate of 169-170):
/// Optimized 2-Phase Processing: Handles multiple table updates at once.
/// is_optimistic: true = increment versions (local mutations), false = keep versions (remote sync)
```

**circuit.rs line 518:**
```rust
// REMOVE line 518 (duplicate of 517):
// 3. Execution Phase
```

### 2.2 Remove Dev Comment (circuit.rs:228)

```rust
// REMOVE this line:
// I will just use 'Table' name but with new types.
```

### 2.3 Remove Redundant all_updates (circuit.rs:515-555)

**Current:**
```rust
let mut all_updates: Vec<ViewUpdate> = Vec::new();
// ... parallel or sequential computation produces `updates`
all_updates.extend(updates);
all_updates
```

**Fix:**
```rust
// Just return updates directly, no intermediate vector needed
#[cfg(all(feature = "parallel", not(target_arch = "wasm32")))]
{
    use rayon::prelude::*;
    self.views
        .par_iter_mut()
        .enumerate()
        .filter_map(|(i, view)| {
            if impacted_view_indices.binary_search(&i).is_ok() {
                view.process_ingest_with_meta(deltas_ref, db_ref, is_optimistic, batch_meta)
            } else {
                None
            }
        })
        .collect()
}

#[cfg(any(target_arch = "wasm32", not(feature = "parallel")))]
{
    let mut updates = Vec::with_capacity(impacted_view_indices.len());
    for i in impacted_view_indices {
        if i < self.views.len() {
            if let Some(update) = self.views[i].process_ingest_with_meta(deltas_ref, db_ref, is_optimistic, batch_meta) {
                updates.push(update);
            }
        }
    }
    updates
}
```

## Priority 3: NICE TO HAVE (Optimization)

### 3.1 Use Subquery Tables Cache (view.rs)

Add this method to View:
```rust
/// Get or compute subquery tables (cached)
fn get_subquery_tables(&mut self) -> &std::collections::HashSet<SmolStr> {
    if self.subquery_tables_cache.is_none() {
        self.subquery_tables_cache = Some(
            self.extract_subquery_tables(&self.plan.root)
                .into_iter()
                .map(SmolStr::from)
                .collect()
        );
    }
    self.subquery_tables_cache.as_ref().unwrap()
}

/// Clear the cache (call when plan changes, if ever)
#[allow(dead_code)]
fn clear_subquery_cache(&mut self) {
    self.subquery_tables_cache = None;
}
```

Then update `compute_changes()` to use it. Note: `compute_changes` takes `&self`, so either:
1. Change to `&mut self`, or
2. Use interior mutability (`RefCell`), or  
3. Keep current behavior (acceptable for fallback path)

**Recommendation:** Keep current behavior - the fallback path is rare and `extract_subquery_tables` is cheap.

### 3.2 Use sort_unstable_by in update.rs (line 87)

```rust
// BEFORE:
sorted_data.sort_by(|a, b| a.0.cmp(&b.0));

// AFTER:
sorted_data.sort_unstable_by(|a, b| a.0.cmp(&b.0));
```

### 3.3 Add #[inline] to Table::new (circuit.rs:194)

```rust
#[inline]
pub fn new(name: String) -> Self {
    Self {
        name,
        zset: FastMap::default(),
        rows: FastMap::default(),
        hashes: FastMap::default(),
    }
}
```

### 3.4 Add Default Implementations

```rust
// circuit.rs
impl Default for Database {
    fn default() -> Self {
        Self::new()
    }
}

impl Default for Circuit {
    fn default() -> Self {
        Self::new()
    }
}
```

## Summary Checklist

### Must Fix
- [ ] Fix O(n²) in `build_materialized_raw_result` - use HashSet for updates lookup
- [ ] Add `register_view_with_strategy()`, keep `register_view()` with 3 params

### Should Fix  
- [ ] Remove duplicate comment (view.rs:171-172)
- [ ] Remove duplicate comment (circuit.rs:518)
- [ ] Remove dev comment (circuit.rs:228)
- [ ] Remove redundant `all_updates` vector

### Nice to Have
- [ ] Use `sort_unstable_by` in update.rs
- [ ] Add `#[inline]` to `Table::new()`
- [ ] Add `Default` impls for Database and Circuit
- [ ] (Optional) Use subquery_tables_cache

## Verification Tests

After applying fixes, ensure these tests pass:

```rust
#[test]
fn test_backward_compat_register_view() {
    let mut circuit = Circuit::new();
    // Old 3-param API still works
    circuit.register_view(plan, None, Some(ViewResultFormat::Streaming));
}

#[test]
fn test_register_view_with_strategy() {
    let mut circuit = Circuit::new();
    // New 4-param API
    circuit.register_view_with_strategy(
        plan, 
        None, 
        Some(ViewResultFormat::Streaming),
        Some(VersionStrategy::Explicit)
    );
}

#[test]
fn test_build_materialized_performance() {
    // Create a view with 1000+ records
    // Verify build_materialized_raw_result completes in < 10ms
    // (Previously would be slow due to O(n²))
}
```

## Final Notes

After these fixes, the engine will be:
- ✅ Feature complete (all requested features implemented)
- ✅ Performance optimized (no O(n²) bugs)
- ✅ Backward compatible (old APIs preserved)
- ✅ Clean code (no duplicate comments, no dead code)

Total estimated effort: **30-60 minutes**
