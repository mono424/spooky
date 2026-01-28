# Implementation Plan: SSP View Engine Fixes

## Overview

This plan implements all fixes from CODE-ANALYSIS-VIEW-ENGINE-v2.md, converting the hybrid model to **Pure Membership Model** and fixing all identified issues.

**Estimated Time:** 3-4 days
**Files to Modify:**
- `packages/ssp/src/types/zset.rs` - Membership operations
- `packages/ssp/src/view.rs` - Core view logic
- `packages/ssp/src/circuit.rs` - Deserialization fix
- `packages/ssp/src/eval/filter.rs` - Performance optimization

---

## Phase 1: Foundation - ZSet Membership Operations (Day 1, Morning)

### Step 1.1: Add Membership Trait to zset.rs

**File:** `packages/ssp/src/types/zset.rs`

**Goal:** Add clean membership-based operations alongside existing ZSetOps.

```rust
// Add after existing ZSetOps trait (around line 140)

/// Pure Membership operations for ZSet
/// 
/// In membership model:
/// - weight > 0 means "present" (normalized to 1)
/// - weight <= 0 means "absent" (removed from map)
/// 
/// This is simpler than full DBSP and matches edge system requirements:
/// - One edge per (view, record) pair
/// - No multiplicity tracking needed
pub trait ZSetMembershipOps {
    /// Check if record is a member (weight > 0)
    fn is_member(&self, key: &str) -> bool;
    
    /// Add record as member (sets weight to 1)
    fn add_member(&mut self, key: SmolStr);
    
    /// Remove record from membership
    fn remove_member(&mut self, key: &str) -> bool;
    
    /// Apply delta with membership normalization
    /// All positive weights become 1, zero/negative weights remove the key
    fn apply_membership_delta(&mut self, delta: &ZSet);
    
    /// Compute membership changes from self to target
    /// Returns (keys_added, keys_removed)
    fn membership_diff(&self, target: &ZSet) -> (Vec<SmolStr>, Vec<SmolStr>);
    
    /// Normalize all weights to membership (1 if present, remove if not)
    fn normalize_to_membership(&mut self);
    
    /// Get count of members (weight > 0)
    fn member_count(&self) -> usize;
}

impl ZSetMembershipOps for ZSet {
    #[inline]
    fn is_member(&self, key: &str) -> bool {
        self.get(key).map(|&w| w > 0).unwrap_or(false)
    }
    
    #[inline]
    fn add_member(&mut self, key: SmolStr) {
        self.insert(key, 1);
    }
    
    #[inline]
    fn remove_member(&mut self, key: &str) -> bool {
        self.remove(key).is_some()
    }
    
    fn apply_membership_delta(&mut self, delta: &ZSet) {
        for (key, &weight_delta) in delta {
            let old_weight = self.get(key).copied().unwrap_or(0);
            let new_weight = old_weight + weight_delta;
            
            // Membership normalization: present = 1, absent = removed
            if new_weight > 0 {
                self.insert(key.clone(), 1);
            } else {
                self.remove(key);
            }
        }
    }
    
    fn membership_diff(&self, target: &ZSet) -> (Vec<SmolStr>, Vec<SmolStr>) {
        let mut additions = Vec::new();
        let mut removals = Vec::new();
        
        // Records in target but not in self (or not present in self)
        for (key, &weight) in target {
            if weight > 0 && !self.is_member(key) {
                additions.push(key.clone());
            }
        }
        
        // Records in self but not in target (or not present in target)
        for (key, &weight) in self.iter() {
            if weight > 0 {
                let target_present = target.get(key).map(|&w| w > 0).unwrap_or(false);
                if !target_present {
                    removals.push(key.clone());
                }
            }
        }
        
        (additions, removals)
    }
    
    fn normalize_to_membership(&mut self) {
        // Collect keys to modify (can't mutate while iterating)
        let keys_to_normalize: Vec<_> = self.iter()
            .filter(|(_, &w)| w > 1)
            .map(|(k, _)| k.clone())
            .collect();
        
        let keys_to_remove: Vec<_> = self.iter()
            .filter(|(_, &w)| w <= 0)
            .map(|(k, _)| k.clone())
            .collect();
        
        for key in keys_to_normalize {
            self.insert(key, 1);
        }
        
        for key in keys_to_remove {
            self.remove(&key);
        }
    }
    
    fn member_count(&self) -> usize {
        self.values().filter(|&&w| w > 0).count()
    }
}
```

### Step 1.2: Add Tests for Membership Operations

**File:** `packages/ssp/src/types/zset.rs` (in test module)

```rust
// Add to existing #[cfg(test)] mod tests

#[test]
fn test_membership_is_member() {
    let mut z = make_zset(&[("a", 1), ("b", 2), ("c", 0), ("d", -1)]);
    
    assert!(z.is_member("a"));
    assert!(z.is_member("b"));  // weight > 1 still counts as member
    assert!(!z.is_member("c")); // weight 0 = not member
    assert!(!z.is_member("d")); // weight < 0 = not member
    assert!(!z.is_member("e")); // not in map = not member
}

#[test]
fn test_membership_add_remove() {
    let mut z: ZSet = FastMap::default();
    
    z.add_member(SmolStr::new("a"));
    assert_eq!(z.get("a"), Some(&1));
    
    z.add_member(SmolStr::new("a")); // Adding again keeps weight at 1
    assert_eq!(z.get("a"), Some(&1));
    
    assert!(z.remove_member("a"));
    assert!(!z.is_member("a"));
    
    assert!(!z.remove_member("nonexistent"));
}

#[test]
fn test_apply_membership_delta_normalizes() {
    let mut cache = make_zset(&[("a", 1), ("b", 1)]);
    let delta = make_zset(&[("a", 1), ("b", -1), ("c", 5)]);
    
    cache.apply_membership_delta(&delta);
    
    // a: was 1, delta +1 = 2 → normalized to 1
    assert_eq!(cache.get("a"), Some(&1));
    
    // b: was 1, delta -1 = 0 → removed
    assert!(!cache.contains_key("b"));
    
    // c: was 0, delta +5 = 5 → normalized to 1
    assert_eq!(cache.get("c"), Some(&1));
}

#[test]
fn test_membership_diff() {
    let old = make_zset(&[("a", 1), ("b", 1), ("c", 1)]);
    let new = make_zset(&[("b", 1), ("c", 1), ("d", 1)]);
    
    let (additions, removals) = old.membership_diff(&new);
    
    assert_eq!(additions, vec![SmolStr::new("d")]);
    assert_eq!(removals, vec![SmolStr::new("a")]);
}

#[test]
fn test_membership_diff_ignores_weight_changes() {
    let old = make_zset(&[("a", 1)]);
    let new = make_zset(&[("a", 5)]);  // Weight changed but still present
    
    let (additions, removals) = old.membership_diff(&new);
    
    // No membership change - just weight change
    assert!(additions.is_empty());
    assert!(removals.is_empty());
}

#[test]
fn test_normalize_to_membership() {
    let mut z = make_zset(&[("a", 1), ("b", 5), ("c", 0), ("d", -2)]);
    
    z.normalize_to_membership();
    
    assert_eq!(z.get("a"), Some(&1));
    assert_eq!(z.get("b"), Some(&1));  // 5 → 1
    assert!(!z.contains_key("c"));      // 0 → removed
    assert!(!z.contains_key("d"));      // -2 → removed
}
```

### Step 1.3: Export New Trait

**File:** `packages/ssp/src/types/mod.rs` (or wherever types are exported)

```rust
// Add to exports
pub use zset::{ZSetMembershipOps, /* existing exports */};
```

---

## Phase 2: Fix View Deserialization (Day 1, Afternoon)

### Step 2.1: Add Initialization Method to View

**File:** `packages/ssp/src/view.rs`

**Location:** After `View::new()` (around line 71)

```rust
impl View {
    // ... existing new() ...

    /// Initialize cached flags after deserialization
    /// 
    /// IMPORTANT: Call this after deserializing a View from storage!
    /// The cached flags are not serialized to save space, so they must
    /// be recomputed when loading state.
    pub fn initialize_after_deserialize(&mut self) {
        self.has_subqueries_cached = self.plan.root.has_subquery_projections();
        self.referenced_tables_cached = self.plan.root.referenced_tables();
        self.is_simple_scan = matches!(self.plan.root, Operator::Scan { .. });
        self.is_simple_filter = if let Operator::Filter { input, .. } = &self.plan.root {
            matches!(input.as_ref(), Operator::Scan { .. })
        } else {
            false
        };
        
        tracing::debug!(
            target: "ssp::view::init",
            view_id = %self.plan.id,
            has_subqueries = self.has_subqueries_cached,
            is_simple_scan = self.is_simple_scan,
            is_simple_filter = self.is_simple_filter,
            referenced_tables = ?self.referenced_tables_cached,
            "Initialized cached flags after deserialize"
        );
    }
    
    /// Check if cached flags are initialized
    pub fn is_initialized(&self) -> bool {
        // If referenced_tables is empty but plan has operators, not initialized
        // (A valid initialized view with Scan would have at least one table)
        !self.referenced_tables_cached.is_empty() || 
            matches!(self.plan.root, Operator::Scan { .. })
    }
}
```

### Step 2.2: Update Circuit to Initialize Views After Load

**File:** `packages/ssp/src/circuit.rs`

**Location:** In the function that loads circuit state from disk

```rust
// Find the function that loads state (likely in persistence.rs or circuit.rs)
// Add this after deserializing:

impl Circuit {
    /// Load circuit state from storage and initialize all views
    pub fn load_from_storage(path: &str) -> Result<Self, Error> {
        // ... existing deserialization code ...
        let mut circuit: Circuit = serde_json::from_str(&contents)?;
        
        // CRITICAL: Initialize cached flags for all views
        for view in &mut circuit.views {
            view.initialize_after_deserialize();
        }
        
        tracing::info!(
            target: "ssp::circuit::load",
            views_count = circuit.views.len(),
            "Loaded and initialized circuit from storage"
        );
        
        Ok(circuit)
    }
}
```

### Step 2.3: Add Test for Deserialization

**File:** `packages/ssp/src/view.rs` (in test module)

```rust
#[test]
fn test_view_serialization_roundtrip() {
    // Create a view with computed flags
    let plan = QueryPlan {
        id: "test".to_string(),
        root: Operator::Filter {
            input: Box::new(Operator::Scan { table: "users".to_string() }),
            predicate: Predicate::Eq {
                field: Path::new("id"),
                value: serde_json::json!("user:1"),
            },
        },
    };
    let view = View::new(plan, None, Some(ViewResultFormat::Streaming));
    
    // Verify flags are set
    assert!(view.is_simple_filter);
    assert!(!view.is_simple_scan);
    assert_eq!(view.referenced_tables_cached, vec!["users"]);
    
    // Serialize
    let json = serde_json::to_string(&view).unwrap();
    
    // Deserialize
    let mut loaded: View = serde_json::from_str(&json).unwrap();
    
    // Flags should be default (false/empty) before initialization
    assert!(!loaded.is_simple_filter);
    assert!(loaded.referenced_tables_cached.is_empty());
    
    // Initialize
    loaded.initialize_after_deserialize();
    
    // Now flags should match original
    assert!(loaded.is_simple_filter);
    assert!(!loaded.is_simple_scan);
    assert_eq!(loaded.referenced_tables_cached, vec!["users"]);
}
```

---

## Phase 3: Fix Core View Logic for Membership Model (Day 2)

### Step 3.1: Update `apply_cache_delta` for Membership

**File:** `packages/ssp/src/view.rs`

**Location:** Replace existing `apply_cache_delta` (around line 722)

```rust
/// Apply delta to cache using MEMBERSHIP semantics
/// 
/// Key difference from DBSP:
/// - Weights are normalized to 1 (present) or removed (absent)
/// - This ensures one edge per (view, record) pair
fn apply_cache_delta(&mut self, delta: &ZSet) {
    use crate::engine::types::ZSetMembershipOps;
    
    let cache_before = self.cache.len();
    
    // Use membership-aware delta application
    self.cache.apply_membership_delta(delta);
    
    tracing::debug!(
        target: "ssp::view::cache",
        view_id = %self.plan.id,
        cache_before = cache_before,
        cache_after = self.cache.len(),
        delta_size = delta.len(),
        "Cache updated with membership delta"
    );
}
```

### Step 3.2: Update `expand_with_subqueries` to Normalize Weights

**File:** `packages/ssp/src/view.rs`

**Location:** Update existing function (around line 540)

```rust
/// Expand target set with subquery results
/// 
/// MEMBERSHIP MODEL: After expansion, all weights are normalized to 1.
/// This ensures that even if a user is referenced by 10 threads,
/// we only create ONE edge to that user.
fn expand_with_subqueries(&self, target_set: &mut ZSet, db: &Database) {
    use crate::engine::types::ZSetMembershipOps;
    
    if !self.has_subqueries() {
        return;
    }

    // Collect subquery results
    let mut subquery_additions: ZSet = FastMap::default();

    let parent_records: Vec<(SmolStr, i64)> = target_set
        .iter()
        .filter(|(_, &w)| w > 0)  // Only process present records
        .map(|(k, &w)| (k.clone(), w))
        .collect();

    for (parent_key, _parent_weight) in parent_records {
        // Note: We ignore parent_weight for membership model
        // Each parent contributes its subquery results once
        
        let parent_data = match self.get_row_value(&parent_key, db) {
            Some(data) => data,
            None => continue,
        };

        let subquery_results = self.evaluate_subqueries_for_parent(
            &self.plan.root,
            parent_data,
            db,
        );

        // For membership: just mark as present (weight = 1)
        for (subquery_key, subquery_weight) in subquery_results {
            if subquery_weight > 0 {
                subquery_additions.add_member(subquery_key);
            }
        }
    }

    // Merge subquery results (membership: weight = 1)
    for (key, _) in subquery_additions {
        target_set.add_member(key);
    }

    // Ensure entire target set is normalized
    target_set.normalize_to_membership();

    tracing::debug!(
        target: "ssp::view::subquery",
        view_id = %self.plan.id,
        target_set_size = target_set.len(),
        "Expanded and normalized subqueries"
    );
}
```

### Step 3.3: Simplify `categorize_changes` for Membership

**File:** `packages/ssp/src/view.rs`

**Location:** Replace existing function (around line 737)

```rust
/// Categorize changes for edge operations
/// 
/// MEMBERSHIP MODEL simplification:
/// - Additions: records entering the view (not in cache, positive delta)
/// - Removals: records leaving the view (in cache, will be removed)
/// - Updates: records staying in view with content changes
fn categorize_changes(
    &self,
    view_delta: &ZSet,
    content_update_keys: &[SmolStr],
) -> (Vec<SmolStr>, Vec<SmolStr>, Vec<SmolStr>) {
    use crate::engine::types::ZSetMembershipOps;
    
    let mut additions = Vec::new();
    let mut removals = Vec::new();

    for (key, &weight_delta) in view_delta {
        let is_currently_member = self.cache.is_member(key);
        let will_be_member = {
            let old_weight = self.cache.get(key).copied().unwrap_or(0);
            old_weight + weight_delta > 0
        };

        match (is_currently_member, will_be_member) {
            (false, true) => {
                // Entering view
                additions.push(key.clone());
                tracing::trace!(
                    target: "ssp::view::membership",
                    view_id = %self.plan.id,
                    key = %key,
                    "Record ENTERING view"
                );
            }
            (true, false) => {
                // Leaving view
                removals.push(key.clone());
                tracing::trace!(
                    target: "ssp::view::membership",
                    view_id = %self.plan.id,
                    key = %key,
                    "Record LEAVING view"
                );
            }
            _ => {
                // No membership change (staying in or staying out)
            }
        }
    }

    // Content updates: keys that are members AND have content changes AND not leaving
    let removal_set: std::collections::HashSet<&str> = 
        removals.iter().map(|s| s.as_str()).collect();
    
    let updates: Vec<SmolStr> = content_update_keys
        .iter()
        .filter(|key| {
            self.cache.is_member(key) && !removal_set.contains(key.as_str())
        })
        .cloned()
        .collect();

    tracing::debug!(
        target: "ssp::view::categorize",
        view_id = %self.plan.id,
        additions = additions.len(),
        removals = removals.len(),
        updates = updates.len(),
        "Categorized membership changes"
    );

    (additions, removals, updates)
}
```

### Step 3.4: Update `compute_full_diff` for Membership

**File:** `packages/ssp/src/view.rs`

**Location:** Replace existing function (around line 689)

```rust
/// Compute full diff using membership semantics
fn compute_full_diff(&self, db: &Database) -> ZSet {
    use crate::engine::types::ZSetMembershipOps;
    
    // Compute target state
    let mut target_set = self
        .eval_snapshot(&self.plan.root, db, self.params.as_ref())
        .into_owned();
    
    // Expand with subqueries (will normalize weights)
    self.expand_with_subqueries(&mut target_set, db);
    
    // Ensure target is normalized to membership
    target_set.normalize_to_membership();

    tracing::debug!(
        target: "ssp::view::delta",
        view_id = %self.plan.id,
        target_set_size = target_set.len(),
        cache_size = self.cache.len(),
        target_sample = ?target_set.keys().take(3).collect::<Vec<_>>(),
        cache_sample = ?self.cache.keys().take(3).collect::<Vec<_>>(),
        "compute_full_diff: membership comparison"
    );

    // Compute membership diff and convert to ZSet delta format
    let (additions, removals) = self.cache.membership_diff(&target_set);
    
    let mut diff = FastMap::default();
    for key in additions {
        diff.insert(key, 1);  // +1 = entering
    }
    for key in removals {
        diff.insert(key, -1); // -1 = leaving
    }
    
    tracing::debug!(
        target: "ssp::view::delta",
        view_id = %self.plan.id,
        diff_additions = diff.values().filter(|&&w| w > 0).count(),
        diff_removals = diff.values().filter(|&&w| w < 0).count(),
        "compute_full_diff: result"
    );
    
    diff
}
```

---

## Phase 4: Fix Correctness Issues (Day 2, Afternoon)

### Step 4.1: Fix Join Hash Collision Verification

**File:** `packages/ssp/src/view.rs`

**Location:** In `eval_snapshot`, Join case (around line 961)

```rust
Operator::Join { left, right, on } => {
    let s_left = self.eval_snapshot(left, db, context);
    let s_right = self.eval_snapshot(right, db, context);
    let mut out = FastMap::default();

    // BUILD: Index right side by join key hash
    let mut right_index: FastMap<u64, Vec<(&SmolStr, &i64, &SpookyValue)>> = FastMap::default();

    for (r_key, r_weight) in s_right.as_ref() {
        if let Some(r_val) = self.get_row_value(r_key.as_str(), db) {
            if let Some(r_field) = resolve_nested_value(Some(r_val), &on.right_field) {
                let hash = hash_spooky_value(r_field);
                // Store the actual field value for equality check
                right_index.entry(hash).or_default().push((r_key, r_weight, r_field));
            }
        }
    }

    // PROBE: Lookup and verify equality
    for (l_key, l_weight) in s_left.as_ref() {
        if let Some(l_val) = self.get_row_value(l_key.as_str(), db) {
            if let Some(l_field) = resolve_nested_value(Some(l_val), &on.left_field) {
                let hash = hash_spooky_value(l_field);

                if let Some(matches) = right_index.get(&hash) {
                    for (_r_key, r_weight, r_field) in matches {
                        // CRITICAL: Verify actual equality, not just hash match
                        if compare_spooky_values(Some(l_field), Some(*r_field)) == Ordering::Equal {
                            let w = l_weight * *r_weight;
                            *out.entry(l_key.clone()).or_insert(0) += w;
                        }
                    }
                }
            }
        }
    }
    Cow::Owned(out)
}
```

### Step 4.2: Fix `get_row_value` Allocation

**File:** `packages/ssp/src/view.rs`

**Location:** Replace existing function (around line 1001)

```rust
/// Get row value from database by ZSet key
/// 
/// OPTIMIZATION: Avoid allocation by trying raw ID first.
/// If your DB consistently uses one format, simplify this.
#[inline]
fn get_row_value<'a>(&self, key: &str, db: &'a Database) -> Option<&'a SpookyValue> {
    let (table_name, id) = parse_zset_key(key)?;
    let table = db.tables.get(table_name)?;
    
    // Fast path: Try raw ID (most common case)
    if let Some(row) = table.rows.get(id) {
        return Some(row);
    }
    
    // Slow path: Try with table prefix
    // TODO: Normalize row key format at ingestion to eliminate this branch
    // For now, use a static buffer pattern to reduce allocations
    
    // Check if the key format matches "table:id" where id doesn't have prefix
    // If so, the row might be stored with the full key
    if !id.contains(':') {
        // ID doesn't have prefix, try reconstructing
        // This allocation is unavoidable without changing ingestion
        let prefixed = format!("{}:{}", table_name, id);
        return table.rows.get(prefixed.as_str());
    }
    
    None
}
```

**Better Long-term Fix:** Normalize at ingestion time in `circuit.rs`:

```rust
// In Table::apply_mutation or wherever records are stored
impl Table {
    pub fn store_row(&mut self, id: &str, value: SpookyValue) {
        // Always store with consistent format (raw ID without table prefix)
        let normalized_id = if let Some((prefix, raw)) = id.split_once(':') {
            if prefix == self.name {
                raw.to_string()
            } else {
                id.to_string()
            }
        } else {
            id.to_string()
        };
        
        self.rows.insert(SmolStr::new(normalized_id), value);
    }
}
```

---

## Phase 5: Performance Optimizations (Day 3)

### Step 5.1: Optimize `build_result_data` for Streaming

**File:** `packages/ssp/src/view.rs`

**Location:** Replace existing function (around line 800)

```rust
/// Build result data from cache
/// 
/// For Streaming mode, sorting is optional since we only emit deltas.
/// For Flat/Tree modes, sorting is required for consistent hashing.
fn build_result_data(&self) -> Vec<SmolStr> {
    let mut result_data: Vec<SmolStr> = self.cache.keys().cloned().collect();
    
    // Only sort if needed for hash consistency
    // Streaming mode doesn't need sorted data for delta emission
    if !matches!(self.format, ViewResultFormat::Streaming) || self.cache.len() > 1 {
        result_data.sort_unstable();
    }
    
    result_data
}

/// Build result data only if needed
/// Returns None for streaming mode when we only need deltas
fn build_result_data_if_needed(&self, need_hash: bool) -> Option<Vec<SmolStr>> {
    if matches!(self.format, ViewResultFormat::Streaming) && !need_hash {
        return None;
    }
    Some(self.build_result_data())
}
```

### Step 5.2: Add `#[inline]` to Hot Path Functions

**File:** `packages/ssp/src/view.rs`

Add `#[inline]` to these functions:

```rust
#[inline]
fn has_subqueries(&self) -> bool {
    self.has_subqueries_cached
}

#[inline]
fn get_row_value<'a>(&self, key: &str, db: &'a Database) -> Option<&'a SpookyValue> {
    // ... existing implementation
}
```

**File:** `packages/ssp/src/types/zset.rs`

Already added `#[inline]` to `is_member`, `add_member`, `remove_member` in Step 1.1.

### Step 5.3: Optimize `extract_number_column` to Avoid Cloning

**File:** `packages/ssp/src/eval/filter.rs`

**Location:** Replace existing function (around line 92)

```rust
/// Extract numeric column with lazy key cloning
/// 
/// Returns indices of passing records instead of cloning keys upfront.
/// Keys are only cloned for records that pass the filter.
pub fn filter_numeric_lazy<'a>(
    zset: &'a ZSet,
    path: &Path,
    target: f64,
    op: NumericOp,
    db: &Database,
) -> ZSet {
    let mut result = FastMap::default();
    
    for (key, &weight) in zset {
        // Extract numeric value
        let num_val = if let Some((table_name, id)) = key.split_once(':') {
            db.tables.get(table_name)
                .and_then(|t| t.rows.get(id).or_else(|| {
                    t.rows.get(format!("{}:{}", table_name, id).as_str())
                }))
                .and_then(|row| resolve_nested_value(Some(row), path))
                .and_then(|v| if let SpookyValue::Number(n) = v { Some(*n) } else { None })
                .unwrap_or(f64::NAN)
        } else {
            f64::NAN
        };
        
        // Check predicate
        let passes = match op {
            NumericOp::Gt => num_val > target,
            NumericOp::Gte => num_val >= target,
            NumericOp::Lt => num_val < target,
            NumericOp::Lte => num_val <= target,
            NumericOp::Eq => (num_val - target).abs() < f64::EPSILON,
            NumericOp::Neq => (num_val - target).abs() > f64::EPSILON,
        };
        
        if passes {
            // Only clone key if it passes
            result.insert(key.clone(), weight);
        }
    }
    
    result
}

/// Apply numeric filter with SIMD optimization for large datasets
#[inline]
pub fn apply_numeric_filter(upstream: &ZSet, config: &NumericFilterConfig, db: &Database) -> ZSet {
    // For small datasets, use simple loop (avoid SIMD overhead)
    if upstream.len() < 64 {
        return filter_numeric_lazy(upstream, config.path, config.target, config.op, db);
    }
    
    // For larger datasets, use vectorized approach
    let (keys, weights, numbers) = extract_number_column(upstream, config.path, db);
    let passing_indices = filter_f64_batch(&numbers, config.target, config.op);

    let mut out = FastMap::default();
    for idx in passing_indices {
        out.insert(keys[idx].clone(), weights[idx]);
    }
    out
}
```

### Step 5.4: Optimize Subquery Evaluation to Reuse Allocations

**File:** `packages/ssp/src/view.rs`

**Location:** Update `expand_with_subqueries` to use accumulator pattern

```rust
/// Evaluate subqueries for a parent record, accumulating into existing ZSet
fn evaluate_subqueries_for_parent_into(
    &self,
    op: &Operator,
    parent_context: &SpookyValue,
    db: &Database,
    results: &mut ZSet,  // Accumulate instead of returning new ZSet
) {
    match op {
        Operator::Project { input, projections } => {
            // Recurse into input
            self.evaluate_subqueries_for_parent_into(input, parent_context, db, results);

            // Evaluate subquery projections
            for proj in projections {
                if let Projection::Subquery { plan: operator, .. } = proj {
                    let subquery_result = self.eval_snapshot(operator, db, Some(parent_context));

                    for (key, &weight) in subquery_result.iter() {
                        if weight > 0 {
                            // Membership: mark as present
                            results.add_member(key.clone());
                        }

                        // Recursively expand nested subqueries
                        if let Some(record) = self.get_row_value(key, db) {
                            self.evaluate_subqueries_for_parent_into(operator, record, db, results);
                        }
                    }
                }
            }
        }
        Operator::Filter { input, .. } => {
            self.evaluate_subqueries_for_parent_into(input, parent_context, db, results);
        }
        Operator::Limit { input, .. } => {
            self.evaluate_subqueries_for_parent_into(input, parent_context, db, results);
        }
        Operator::Join { left, right, .. } => {
            self.evaluate_subqueries_for_parent_into(left, parent_context, db, results);
            self.evaluate_subqueries_for_parent_into(right, parent_context, db, results);
        }
        Operator::Scan { .. } => {
            // No subqueries in scan
        }
    }
}

// Update expand_with_subqueries to use the new function:
fn expand_with_subqueries(&self, target_set: &mut ZSet, db: &Database) {
    use crate::engine::types::ZSetMembershipOps;
    
    if !self.has_subqueries() {
        return;
    }

    // Reusable accumulator for subquery results
    let mut subquery_results: ZSet = FastMap::default();

    let parent_keys: Vec<SmolStr> = target_set
        .iter()
        .filter(|(_, &w)| w > 0)
        .map(|(k, _)| k.clone())
        .collect();

    for parent_key in parent_keys {
        let parent_data = match self.get_row_value(&parent_key, db) {
            Some(data) => data,
            None => continue,
        };

        // Accumulate into existing ZSet (no allocation per parent)
        self.evaluate_subqueries_for_parent_into(
            &self.plan.root,
            parent_data,
            db,
            &mut subquery_results,
        );
    }

    // Merge results
    for (key, _) in subquery_results {
        target_set.add_member(key);
    }

    // Normalize entire set
    target_set.normalize_to_membership();
}
```

---

## Phase 6: Code Cleanup (Day 3, Afternoon)

### Step 6.1: Extract Magic Strings to Constants

**File:** `packages/ssp/src/view.rs` (top of file)

```rust
// Add at the top of the file, after imports

/// Sort direction constants
mod sort_direction {
    pub const ASC: &str = "ASC";
    pub const DESC: &str = "DESC";
}
```

**Update usage in `eval_snapshot` Limit case:**

```rust
return if ord.direction.eq_ignore_ascii_case(sort_direction::DESC) {
    cmp.reverse()
} else {
    cmp
};
```

### Step 6.2: Extract Complex Match Arms to Methods

**File:** `packages/ssp/src/view.rs`

**Refactor `process_content_update`:**

```rust
/// Handle content-only update (no membership change)
fn process_content_update(&mut self, delta: &Delta, db: &Database) -> Option<ViewUpdate> {
    let is_in_cache = self.cache.is_member(&delta.key);
    let matches_filter = self.record_matches_view(&delta.key, db);
    
    match (is_in_cache, matches_filter) {
        (true, true) => self.emit_content_update(&delta.key),
        (true, false) => self.emit_record_left_view(&delta.key),
        (false, true) => self.emit_record_entered_view(delta, db),
        (false, false) => None,
    }
}

/// Emit update for record that stayed in view with content change
fn emit_content_update(&mut self, key: &SmolStr) -> Option<ViewUpdate> {
    use super::update::{build_update, RawViewResult, ViewDelta};
    
    let result_data = self.build_result_data();
    let view_delta = ViewDelta::updates_only(vec![key.clone()]);
    
    let raw_result = RawViewResult {
        query_id: self.plan.id.clone(),
        records: result_data,
        delta: Some(view_delta),
    };
    
    let update = build_update(raw_result, self.format);
    self.update_hash_if_changed(&update)
}

/// Emit update for record that left the view
fn emit_record_left_view(&mut self, key: &SmolStr) -> Option<ViewUpdate> {
    use super::update::{build_update, RawViewResult, ViewDelta};
    
    self.cache.remove_member(key);
    
    let result_data = self.build_result_data();
    let view_delta = ViewDelta::removals_only(vec![key.clone()]);
    
    let raw_result = RawViewResult {
        query_id: self.plan.id.clone(),
        records: result_data,
        delta: Some(view_delta),
    };
    
    let update = build_update(raw_result, self.format);
    self.update_hash_if_changed(&update)
}

/// Emit update for record that entered the view
fn emit_record_entered_view(&mut self, delta: &Delta, db: &Database) -> Option<ViewUpdate> {
    let addition_delta = Delta {
        table: delta.table.clone(),
        key: delta.key.clone(),
        weight: 1,
        content_changed: false,
    };
    self.process_delta(&addition_delta, db)
}

/// Update hash and return update if changed
fn update_hash_if_changed(&mut self, update: &ViewUpdate) -> Option<ViewUpdate> {
    use super::update::compute_flat_hash;
    
    let hash = match update {
        ViewUpdate::Flat(flat) | ViewUpdate::Tree(flat) => flat.result_hash.clone(),
        ViewUpdate::Streaming(_) => {
            // For streaming, compute from cache
            let data = self.build_result_data();
            compute_flat_hash(&data)
        }
    };
    
    if hash != self.last_hash {
        self.last_hash = hash;
        Some(update.clone())
    } else {
        None
    }
}
```

### Step 6.3: Remove Dead Code

**File:** `packages/ssp/src/types/zset.rs`

Check if `WeightTransition` is still needed. With pure membership model:

```rust
// Keep WeightTransition but simplify if only using Inserted/Deleted:

/// Weight transition for membership tracking
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MembershipChange {
    /// Record entered the view
    Entered,
    /// Record left the view  
    Left,
    /// No membership change
    None,
}

impl MembershipChange {
    #[inline]
    pub fn compute(was_member: bool, is_member: bool) -> Self {
        match (was_member, is_member) {
            (false, true) => MembershipChange::Entered,
            (true, false) => MembershipChange::Left,
            _ => MembershipChange::None,
        }
    }
}
```

Or keep `WeightTransition` for backward compatibility but document that only `Inserted` and `Deleted` are used.

---

## Phase 7: Comprehensive Testing (Day 4)

### Step 7.1: Unit Tests for Membership Model

**File:** `packages/ssp/src/view.rs` (test module)

```rust
#[cfg(test)]
mod membership_tests {
    use super::*;
    
    fn setup_db_with_users(count: usize) -> Database {
        let mut db = Database::new();
        let table = db.ensure_table("user");
        
        for i in 1..=count {
            let id = format!("{}", i);
            table.rows.insert(
                SmolStr::new(&id),
                SpookyValue::Object(/* user data */),
            );
            table.zset.insert(SmolStr::new(format!("user:{}", i)), 1);
        }
        
        db
    }
    
    #[test]
    fn test_membership_single_user_single_view() {
        let db = setup_db_with_users(1);
        
        let plan = QueryPlan {
            id: "view1".to_string(),
            root: Operator::Scan { table: "user".to_string() },
        };
        let mut view = View::new(plan, None, Some(ViewResultFormat::Streaming));
        
        // First run
        let update = view.process_batch(&BatchDeltas::new(), &db);
        
        assert!(update.is_some());
        if let Some(ViewUpdate::Streaming(s)) = update {
            assert_eq!(s.records.len(), 1);
            assert!(matches!(s.records[0].event, DeltaEvent::Created));
            assert_eq!(s.records[0].id.as_str(), "user:1");
        }
        
        // Verify cache has weight 1 (membership normalized)
        assert_eq!(view.cache.get("user:1"), Some(&1));
    }
    
    #[test]
    fn test_membership_user_referenced_by_multiple_threads() {
        let mut db = Database::new();
        
        // Create user
        let users = db.ensure_table("user");
        users.rows.insert(SmolStr::new("1"), SpookyValue::Null);
        users.zset.insert(SmolStr::new("user:1"), 1);
        
        // Create 3 threads all referencing user:1
        let threads = db.ensure_table("thread");
        for i in 1..=3 {
            threads.rows.insert(
                SmolStr::new(&i.to_string()),
                SpookyValue::Object(/* author: user:1 */),
            );
            threads.zset.insert(SmolStr::new(format!("thread:{}", i)), 1);
        }
        
        // View with subquery: SELECT *, (SELECT * FROM user WHERE id=$parent.author) FROM thread
        let plan = QueryPlan {
            id: "threads_with_author".to_string(),
            root: Operator::Project {
                input: Box::new(Operator::Scan { table: "thread".to_string() }),
                projections: vec![
                    Projection::Subquery {
                        alias: "author".to_string(),
                        plan: Box::new(Operator::Filter {
                            input: Box::new(Operator::Scan { table: "user".to_string() }),
                            predicate: Predicate::Eq {
                                field: Path::new("id"),
                                value: json!({"$param": "parent.author"}),
                            },
                        }),
                    },
                ],
            },
        };
        
        let mut view = View::new(plan, None, Some(ViewResultFormat::Streaming));
        
        // First run
        let update = view.process_batch(&BatchDeltas::new(), &db).unwrap();
        
        // Should have 4 records: 3 threads + 1 user (NOT 3 users!)
        if let ViewUpdate::Streaming(s) = update {
            assert_eq!(s.records.len(), 4);
            
            // Count user:1 appearances
            let user_creates: Vec<_> = s.records.iter()
                .filter(|r| r.id == "user:1" && matches!(r.event, DeltaEvent::Created))
                .collect();
            
            // CRITICAL: Only ONE created event for user, even though referenced by 3 threads
            assert_eq!(user_creates.len(), 1, "User should only appear once in Created events");
        }
        
        // Verify cache has user:1 with weight 1 (not 3!)
        assert_eq!(view.cache.get("user:user:1"), Some(&1));
    }
    
    #[test]
    fn test_membership_delete_one_of_multiple_referencing_threads() {
        // Setup: 3 threads referencing same user
        // ... (similar setup as above)
        
        // Delete thread:1
        let mut batch = BatchDeltas::new();
        batch.membership.insert(
            "thread".to_string(),
            make_zset(&[("thread:thread:1", -1)]),
        );
        
        let update = view.process_batch(&batch, &db);
        
        // Thread:1 should be deleted
        // User:1 should NOT be deleted (still referenced by thread:2 and thread:3)
        if let Some(ViewUpdate::Streaming(s)) = update {
            let user_deletes: Vec<_> = s.records.iter()
                .filter(|r| r.id.starts_with("user:") && matches!(r.event, DeltaEvent::Deleted))
                .collect();
            
            assert!(user_deletes.is_empty(), "User should NOT be deleted");
            
            let thread_deletes: Vec<_> = s.records.iter()
                .filter(|r| r.id == "thread:1" && matches!(r.event, DeltaEvent::Deleted))
                .collect();
            
            assert_eq!(thread_deletes.len(), 1);
        }
    }
    
    #[test]
    fn test_membership_delete_all_referencing_threads() {
        // Setup: 1 thread referencing user
        // Delete the thread
        // User should also be deleted from view
        
        // ... implementation
    }
    
    #[test]
    fn test_content_update_bumps_version_not_membership() {
        // Setup: user in view
        // Update user content
        // Should emit Update event, not Created
        
        // ... implementation
    }
}
```

### Step 7.2: Integration Tests

**File:** `packages/ssp/tests/integration_membership.rs`

```rust
//! Integration tests for membership model edge cases

#[tokio::test]
async fn test_full_scenario_signup_update_delete() {
    // Simulates your example:
    // 1. User signs up -> 2 edges created (view1, view2)
    // 2. Another user signs up -> 1 edge created (view2 only)
    // 3. User1 updates profile -> 2 edges updated (version bump)
    // 4. User2 deletes account -> 1 edge deleted
    // 5. View2 unregistered -> remaining edges deleted
}

#[tokio::test]
async fn test_limit_view_edge_churn() {
    // Test that LIMIT views correctly handle edge churn
    // when records enter/leave the limit window
}

#[tokio::test]
async fn test_view_persistence_and_reload() {
    // Test that cached flags are properly restored after deserialize
}
```

---

## Summary Checklist

### Phase 1: Foundation (Day 1 Morning) ✅
- [ ] Add `ZSetMembershipOps` trait to `zset.rs`
- [ ] Add tests for membership operations
- [ ] Export new trait from `types/mod.rs`

### Phase 2: Deserialization Fix (Day 1 Afternoon) ✅
- [ ] Add `initialize_after_deserialize()` to View
- [ ] Update Circuit to call initialization after load
- [ ] Add serialization roundtrip test

### Phase 3: Core View Logic (Day 2) ✅
- [ ] Update `apply_cache_delta()` for membership
- [ ] Update `expand_with_subqueries()` to normalize weights
- [ ] Simplify `categorize_changes()` for membership
- [ ] Update `compute_full_diff()` for membership

### Phase 4: Correctness Fixes (Day 2 Afternoon) ✅
- [ ] Fix Join hash collision verification
- [ ] Fix `get_row_value()` allocation

### Phase 5: Performance (Day 3) ✅
- [ ] Optimize `build_result_data()` for streaming
- [ ] Add `#[inline]` to hot paths
- [ ] Optimize numeric filter to avoid cloning
- [ ] Optimize subquery evaluation with accumulator

### Phase 6: Code Cleanup (Day 3 Afternoon) ✅
- [ ] Extract magic strings to constants
- [ ] Extract complex match arms to methods
- [ ] Remove or simplify unused code

### Phase 7: Testing (Day 4) ✅
- [ ] Unit tests for membership model
- [ ] Integration tests for full scenarios
- [ ] Edge case tests (limits, subqueries, etc.)

---

## Verification Queries

After implementation, verify with these SurrealDB queries:

```sql
-- Check edge count per view
SELECT in AS view, count() AS edge_count 
FROM _spooky_list_ref 
GROUP BY in;

-- Check for duplicate edges (should be 0)
SELECT in, out, count() AS cnt 
FROM _spooky_list_ref 
GROUP BY in, out 
HAVING cnt > 1;

-- Verify user only has 1 edge even if referenced by multiple threads
SELECT out.record_id AS record, count() AS edge_count
FROM _spooky_list_ref
WHERE out.record_id CONTAINS 'user:'
GROUP BY out.record_id;
```

Expected results:
- Each (view, record) pair has exactly 1 edge
- No duplicate edges
- User referenced by N threads still has 1 edge per view