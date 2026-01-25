# Implementation Plan: True DBSP Multiset Semantics

## Overview

This plan implements **Option B: True DBSP multiset semantics** where weights represent multiplicity (how many times a record appears in the result). This is the mathematically correct approach used in systems like Materialize, Differential Dataflow, and DBSP.

## Project Structure Reference

```
app/
└── ssp/
    └── src/
        └── lib.rs              # Server handlers

packages/
└── ssp/
    └── src/
        ├── circuit.rs          # Circuit, Database, Table
        ├── view.rs             # View, process_batch, delta computation
        ├── update.rs           # ViewUpdate, StreamingUpdate, DeltaEvent
        ├── operators.rs        # Operator enum, Predicate, Projection
        ├── types.rs            # ZSet, Delta, SpookyValue
        └── eval.rs             # Evaluation helpers
```

---

## Core Concepts

### DBSP ZSet Semantics

A ZSet is a map from elements to weights (integers):
- `weight > 0`: Element appears `weight` times (insertion)
- `weight < 0`: Element removed `|weight|` times (deletion)  
- `weight = 0`: No net change (or element not present)

### Delta Propagation

For an operator `O` with input `I`:
- `O(I + ΔI) = O(I) + O(ΔI)` (linearity property)
- We only need to process `ΔI` to get `ΔO`

### Key Insight for Your Bug

When a thread has a subquery `(SELECT * FROM user WHERE id=$parent.author)`:
- Thread 1 references User A → User A weight = 1
- Thread 2 references User A → User A weight = 2
- This is CORRECT in multiset semantics!

But for **edge creation**, we need to distinguish:
- First time User A appears (weight goes 0→1): CREATE edge
- Additional references (weight goes 1→2): No edge change (already exists)
- Last reference removed (weight goes 1→0): DELETE edge

---

## Phase 1: Fix the Double Table Prefix Bug (Day 1)

### Problem

Keys are being created as `"user:user:xyz"` instead of `"user:xyz"`.

### Step 1.1: Find the source of double prefix

**File:** `packages/ssp/src/view.rs`

Search for where ZSet keys are created. The bug is likely in `eval_snapshot` or `expand_with_subqueries`.

```rust
// WRONG - if id already contains table prefix
let key = format!("{}:{}", table, id);  // "user:user:xyz"

// CORRECT - id should NOT have table prefix
let key = format!("{}:{}", table, raw_id);  // "user:xyz"
```

### Step 1.2: Add helper function for consistent key creation

**File:** `packages/ssp/src/types.rs`

```rust
/// Create a ZSet key from table name and record ID
/// 
/// # Arguments
/// * `table` - Table name (e.g., "user")
/// * `id` - Raw record ID WITHOUT table prefix (e.g., "xyz123")
/// 
/// # Returns
/// * ZSet key in format "table:id" (e.g., "user:xyz123")
#[inline]
pub fn make_zset_key(table: &str, id: &str) -> SmolStr {
    // Strip any existing table prefix from id
    let raw_id = id.split_once(':').map(|(_, rest)| rest).unwrap_or(id);
    
    let combined_len = table.len() + 1 + raw_id.len();
    if combined_len <= 23 {
        // SmolStr inline storage optimization
        let mut buf = String::with_capacity(combined_len);
        buf.push_str(table);
        buf.push(':');
        buf.push_str(raw_id);
        SmolStr::new(buf)
    } else {
        SmolStr::new(format!("{}:{}", table, raw_id))
    }
}

/// Extract table and raw ID from a ZSet key
#[inline]
pub fn parse_zset_key(key: &str) -> Option<(&str, &str)> {
    key.split_once(':')
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_make_zset_key_simple() {
        assert_eq!(make_zset_key("user", "xyz123").as_str(), "user:xyz123");
    }
    
    #[test]
    fn test_make_zset_key_strips_prefix() {
        // If id already has prefix, strip it
        assert_eq!(make_zset_key("user", "user:xyz123").as_str(), "user:xyz123");
    }
    
    #[test]
    fn test_parse_zset_key() {
        assert_eq!(parse_zset_key("user:xyz123"), Some(("user", "xyz123")));
    }
}
```

### Step 1.3: Update all call sites

**File:** `packages/ssp/src/circuit.rs`

```rust
// In build_zset_key function - replace with:
use crate::types::make_zset_key;

// Remove the old build_zset_key function and use make_zset_key everywhere
```

**File:** `packages/ssp/src/view.rs`

```rust
// In eval_snapshot, Operator::Scan case:
Operator::Scan { table } => {
    if let Some(tb) = db.tables.get(table) {
        Cow::Borrowed(&tb.zset)
    } else {
        Cow::Owned(FastMap::default())
    }
}

// In expand_with_subqueries:
// Ensure keys are created correctly
let key = make_zset_key(&subquery_table, &record_id);
```

---

## Phase 2: Implement Proper Weight Tracking (Day 1-2)

### Problem

Currently, weights accumulate incorrectly and the delta computation doesn't handle multiplicity properly.

### Step 2.1: Define clear weight transition types

**File:** `packages/ssp/src/types.rs`

Add new types for weight transitions:

```rust
/// Represents a weight transition for delta computation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeightTransition {
    /// Record newly appears (old_weight <= 0, new_weight > 0)
    Inserted,
    /// Record's multiplicity increased (old_weight > 0, new_weight > old_weight)
    MultiplicityIncreased,
    /// Record's multiplicity decreased but still present (new_weight > 0, new_weight < old_weight)
    MultiplicityDecreased,
    /// Record removed entirely (old_weight > 0, new_weight <= 0)
    Deleted,
    /// No change
    Unchanged,
}

impl WeightTransition {
    pub fn compute(old_weight: i64, new_weight: i64) -> Self {
        let was_present = old_weight > 0;
        let is_present = new_weight > 0;
        
        match (was_present, is_present) {
            (false, true) => WeightTransition::Inserted,
            (true, false) => WeightTransition::Deleted,
            (true, true) if new_weight > old_weight => WeightTransition::MultiplicityIncreased,
            (true, true) if new_weight < old_weight => WeightTransition::MultiplicityDecreased,
            _ => WeightTransition::Unchanged,
        }
    }
    
    /// For edge management, we only care about presence changes
    pub fn is_membership_change(&self) -> bool {
        matches!(self, WeightTransition::Inserted | WeightTransition::Deleted)
    }
}
```

### Step 2.2: Update Delta struct

**File:** `packages/ssp/src/types.rs`

```rust
/// Delta represents a change to the database
#[derive(Debug, Clone)]
pub struct Delta {
    pub table: SmolStr,
    pub key: SmolStr,
    pub weight: i64,
    pub content_changed: bool,
}

impl Delta {
    /// Create delta from database operation
    pub fn from_operation(table: SmolStr, key: SmolStr, op: Operation) -> Self {
        let (weight, content_changed) = match op {
            Operation::Create => (1, false),
            Operation::Update => (0, true),  // Weight 0 = no membership change
            Operation::Delete => (-1, false),
        };
        
        Self { table, key, weight, content_changed }
    }
}
```

### Step 2.3: Update ZSet operations

**File:** `packages/ssp/src/types.rs`

```rust
/// ZSet operations following DBSP semantics
pub trait ZSetOps {
    /// Add delta to ZSet: result[k] = self[k] + delta[k]
    fn add_delta(&mut self, delta: &ZSet);
    
    /// Compute difference: result[k] = other[k] - self[k]
    fn diff(&self, other: &ZSet) -> ZSet;
    
    /// Check if record is present (weight > 0)
    fn is_present(&self, key: &str) -> bool;
    
    /// Get records that transitioned to/from presence
    fn membership_changes(&self, other: &ZSet) -> Vec<(SmolStr, WeightTransition)>;
}

impl ZSetOps for ZSet {
    fn add_delta(&mut self, delta: &ZSet) {
        for (key, &weight) in delta {
            let entry = self.entry(key.clone()).or_insert(0);
            *entry += weight;
            // Clean up zero weights
            if *entry == 0 {
                self.remove(key);
            }
        }
    }
    
    fn diff(&self, other: &ZSet) -> ZSet {
        let mut result = FastMap::default();
        
        // Records in other
        for (key, &new_weight) in other {
            let old_weight = self.get(key).copied().unwrap_or(0);
            let diff = new_weight - old_weight;
            if diff != 0 {
                result.insert(key.clone(), diff);
            }
        }
        
        // Records only in self (removed from other)
        for (key, &old_weight) in self {
            if !other.contains_key(key) {
                result.insert(key.clone(), -old_weight);
            }
        }
        
        result
    }
    
    fn is_present(&self, key: &str) -> bool {
        self.get(key).map(|&w| w > 0).unwrap_or(false)
    }
    
    fn membership_changes(&self, other: &ZSet) -> Vec<(SmolStr, WeightTransition)> {
        let mut changes = Vec::new();
        
        // Check all keys in either set
        let all_keys: std::collections::HashSet<&SmolStr> = 
            self.keys().chain(other.keys()).collect();
        
        for key in all_keys {
            let old_w = self.get(key).copied().unwrap_or(0);
            let new_w = other.get(key).copied().unwrap_or(0);
            let transition = WeightTransition::compute(old_w, new_w);
            
            if transition.is_membership_change() {
                changes.push((key.clone(), transition));
            }
        }
        
        changes
    }
}
```

---

## Phase 3: Fix compute_full_diff for Multiset Semantics (Day 2)

### Step 3.1: Rewrite compute_full_diff

**File:** `packages/ssp/src/view.rs`

```rust
/// Compute full diff between current cache and target state
/// 
/// Returns a ZSet delta where:
/// - Positive weights = records added or multiplicity increased
/// - Negative weights = records removed or multiplicity decreased
fn compute_full_diff(&self, db: &Database) -> ZSet {
    // Compute target state
    let mut target_set = self
        .eval_snapshot(&self.plan.root, db, self.params.as_ref())
        .into_owned();
    
    // Expand with subquery results
    self.expand_with_subqueries(&mut target_set, db);

    tracing::debug!(
        target: "ssp::view::delta",
        view_id = %self.plan.id,
        target_set_size = target_set.len(),
        cache_size = self.cache.len(),
        "compute_full_diff: comparing target vs cache"
    );

    // Use ZSetOps::diff for correct DBSP semantics
    let diff = self.cache.diff(&target_set);
    
    tracing::debug!(
        target: "ssp::view::delta",
        view_id = %self.plan.id,
        diff_size = diff.len(),
        positive_weights = diff.values().filter(|&&w| w > 0).count(),
        negative_weights = diff.values().filter(|&&w| w < 0).count(),
        "compute_full_diff: result"
    );
    
    diff
}
```

### Step 3.2: Update apply_cache_delta

**File:** `packages/ssp/src/view.rs`

```rust
/// Apply delta to cache using DBSP semantics
fn apply_cache_delta(&mut self, delta: &ZSet) {
    use crate::types::ZSetOps;
    
    let cache_before = self.cache.len();
    self.cache.add_delta(delta);
    
    tracing::debug!(
        target: "ssp::view::cache",
        view_id = %self.plan.id,
        cache_before = cache_before,
        cache_after = self.cache.len(),
        delta_size = delta.len(),
        "Cache updated with delta"
    );
}
```

---

## Phase 4: Update categorize_changes for Membership Tracking (Day 2-3)

### Problem

For edge management, we need to track **membership changes** (present → absent, absent → present), not raw weight changes.

### Step 4.1: Rewrite categorize_changes

**File:** `packages/ssp/src/view.rs`

```rust
/// Categorize changes into membership events for edge management
/// 
/// This converts DBSP weight deltas into discrete events:
/// - `additions`: Records that newly appear in the view (weight was ≤0, now >0)
/// - `removals`: Records that leave the view (weight was >0, now ≤0)
/// - `updates`: Records whose content changed but membership unchanged
fn categorize_changes(
    &self,
    view_delta: &ZSet,
    updated_record_ids: &[SmolStr],
) -> (Vec<SmolStr>, Vec<SmolStr>, Vec<SmolStr>) {
    use crate::types::WeightTransition;
    
    let mut additions: Vec<SmolStr> = Vec::new();
    let mut removals: Vec<SmolStr> = Vec::new();
    
    // Process weight changes
    for (key, &weight_delta) in view_delta {
        // Get old weight from cache (BEFORE applying delta)
        // Note: delta hasn't been applied yet at this point!
        let old_weight = self.cache.get(key).copied().unwrap_or(0);
        let new_weight = old_weight + weight_delta;
        
        let transition = WeightTransition::compute(old_weight, new_weight);
        
        let stripped_key = Self::strip_table_prefix_smol(key);
        
        match transition {
            WeightTransition::Inserted => {
                additions.push(stripped_key);
            }
            WeightTransition::Deleted => {
                removals.push(stripped_key);
            }
            _ => {
                // Multiplicity changes don't affect edges
            }
        }
    }
    
    // Build removal set for filtering updates
    let removal_set_unstripped: std::collections::HashSet<&str> = 
        view_delta.iter()
            .filter(|(key, &weight)| {
                let old_w = self.cache.get(*key).copied().unwrap_or(0);
                let new_w = old_w + weight;
                old_w > 0 && new_w <= 0
            })
            .map(|(k, _)| k.as_str())
            .collect();

    // Updates: records in updated_record_ids that are NOT being removed
    // AND are currently present in the view
    let updates: Vec<SmolStr> = updated_record_ids
        .iter()
        .filter(|id| {
            !removal_set_unstripped.contains(id.as_str()) &&
            self.cache.get(*id).map(|&w| w > 0).unwrap_or(false)
        })
        .map(|id| Self::strip_table_prefix_smol(id))
        .collect();

    tracing::debug!(
        target: "ssp::view::categorize",
        view_id = %self.plan.id,
        additions_count = additions.len(),
        removals_count = removals.len(),
        updates_count = updates.len(),
        "Categorized membership changes"
    );

    (additions, removals, updates)
}
```

### Step 4.2: Update process_batch to call categorize_changes BEFORE apply_cache_delta

**File:** `packages/ssp/src/view.rs`

This is **critical** - we need to categorize changes based on the old cache state, then apply the delta.

```rust
pub fn process_batch(
    &mut self,
    batch_deltas: &BatchDeltas,
    db: &Database,
) -> Option<ViewUpdate> {
    let is_first_run = self.last_hash.is_empty();

    tracing::debug!(
        target: "ssp::view::process_batch",
        view_id = %self.plan.id,
        is_first_run = is_first_run,
        cache_size_before = self.cache.len(),
        "Starting process_batch"
    );

    // Step 1: Compute view delta
    let view_delta = self.compute_view_delta(&batch_deltas.membership, db, is_first_run);
    let updated_record_ids = self.get_content_updates_in_view(batch_deltas);
    
    // Early return if no changes
    if view_delta.is_empty() && !is_first_run && updated_record_ids.is_empty() {
        tracing::debug!(
            target: "ssp::view::process_batch",
            view_id = %self.plan.id,
            "No changes detected, returning None"
        );
        return None;
    }

    // Step 2: Categorize changes BEFORE applying delta
    // This is critical for correct membership tracking!
    let (additions, removals, updates) = self.categorize_changes(&view_delta, &updated_record_ids);

    // Step 3: Apply delta to cache AFTER categorizing
    self.apply_cache_delta(&view_delta);

    // Step 4: Build result data from updated cache
    let result_data = self.build_result_data();

    tracing::debug!(
        target: "ssp::view::process_batch",
        view_id = %self.plan.id,
        cache_size_after = self.cache.len(),
        result_data_count = result_data.len(),
        additions = additions.len(),
        removals = removals.len(),
        updates = updates.len(),
        "Processed batch"
    );

    // Step 5: Build update
    use super::update::{build_update, compute_flat_hash, RawViewResult, ViewDelta};

    let view_delta_struct = Some(ViewDelta {
        additions,
        removals,
        updates,
    });

    // Compute hash if needed
    let pre_hash = if matches!(self.format, ViewResultFormat::Streaming) {
        Some(compute_flat_hash(&result_data))
    } else {
        None
    };

    let raw_result = RawViewResult {
        query_id: self.plan.id.clone(),
        records: result_data,
        delta: view_delta_struct,
    };

    let update = build_update(raw_result, self.format);

    // Extract hash and check for changes
    let hash = match &update {
        ViewUpdate::Flat(flat) | ViewUpdate::Tree(flat) => flat.result_hash.clone(),
        ViewUpdate::Streaming(_) => pre_hash.unwrap_or_default(),
    };

    let has_changes = match &update {
        ViewUpdate::Streaming(s) => !s.records.is_empty(),
        _ => hash != self.last_hash,
    };

    if has_changes {
        self.last_hash = hash;
        return Some(update);
    }

    None
}
```

---

## Phase 5: Fix expand_with_subqueries for Proper Weight Handling (Day 3)

### Problem

Subqueries can reference the same record multiple times. We need to track this correctly.

### Step 5.1: Rewrite expand_with_subqueries

**File:** `packages/ssp/src/view.rs`

```rust
/// Expand target set to include records referenced by subqueries
/// 
/// For each parent record in the target set, evaluate subqueries and add
/// referenced records with appropriate weights.
/// 
/// Weight semantics:
/// - If parent has weight W and subquery returns record R with weight 1,
///   then R gets weight += W in the result
/// - This ensures correct delta propagation when parents are added/removed
fn expand_with_subqueries(&self, target_set: &mut ZSet, db: &Database) {
    if !self.has_subqueries_cached {
        return;
    }

    // Collect subquery results with parent weights
    let mut subquery_additions: ZSet = FastMap::default();
    
    // Get parent records (before modification)
    let parent_records: Vec<(SmolStr, i64)> = target_set
        .iter()
        .map(|(k, &w)| (k.clone(), w))
        .collect();
    
    for (parent_key, parent_weight) in parent_records {
        if parent_weight <= 0 {
            continue;  // Skip non-present parents
        }
        
        // Get parent record data
        let parent_data = match db.get_row_by_key(&parent_key) {
            Some(data) => data,
            None => continue,
        };
        
        // Evaluate subqueries for this parent
        let subquery_results = self.evaluate_subqueries_for_parent(
            &self.plan.root,
            &parent_data,
            db,
        );
        
        // Add subquery results with weight = parent_weight
        // This ensures correct propagation: if parent appears twice,
        // its subquery results also appear twice
        for (subquery_key, subquery_weight) in subquery_results {
            *subquery_additions.entry(subquery_key).or_insert(0) += 
                parent_weight * subquery_weight;
        }
    }
    
    // Merge subquery results into target set
    for (key, weight) in subquery_additions {
        *target_set.entry(key).or_insert(0) += weight;
    }
    
    tracing::debug!(
        target: "ssp::view::subquery",
        view_id = %self.plan.id,
        target_set_size_after = target_set.len(),
        "Expanded with subqueries"
    );
}

/// Evaluate subqueries for a parent record
fn evaluate_subqueries_for_parent(
    &self,
    op: &Operator,
    parent_context: &SpookyValue,
    db: &Database,
) -> ZSet {
    let mut results = FastMap::default();
    
    if let Operator::Project { input, projections } = op {
        // First, recurse into input
        let input_results = self.evaluate_subqueries_for_parent(input, parent_context, db);
        for (k, w) in input_results {
            *results.entry(k).or_insert(0) += w;
        }
        
        // Then evaluate projections
        for proj in projections {
            if let Projection::Subquery { operator, .. } = proj {
                // Evaluate subquery with parent context
                let subquery_result = self.eval_snapshot(operator, db, Some(parent_context));
                
                for (key, &weight) in subquery_result.iter() {
                    *results.entry(key.clone()).or_insert(0) += weight;
                    
                    // Recursively expand nested subqueries
                    if let Some(record) = db.get_row_by_key(key) {
                        let nested = self.evaluate_subqueries_for_parent(operator, &record, db);
                        for (nested_key, nested_weight) in nested {
                            *results.entry(nested_key).or_insert(0) += nested_weight;
                        }
                    }
                }
            }
        }
    }
    
    results
}
```

---

## Phase 6: Add Tests for Multiset Semantics (Day 3-4)

### Step 6.1: Unit tests for ZSetOps

**File:** `packages/ssp/src/types.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    fn make_zset(items: &[(&str, i64)]) -> ZSet {
        items.iter().map(|(k, w)| (SmolStr::new(*k), *w)).collect()
    }
    
    #[test]
    fn test_zset_diff_simple() {
        let old = make_zset(&[("a", 1), ("b", 1)]);
        let new = make_zset(&[("b", 1), ("c", 1)]);
        
        let diff = old.diff(&new);
        
        assert_eq!(diff.get("a").copied(), Some(-1));  // Removed
        assert_eq!(diff.get("b").copied(), None);       // Unchanged
        assert_eq!(diff.get("c").copied(), Some(1));    // Added
    }
    
    #[test]
    fn test_zset_diff_multiplicity() {
        let old = make_zset(&[("a", 1)]);
        let new = make_zset(&[("a", 3)]);
        
        let diff = old.diff(&new);
        
        assert_eq!(diff.get("a").copied(), Some(2));  // Multiplicity increased by 2
    }
    
    #[test]
    fn test_weight_transition_inserted() {
        assert_eq!(
            WeightTransition::compute(0, 1),
            WeightTransition::Inserted
        );
        assert_eq!(
            WeightTransition::compute(-1, 1),
            WeightTransition::Inserted
        );
    }
    
    #[test]
    fn test_weight_transition_deleted() {
        assert_eq!(
            WeightTransition::compute(1, 0),
            WeightTransition::Deleted
        );
        assert_eq!(
            WeightTransition::compute(2, -1),
            WeightTransition::Deleted
        );
    }
    
    #[test]
    fn test_weight_transition_multiplicity() {
        assert_eq!(
            WeightTransition::compute(1, 2),
            WeightTransition::MultiplicityIncreased
        );
        assert_eq!(
            WeightTransition::compute(3, 1),
            WeightTransition::MultiplicityDecreased
        );
    }
    
    #[test]
    fn test_membership_changes() {
        let old = make_zset(&[("a", 1), ("b", 2)]);
        let new = make_zset(&[("b", 3), ("c", 1)]);
        
        let changes = old.membership_changes(&new);
        
        // a: removed (1 -> 0)
        // b: multiplicity change only (2 -> 3), NOT a membership change
        // c: inserted (0 -> 1)
        
        assert_eq!(changes.len(), 2);
        assert!(changes.iter().any(|(k, t)| k == "a" && *t == WeightTransition::Deleted));
        assert!(changes.iter().any(|(k, t)| k == "c" && *t == WeightTransition::Inserted));
        // b should NOT be in the list
        assert!(!changes.iter().any(|(k, _)| k == "b"));
    }
}
```

### Step 6.2: Integration tests for subquery weight propagation

**File:** `packages/ssp/src/view.rs` (test module)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    /// Test: When two threads reference the same user, user should have weight 2
    #[test]
    fn test_subquery_weight_accumulation() {
        // Setup database
        let mut db = Database::new();
        
        // Create user
        let users = db.ensure_table("user");
        users.rows.insert(SmolStr::new("user:1"), SpookyValue::Object(/* user data */));
        users.zset.insert(SmolStr::new("user:1"), 1);
        
        // Create two threads, both referencing user:1
        let threads = db.ensure_table("thread");
        threads.rows.insert(SmolStr::new("thread:1"), SpookyValue::Object(/* author: user:1 */));
        threads.rows.insert(SmolStr::new("thread:2"), SpookyValue::Object(/* author: user:1 */));
        threads.zset.insert(SmolStr::new("thread:1"), 1);
        threads.zset.insert(SmolStr::new("thread:2"), 1);
        
        // Create view: SELECT *, (SELECT * FROM user WHERE id=$parent.author) FROM thread
        let plan = /* ... */;
        let mut view = View::new(plan, None, Some(ViewResultFormat::Streaming));
        
        // Process initial batch
        let update = view.process_batch(&BatchDeltas::new(), &db);
        
        // Verify weights
        assert_eq!(view.cache.get("thread:1").copied(), Some(1));
        assert_eq!(view.cache.get("thread:2").copied(), Some(1));
        assert_eq!(view.cache.get("user:1").copied(), Some(2));  // Referenced twice!
        
        // Verify only one "Created" event for user (first appearance)
        if let Some(ViewUpdate::Streaming(s)) = update {
            let user_creates: Vec<_> = s.records.iter()
                .filter(|r| r.id == "user:1" && r.event == DeltaEvent::Created)
                .collect();
            assert_eq!(user_creates.len(), 1);  // Only ONE create event
        }
    }
    
    /// Test: When one thread is deleted, user weight decreases but user stays
    #[test]
    fn test_subquery_weight_decrease_no_removal() {
        // ... setup with user weight = 2 ...
        
        // Delete thread:1
        let delta = Delta::from_operation(
            SmolStr::new("thread"),
            SmolStr::new("thread:1"),
            Operation::Delete,
        );
        
        let update = view.process_delta(&delta, &db);
        
        // User weight should be 1 now (still present)
        assert_eq!(view.cache.get("user:1").copied(), Some(1));
        
        // NO delete event for user (still in view)
        if let Some(ViewUpdate::Streaming(s)) = update {
            assert!(!s.records.iter().any(|r| r.id == "user:1" && r.event == DeltaEvent::Deleted));
        }
    }
    
    /// Test: When last thread referencing user is deleted, user is removed
    #[test]
    fn test_subquery_weight_to_zero_removal() {
        // ... setup with only one thread referencing user (weight = 1) ...
        
        // Delete the only thread
        let delta = Delta::from_operation(
            SmolStr::new("thread"),
            SmolStr::new("thread:1"),
            Operation::Delete,
        );
        
        let update = view.process_delta(&delta, &db);
        
        // User should be removed
        assert_eq!(view.cache.get("user:1").copied(), None);  // or Some(0)
        
        // DELETE event for user
        if let Some(ViewUpdate::Streaming(s)) = update {
            assert!(s.records.iter().any(|r| r.id == "user:1" && r.event == DeltaEvent::Deleted));
        }
    }
}
```

---

## Phase 7: Update Edge Management (Day 4)

### Problem

Edge operations should only happen on membership changes, not multiplicity changes.

### Step 7.1: Update DeltaEvent documentation

**File:** `packages/ssp/src/update.rs`

```rust
/// Delta event types for streaming updates
/// 
/// These represent MEMBERSHIP changes, not multiplicity changes:
/// - Created: Record newly appears in view (weight went from ≤0 to >0)
/// - Updated: Record content changed (weight unchanged, still present)
/// - Deleted: Record removed from view (weight went from >0 to ≤0)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeltaEvent {
    Created,
    Updated,
    Deleted,
}
```

### Step 7.2: Verify build_update uses membership changes

**File:** `packages/ssp/src/update.rs`

```rust
pub fn build_update(raw: RawViewResult, format: ViewResultFormat) -> ViewUpdate {
    match format {
        // ... other formats ...
        
        ViewResultFormat::Streaming => {
            let mut delta_records = Vec::new();

            if let Some(delta) = raw.delta {
                // These are already membership changes (not raw weight deltas)
                for id in delta.additions {
                    delta_records.push(DeltaRecord {
                        id,
                        event: DeltaEvent::Created,
                    });
                }

                for id in delta.removals {
                    delta_records.push(DeltaRecord {
                        id,
                        event: DeltaEvent::Deleted,
                    });
                }

                for id in delta.updates {
                    delta_records.push(DeltaRecord {
                        id,
                        event: DeltaEvent::Updated,
                    });
                }
            }

            ViewUpdate::Streaming(StreamingUpdate {
                view_id: raw.query_id,
                records: delta_records,
            })
        }
    }
}
```

---

## Phase 8: Logging and Debugging (Day 4)

### Step 8.1: Add comprehensive logging for weight tracking

**File:** `packages/ssp/src/view.rs`

```rust
// In categorize_changes, log weight transitions:
for (key, &weight_delta) in view_delta {
    let old_weight = self.cache.get(key).copied().unwrap_or(0);
    let new_weight = old_weight + weight_delta;
    let transition = WeightTransition::compute(old_weight, new_weight);
    
    tracing::trace!(
        target: "ssp::view::weights",
        view_id = %self.plan.id,
        key = %key,
        old_weight = old_weight,
        new_weight = new_weight,
        weight_delta = weight_delta,
        transition = ?transition,
        "Weight transition"
    );
    
    // ... rest of logic
}
```

### Step 8.2: Add debug endpoint to inspect view state

**File:** `app/ssp/src/lib.rs`

```rust
async fn debug_view_handler(
    State(state): State<AppState>,
    Path(view_id): Path<String>,
) -> impl IntoResponse {
    let circuit = state.processor.read().await;
    
    if let Some(view) = circuit.views.iter().find(|v| v.plan.id == view_id) {
        let cache_summary: Vec<_> = view.cache.iter()
            .map(|(k, &w)| json!({ "key": k, "weight": w }))
            .collect();
        
        Json(json!({
            "view_id": view_id,
            "cache_size": view.cache.len(),
            "last_hash": view.last_hash,
            "cache": cache_summary,
        }))
    } else {
        Json(json!({ "error": "View not found" }))
    }
}
```

---

## Phase 9: Documentation (Day 5)

### Step 9.1: Add architecture documentation

**File:** `packages/ssp/README.md`

```markdown
# SSP - Spooky Stream Processor

## DBSP Semantics

SSP uses DBSP (Database Stream Processing) semantics for incremental view maintenance.

### ZSet (Z-Set)

A ZSet is a multiset represented as a map from elements to integer weights:
- `weight > 0`: Element present with multiplicity `weight`
- `weight = 0`: Element not present (removed from map)
- `weight < 0`: Element "negatively present" (used in deltas)

### Delta Propagation

Changes propagate through the query plan:
```
Input Delta → Filter → Project → Output Delta
    ΔI     →   Δ(Filter(I))  →  ΔO
```

### Membership vs Multiplicity

For edge management, we distinguish:
- **Membership change**: Record enters or leaves the view (weight crosses 0)
- **Multiplicity change**: Record's count changes but it remains in the view

Only membership changes trigger edge creation/deletion.

### Example

```
Thread 1 → User A (weight 1)
Thread 2 → User A (weight 1)
─────────────────────────────
User A total weight = 2

Delete Thread 1:
User A weight: 2 → 1 (multiplicity decrease, NOT a removal)

Delete Thread 2:
User A weight: 1 → 0 (membership change: REMOVED)
```
```

---

## Summary Checklist

### Phase 1: Fix Double Prefix (Day 1)
- [ ] Add `make_zset_key` helper function
- [ ] Add `parse_zset_key` helper function
- [ ] Update all call sites to use `make_zset_key`
- [ ] Add unit tests for key functions

### Phase 2: Weight Tracking (Day 1-2)
- [ ] Add `WeightTransition` enum
- [ ] Add `ZSetOps` trait with `add_delta`, `diff`, `membership_changes`
- [ ] Implement `ZSetOps` for `ZSet`
- [ ] Add unit tests for ZSet operations

### Phase 3: Fix compute_full_diff (Day 2)
- [ ] Rewrite to use `ZSetOps::diff`
- [ ] Add logging for debugging

### Phase 4: Fix categorize_changes (Day 2-3)
- [ ] Rewrite to use `WeightTransition`
- [ ] Ensure it runs BEFORE `apply_cache_delta`
- [ ] Update `process_batch` ordering

### Phase 5: Fix expand_with_subqueries (Day 3)
- [ ] Implement proper weight propagation
- [ ] Handle nested subqueries

### Phase 6: Add Tests (Day 3-4)
- [ ] Unit tests for `ZSetOps`
- [ ] Unit tests for `WeightTransition`
- [ ] Integration tests for subquery weight propagation
- [ ] Integration tests for membership changes

### Phase 7: Update Edge Management (Day 4)
- [ ] Verify edge operations only on membership changes
- [ ] Update documentation for `DeltaEvent`

### Phase 8: Logging (Day 4)
- [ ] Add weight transition logging
- [ ] Add debug endpoint

### Phase 9: Documentation (Day 5)
- [ ] Add architecture README
- [ ] Document DBSP semantics
- [ ] Add code comments

---

## Verification Queries

After implementation, verify with these scenarios:

```sql
-- Scenario 1: Two threads, same author
-- Expected: user weight = 2, ONE create edge

-- Scenario 2: Delete one thread
-- Expected: user weight = 1, NO delete edge

-- Scenario 3: Delete second thread  
-- Expected: user weight = 0, ONE delete edge
```

```bash
# Enable trace logging for weight tracking
RUST_LOG=ssp::view::weights=trace cargo run
```