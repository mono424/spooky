# Deep Analysis: `propagate_deltas()`

## Function Signature

```rust
pub fn propagate_deltas(
    &mut self, 
    mut table_deltas: FastMap<String, ZSet>, 
    batch_meta: Option<&BatchMeta>,
    is_optimistic: bool
) -> Vec<ViewUpdate>
```

---

## 1. Overview

**Purpose:** This function is the heart of the DBSP (Database Stream Processing) circuit. It takes changes (deltas) that occurred in database tables and propagates them to all registered views that depend on those tables.

**When is it called?** After records are ingested (Create/Update/Delete operations) and the table deltas have been computed.

### Execution Phases

| Phase | Description | Complexity |
|-------|-------------|------------|
| 1. Cleanup | Remove zero-weight entries from deltas | O(n) |
| 2. Apply | Apply deltas to database ZSets | O(n) |
| 3. Rebuild | Lazy rebuild of dependency graph | O(views × tables) |
| 4. Identify | Find affected views | O(changed_tables) |
| 5. Dedupe | Remove duplicate view indices | O(n log n) |
| 6. Execute | Process each affected view | O(affected_views) |

---

## 2. Parameters Explained

| Parameter | Type | Purpose |
|-----------|------|---------|
| `&mut self` | `Circuit` | Mutable reference to circuit (contains db, views, dependency_graph) |
| `table_deltas` | `FastMap<String, ZSet>` | Map of table names to their change sets (ZSet = Map<RowKey, Weight>) |
| `batch_meta` | `Option<&BatchMeta>` | Optional metadata about the batch (versions, timestamps) |
| `is_optimistic` | `bool` | Whether to use optimistic update strategy for views |

---

## 3. Phase 1: Cleanup & Apply Deltas

### Code

```rust
let mut changed_tables = Vec::with_capacity(table_deltas.len());

for (table, delta) in &mut table_deltas {
    // Remove entries with weight = 0
    delta.retain(|_, w| *w != 0);
    
    if !delta.is_empty() {
        let tb = self.db.ensure_table(table.as_str());
        tb.apply_delta(delta);
        changed_tables.push(table.to_string());
    }
}
```

### Step-by-Step Explanation

**Step 1: Pre-allocate changed_tables vector**

We know the maximum number of changed tables equals the number of deltas, so we pre-allocate to avoid reallocation during the loop.

**Step 2: Filter zero-weight entries**

A ZSet maps RowKey → Weight. Weight represents:
- `+1` = Record was added (Create)
- `-1` = Record was removed (Delete)  
- `0` = Changes cancelled out (e.g., Create then Delete in same batch)

We remove weight=0 entries because they represent no net change.

**Example:**

| Before `retain()` | | After `retain()` |
|-------------------|---|------------------|
| user_1 → +1 | → | user_1 → +1 |
| user_2 → 0 | → | *(removed)* |
| user_3 → -1 | → | user_3 → -1 |

**Step 3: Apply delta to table ZSet**

Each Table has its own ZSet that tracks the current state of all records. `apply_delta()` merges the new weights with existing weights:

```rust
pub fn apply_delta(&mut self, delta: &ZSet) {
    for (key, weight) in delta {
        let entry = self.zset.entry(key.clone()).or_insert(0);
        *entry += weight;
        if *entry == 0 {
            self.zset.remove(key);  // Clean up zero entries
        }
    }
}
```

> ⚠️ **Optimization Note:** The code converts SmolStr to String multiple times (`table.as_str()`, `table.to_string()`). Consider using SmolStr throughout the chain to avoid heap allocations for short strings (<23 bytes).

---

## 4. Phase 2: Lazy Dependency Graph Rebuild

### Code

```rust
if self.dependency_graph.is_empty() && !self.views.is_empty() {
    self.rebuild_dependency_graph();
}
```

### What is the Dependency Graph?

The `dependency_graph` maps each table name to a list of view indices that depend on it:

```rust
dependency_graph: FastMap<String, Vec<usize>>
```

**Example:**

| Table | View Indices | Meaning |
|-------|--------------|---------|
| "users" | [0, 2, 5] | Views 0, 2, 5 query the users table |
| "orders" | [1, 2] | Views 1, 2 query the orders table |
| "products" | [1, 3, 4] | Views 1, 3, 4 query the products table |

### Why Lazy Rebuild?

**Problem:** The dependency_graph is marked with `#[serde(skip)]`, meaning it's NOT serialized. After deserializing a Circuit, it will be empty.

**Solution:** Instead of requiring a manual `rebuild_dependency_graph()` call after deserialization, we check if it's empty on first use and rebuild automatically. This is the **"lazy initialization"** pattern.

### Why Skip Serialization?

1. **Derived State:** The graph is computed from `views[].plan` - it's redundant to store it
2. **Index Validity:** `Vec<usize>` indices could be invalid after views are added/removed
3. **Smaller Payload:** Less data to serialize/deserialize

### How rebuild_dependency_graph() Works

```rust
pub fn rebuild_dependency_graph(&mut self) {
    self.dependency_graph.clear();
    for (i, view) in self.views.iter().enumerate() {
        let tables = view.plan.root.referenced_tables();
        for t in tables {
            self.dependency_graph.entry(t).or_default().push(i);
        }
    }
}
```

---

## 5. Phase 3: Identify Affected Views

### Code

```rust
let mut impacted_view_indices: Vec<usize> = Vec::with_capacity(self.views.len());

for table in changed_tables {
    if let Some(indices) = self.dependency_graph.get(&table) {
        impacted_view_indices.extend(indices.iter().copied());
    }
}

// Deduplicate
impacted_view_indices.sort_unstable();
impacted_view_indices.dedup();
```

### Visual Example

Suppose we have:
- `changed_tables = ["users", "orders"]`
- `dependency_graph = { "users" → [0, 2, 5], "orders" → [1, 2] }`

| Step | Operation | impacted_view_indices |
|------|-----------|----------------------|
| 1 | Lookup "users" | [0, 2, 5] |
| 2 | Lookup "orders" | [0, 2, 5, 1, 2] |
| 3 | sort_unstable() | [0, 1, 2, 2, 5] |
| 4 | dedup() | [0, 1, 2, 5] ✓ |

### Why Deduplication is Necessary

View 2 depends on BOTH users and orders. If both tables changed, View 2's index would appear twice. Without dedup, we would:
- Process View 2 twice (wasting CPU)
- Potentially cause incorrect results (double-counting changes)

> ⚠️ **Optimization Note:** For many views, a `HashSet` might be faster than `Vec + sort + dedup`. HashSet has O(1) insert with automatic deduplication vs O(n log n) for sort.

---

## 6. Phase 4: Execute View Updates

### Sequential Execution (WASM / No Parallel Feature)

```rust
#[cfg(any(target_arch = "wasm32", not(feature = "parallel")))]
{
    let mut ups = Vec::new();
    for i in impacted_view_indices {
        if i < self.views.len() {
            let view: &mut View = &mut self.views[i];
            if let Some(update) = view.process_ingest_with_meta(
                deltas_ref, db_ref, is_optimistic, batch_meta
            ) {
                ups.push(update);
            }
        }
    }
    ups
}
```

### Parallel Execution (Native with Rayon)

```rust
#[cfg(all(feature = "parallel", not(target_arch = "wasm32")))]
{
    use rayon::prelude::*;
    
    self.views
        .par_iter_mut()
        .enumerate()
        .filter_map(|(i, view)| {
            // binary_search is O(log n) - efficient because list is sorted
            if impacted_view_indices.binary_search(&i).is_ok() {
                view.process_ingest_with_meta(deltas_ref, db_ref, is_optimistic, batch_meta)
            } else {
                None
            }
        })
        .collect()
}
```

### Key Differences

| Aspect | Sequential | Parallel (Rayon) |
|--------|------------|------------------|
| Iteration | Only impacted views | All views, filter with binary_search |
| Parallelism | Single thread | Work-stealing thread pool |
| Use Case | WASM (no threads) | Native with many cores |
| Complexity | O(impacted) | O(total_views) but parallelized |

### Why binary_search in Parallel?

In parallel mode, Rayon iterates over ALL views (`par_iter_mut`). We can't just iterate over the indices because Rayon needs to split the work. So we check each view's index against our sorted list using `binary_search()` which is O(log n).

> ⚠️ **Optimization Note:** A `BitSet` would give O(1) lookup instead of O(log n) binary_search.

---

## 7. Complete Data Flow Diagram

```
┌─────────────────────────────────────────────────────────────────────┐
│                        INPUT                                        │
│   table_deltas: { "users" → {id1: +1, id2: -1}, "orders" → {...} } │
└─────────────────────────────────────────────────────────────────────┘
                                │
                                ▼
┌─────────────────────────────────────────────────────────────────────┐
│  PHASE 1: Cleanup                                                   │
│  • Remove weight=0 entries from each ZSet                          │
│  • Skip empty deltas                                                │
└─────────────────────────────────────────────────────────────────────┘
                                │
                                ▼
┌─────────────────────────────────────────────────────────────────────┐
│  PHASE 2: Apply to Database                                         │
│  • For each non-empty delta:                                        │
│    - Get or create table (ensure_table)                             │
│    - Merge delta weights into table's ZSet (apply_delta)            │
│    - Track table name in changed_tables                             │
└─────────────────────────────────────────────────────────────────────┘
                                │
                                ▼
┌─────────────────────────────────────────────────────────────────────┐
│  PHASE 3: Lazy Rebuild (if needed)                                  │
│  • If dependency_graph is empty AND views exist:                    │
│    - Rebuild graph from view query plans                            │
└─────────────────────────────────────────────────────────────────────┘
                                │
                                ▼
┌─────────────────────────────────────────────────────────────────────┐
│  PHASE 4: Find Affected Views                                       │
│  • For each changed_table:                                          │
│    - Lookup view indices in dependency_graph                        │
│    - Collect all indices                                            │
│  • Sort and deduplicate indices                                     │
└─────────────────────────────────────────────────────────────────────┘
                                │
                                ▼
┌─────────────────────────────────────────────────────────────────────┐
│  PHASE 5: Execute Updates                                           │
│  • For each affected view:                                          │
│    - Call process_ingest_with_meta()                                │
│    - Collect ViewUpdate results                                     │
│  • Return Vec<ViewUpdate>                                           │
└─────────────────────────────────────────────────────────────────────┘
                                │
                                ▼
┌─────────────────────────────────────────────────────────────────────┐
│                        OUTPUT                                       │
│   Vec<ViewUpdate> - Changes to send to clients                     │
└─────────────────────────────────────────────────────────────────────┘
```

---

## 8. Optimization Opportunities

| Location | Current | Suggested | Impact |
|----------|---------|-----------|--------|
| table key type | `String` | `SmolStr` | Avoid heap allocs for short names |
| changed_tables | `Vec<String>` | `Vec<SmolStr>` or `SmallVec` | Less allocations |
| deduplication | `sort + dedup` | `HashSet` (for many views) | O(n) vs O(n log n) |
| ensure_table | `table.as_str()` | `&SmolStr` directly | 0 allocations if <23 bytes |
| parallel check | `binary_search` | `BitSet` | O(1) vs O(log n) |

---

## 9. Key DBSP Concepts

### ZSet (Z-Set)

A map from keys to integer weights. The 'Z' refers to integers (ℤ).

```rust
type ZSet = FastMap<RowKey, i64>;
```

- Positive weight = record exists (count)
- Negative weight = record deleted (debt)
- Zero weight = cancelled out / doesn't exist

### Delta

A ZSet representing **changes**, not absolute state.

- Weight `+1` means "add this record"
- Weight `-1` means "remove this record"

### Incremental View Maintenance

Instead of recomputing entire view results on every change, we:
1. Receive only the **delta** (what changed)
2. Apply the delta to the view incrementally
3. Output only what changed in the view

This gives us **O(delta_size)** complexity instead of **O(total_data_size)**.

### Dependency Graph

Tracks which views depend on which tables, allowing efficient routing of deltas to only the views that need them.

```
Table "users" changed → Only update Views [0, 2, 5]
                        (not all 100 views!)
```

---

## 10. Summary

`propagate_deltas()` is the core function that makes DBSP work incrementally:

1. **It receives CHANGES (deltas)**, not full state
2. **It applies those changes** to the database's ZSets
3. **It efficiently routes changes** only to affected views (via dependency graph)
4. **It supports both sequential (WASM) and parallel (native)** execution
5. **It uses lazy initialization** for the dependency graph

**The key insight:** By working with deltas (changes) instead of full state, and by maintaining a dependency graph to route changes efficiently, we achieve **O(delta_size)** complexity instead of **O(total_data_size)** for each update.

This is what makes DBSP suitable for real-time, incremental view maintenance at scale.
