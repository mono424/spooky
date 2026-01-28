# Implementation Plan: Improved `ingest_single()`

## Problem Statement

Current `ingest_single()` returns `Option<ViewUpdate>` - only the **first** view update. This is problematic because:

1. **Unpredictable behavior**: Caller doesn't know which view's update they'll get
2. **Lost updates**: If 3 views depend on "users" table, 2 updates are silently dropped
3. **Inconsistent with `ingest_batch()`**: Batch returns all updates, single doesn't
4. **Can't be used safely**: Developer must know view topology to use correctly

### Example Problem

```rust
// Setup: 3 views depend on "users" table
circuit.register_view(users_list_view, ...);    // View 0
circuit.register_view(users_count_view, ...);   // View 1  
circuit.register_view(users_active_view, ...);  // View 2

// Problem: Only View 0's update returned, Views 1 & 2 updates LOST
let update = circuit.ingest_single("users", Operation::Create, "u1", data);
// update = Some(View0Update)  -- View1Update and View2Update are gone!
```

---

## Solution

Change `ingest_single()` to return **all affected view updates** while keeping it optimized for the single-record case.

### New Signature

```rust
// OLD (broken)
pub fn ingest_single(...) -> Option<ViewUpdate>

// NEW (correct)
pub fn ingest_single(...) -> SmallVec<[ViewUpdate; 2]>
```

### Why `SmallVec<[ViewUpdate; 2]>`?

| Return Type | Pros | Cons |
|-------------|------|------|
| `Vec<ViewUpdate>` | Simple | Always allocates heap |
| `Option<ViewUpdate>` | No allocation | Only 1 update (broken) |
| `SmallVec<[ViewUpdate; 2]>` | Inline for ≤2 updates, heap for more | Slight complexity |

**Rationale**: Most single-record ingests affect 1-2 views. `SmallVec<[_; 2]>` stores up to 2 updates on the stack (no heap allocation), covering the common case efficiently.

---

## Implementation

### Step 1: Add SmallVec Import

```rust
use smallvec::SmallVec;

// Add type alias for clarity
pub type ViewUpdateList = SmallVec<[ViewUpdate; 2]>;
```

### Step 2: Update `ingest_single()`

```rust
/// Single record ingestion - returns ALL affected view updates
/// 
/// Optimized for single-record mutations while correctly handling
/// multiple dependent views.
/// 
/// # Returns
/// - `SmallVec<[ViewUpdate; 2]>` - All view updates (inline for ≤2, heap for more)
/// 
/// # Example
/// ```rust
/// let updates = circuit.ingest_single("users", Operation::Create, "u1", data);
/// for update in updates {
///     send_to_client(update);
/// }
/// ```
pub fn ingest_single(
    &mut self,
    table: &str,
    op: Operation,
    id: &str,
    data: SpookyValue,
) -> ViewUpdateList {
    let key = SmolStr::new(id);
    let (zset_key, weight) = self.db.ensure_table(table).apply_mutation(op, key, data);

    // Early return if no actual change (e.g., delete of non-existent)
    if weight == 0 {
        return SmallVec::new();
    }

    self.ensure_dependency_graph();

    let table_key = SmolStr::new(table);
    
    // Get view indices for this table
    let view_indices: SmallVec<[ViewIndex; 4]> = self
        .dependency_graph
        .get(&table_key)
        .map(|v| v.iter().copied().collect())
        .unwrap_or_default();

    if view_indices.is_empty() {
        return SmallVec::new();
    }

    // Build delta once
    let delta = Delta::new(table_key, zset_key, weight);

    // Collect ALL updates
    let mut updates: ViewUpdateList = SmallVec::new();
    
    for view_idx in view_indices {
        if let Some(view) = self.views.get_mut(view_idx) {
            if let Some(update) = view.process_single(&delta, &self.db) {
                updates.push(update);
            }
        }
    }

    updates
}
```

### Step 3: Alternative - Keep Both Signatures

If you want backwards compatibility or a true "single update" fast path:

```rust
/// Single record ingestion - returns ALL affected view updates
pub fn ingest_single(
    &mut self,
    table: &str,
    op: Operation,
    id: &str,
    data: SpookyValue,
) -> ViewUpdateList {
    // ... full implementation
}

/// Single record ingestion - returns only FIRST update (use when you KNOW only one view)
/// 
/// ⚠️ Only use this if you're certain only one view depends on the table,
/// or you only care about the first update.
pub fn ingest_single_first(
    &mut self,
    table: &str,
    op: Operation,
    id: &str,
    data: SpookyValue,
) -> Option<ViewUpdate> {
    self.ingest_single(table, op, id, data).into_iter().next()
}
```

---

## Full Updated Code

```rust
use smallvec::SmallVec;

pub mod types {
    use super::*;
    
    pub type ViewIndex = usize;
    pub type TableName = SmolStr;
    pub type DependencyList = SmallVec<[ViewIndex; 4]>;
    
    /// Return type for ingest_single - inline storage for ≤2 updates
    pub type ViewUpdateList = SmallVec<[ViewUpdate; 2]>;
}

use self::types::*;

impl Circuit {
    /// Single record ingestion - returns ALL affected view updates
    /// 
    /// # Performance
    /// - Optimized for single-record mutations
    /// - Returns `SmallVec` (no heap allocation for ≤2 updates)
    /// - Processes all dependent views
    /// 
    /// # Example
    /// ```rust
    /// let updates = circuit.ingest_single("users", Operation::Create, "u1", data);
    /// 
    /// // Handle all updates
    /// for update in updates {
    ///     websocket.send(update);
    /// }
    /// 
    /// // Or check if any updates occurred
    /// if !updates.is_empty() {
    ///     println!("Views updated: {}", updates.len());
    /// }
    /// ```
    pub fn ingest_single(
        &mut self,
        table: &str,
        op: Operation,
        id: &str,
        data: SpookyValue,
    ) -> ViewUpdateList {
        let key = SmolStr::new(id);
        let (zset_key, weight) = self.db.ensure_table(table).apply_mutation(op, key, data);

        if weight == 0 {
            return SmallVec::new();
        }

        self.ensure_dependency_graph();

        let table_key = SmolStr::new(table);
        
        // Clone indices to avoid borrow conflict with self.views
        let view_indices: SmallVec<[ViewIndex; 4]> = self
            .dependency_graph
            .get(&table_key)
            .map(|v| v.iter().copied().collect())
            .unwrap_or_default();

        if view_indices.is_empty() {
            return SmallVec::new();
        }

        let delta = Delta::new(table_key, zset_key, weight);
        let mut updates: ViewUpdateList = SmallVec::new();
        
        for view_idx in view_indices {
            if let Some(view) = self.views.get_mut(view_idx) {
                if let Some(update) = view.process_single(&delta, &self.db) {
                    updates.push(update);
                }
            }
        }

        updates
    }
}
```

---

## Migration Guide

### Before (Broken)
```rust
// Only got first update, others lost
if let Some(update) = circuit.ingest_single("users", op, "u1", data) {
    send(update);
}
```

### After (Correct)
```rust
// Get ALL updates
let updates = circuit.ingest_single("users", op, "u1", data);
for update in updates {
    send(update);
}

// Or if you expect 0-1 updates:
if let Some(update) = updates.into_iter().next() {
    send(update);
}

// Or check count:
if updates.is_empty() {
    println!("No views affected");
} else {
    println!("{} views updated", updates.len());
}
```

---

## Comparison: `ingest_single` vs `ingest_batch`

After this change, both methods behave consistently:

| Aspect | `ingest_single` | `ingest_batch` |
|--------|-----------------|----------------|
| Input | 1 record | N records |
| Returns | `SmallVec<[ViewUpdate; 2]>` | `Vec<ViewUpdate>` |
| All updates? | ✅ Yes | ✅ Yes |
| Heap allocation | Only if >2 updates | Always |
| Use case | Real-time single mutations | Bulk imports, batched changes |

---

## Checklist

- [ ] Add `ViewUpdateList` type alias
- [ ] Update `ingest_single()` to return `ViewUpdateList`
- [ ] Collect ALL view updates in loop (remove early return)
- [ ] Update documentation
- [ ] Update tests
- [ ] Update refactoring plan document

---

## Optional Enhancement: Parallel Single Ingest

For cases where many views depend on one table, you could parallelize:

```rust
#[cfg(all(feature = "parallel", not(target_arch = "wasm32")))]
pub fn ingest_single(...) -> ViewUpdateList {
    // ... setup ...
    
    if view_indices.len() > 4 {
        // Parallel path for many views
        use rayon::prelude::*;
        
        view_indices
            .par_iter()
            .filter_map(|&idx| {
                // Note: Requires unsafe or different architecture for mutable view access
            })
            .collect()
    } else {
        // Sequential path (current implementation)
    }
}
```

This is complex due to mutable borrow issues and probably not worth it for `ingest_single` which is meant for low-latency single operations.

---

*Document Version: 1.0*
*Status: Ready for Implementation*