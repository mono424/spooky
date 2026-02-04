use rustc_hash::FxHasher;
use smol_str::SmolStr;
use std::hash::BuildHasherDefault;

pub type Weight = i64;
pub type RowKey = SmolStr;
pub type FastMap<K, V> = std::collections::HashMap<K, V, BuildHasherDefault<FxHasher>>;
pub type ZSet = FastMap<RowKey, Weight>;
pub type VersionMap = FastMap<SmolStr, u64>;

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

/// Represents a weight transition for delta computation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeightTransition {
    /// Record newly appears (old_weight <= 0, new_weight > 0)
    Inserted,
    /// Record's multiplicity increased (old_weight > 0, new_weight > old_weight)
    #[allow(dead_code)]
    MultiplicityIncreased,
    /// Record's multiplicity decreased but still present (new_weight > 0, new_weight < old_weight)
    #[allow(dead_code)]
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
    /// Compute membership changes from self to target
    /// Returns (keys_added, keys_removed)
    fn membership_diff(&self, target: &ZSet) -> (Vec<SmolStr>, Vec<SmolStr>);

    /// Compute membership diff (additions, removals) into provided ZSet
    /// OPTIMIZATION: Avoids allocations
    fn membership_diff_into(&self, target: &ZSet, diff: &mut ZSet);

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

    /// Compute membership diff (additions, removals)
    /// OPTIMIZATION: Returns diff into provided ZSet to avoid allocations
    fn membership_diff_into(&self, target: &ZSet, diff: &mut ZSet) {
        // Records in target but not in self
        for (key, &weight) in target {
            if weight > 0 && !self.is_member(key) {
                diff.insert(key.clone(), 1);
            }
        }

        // Records in self but not in target
        for (key, &weight) in self.iter() {
            if weight > 0 && !target.get(key).map(|&w| w > 0).unwrap_or(false) {
                diff.insert(key.clone(), -1);
            }
        }
    }

    /// Compute membership diff (additions, removals)
    /// Wraps membership_diff_into for convenience
    fn membership_diff(&self, target: &ZSet) -> (Vec<SmolStr>, Vec<SmolStr>) {
        let mut diff_set = FastMap::default();
        self.membership_diff_into(target, &mut diff_set);

        let mut additions = Vec::new();
        let mut removals = Vec::new();

        for (key, weight) in diff_set {
            if weight > 0 {
                additions.push(key);
            } else {
                removals.push(key);
            }
        }
        (additions, removals)
    }

    /// Normalize weights to membership (0 or 1)
    /// OPTIMIZATION: In-place modification
    fn normalize_to_membership(&mut self) {
        // Remove non-members first
        self.retain(|_, &mut w| w > 0);

        // Normalize remaining to 1
        for weight in self.values_mut() {
            if *weight > 1 {
                *weight = 1;
            }
        }
    }

    fn member_count(&self) -> usize {
        self.values().filter(|&&w| w > 0).count()
    }
}

#[cfg(test)]
mod zset_basic_types_test {
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

    #[test]
    fn test_make_zset_key_inline_optimization() {
        // SmolStr inlines strings up to 23 bytes (no heap allocation)
        // Format is "table:id", so we need table.len() + 1 + id.len() <= 23

        // Test case 1: Exactly 23 chars (should be inlined)
        // "table12345:id12345678" = 10 + 1 + 12 = 23 chars
        let key_23 = make_zset_key("table12345", "id123dd45678");
        assert_eq!(key_23.as_str(), "table12345:id123dd45678");
        assert_eq!(key_23.len(), 23);
        assert!(!key_23.is_heap_allocated());

        // Test case 2: Under 23 chars (should be inlined)
        // "user:123" = 4 + 1 + 3 = 8 chars
        let key_short = make_zset_key("user", "123");
        assert_eq!(key_short.as_str(), "user:123");
        assert_eq!(key_short.len(), 8);
        assert!(!key_short.is_heap_allocated());

        // Test case 3: Minimum size (should be inlined)
        // "a:b" = 1 + 1 + 1 = 3 chars
        let key_min = make_zset_key("a", "b");
        assert_eq!(key_min.as_str(), "a:b");
        assert_eq!(key_min.len(), 3);
        assert!(!key_min.is_heap_allocated());

        // Test case 4: Over 23 chars (will be heap allocated)
        // "verylongtable:verylongid123" = 13 + 1 + 13 = 27 chars
        let key_long = make_zset_key("verylongtable", "verylongid123");
        assert_eq!(key_long.as_str(), "verylongtable:verylongid123");
        assert_eq!(key_long.len(), 27);
        assert!(key_long.is_heap_allocated());

        // Test case 5: Exactly 24 chars (just over limit, heap allocated)
        // "table12345:id123456789" = 10 + 1 + 13 = 24 chars
        let key_24 = make_zset_key("table12345", "id12345678999");
        assert_eq!(key_24.as_str(), "table12345:id12345678999");
        assert_eq!(key_24.len(), 24);
        assert!(key_24.is_heap_allocated());

        // Test case 6: Typical real-world keys (should be inlined)
        // "user:abc123" = 4 + 1 + 6 = 11 chars
        let key_typical = make_zset_key("user", "abc123");
        assert!(!key_typical.is_heap_allocated());

        // "post:xyz789" = 4 + 1 + 6 = 11 chars
        let key_typical2 = make_zset_key("post", "xyz789");
        assert!(!key_typical2.is_heap_allocated());

        // "thread:12345678" = 6 + 1 + 8 = 15 chars
        let key_typical3 = make_zset_key("thread", "12345678");
        assert!(!key_typical3.is_heap_allocated());
    }

    #[test]
    fn test_make_zset_key_inline_boundary() {
        // Precisely test the 23-byte boundary

        // Build keys of increasing length around the boundary
        for total_len in 20..=26 {
            // Split between table and id (accounting for colon)
            let table_len = 5;
            let id_len = total_len - table_len - 1; // -1 for colon

            let table: String = "t".repeat(table_len);
            let id: String = "i".repeat(id_len);

            let key = make_zset_key(&table, &id);

            assert_eq!(key.len(), total_len);

            if total_len <= 23 {
                assert!(
                    !key.is_heap_allocated(),
                    "Key of length {} should be inlined but was heap allocated",
                    total_len
                );
            } else {
                assert!(
                    key.is_heap_allocated(),
                    "Key of length {} should be heap allocated but was inlined",
                    total_len
                );
            }
        }
    }

    fn make_zset(items: &[(&str, i64)]) -> ZSet {
        items.iter().map(|(k, w)| (SmolStr::new(*k), *w)).collect()
    }

    #[test]
    fn test_zset_diff_simple() {
        let old = make_zset(&[("a", 1), ("b", 1)]);
        let new = make_zset(&[("b", 1), ("c", 1)]);

        let diff = old.diff(&new);

        assert_eq!(diff.get("a").copied(), Some(-1)); // Removed
        assert_eq!(diff.get("b").copied(), None); // Unchanged
        assert_eq!(diff.get("c").copied(), Some(1)); // Added
    }

    #[test]
    fn test_zset_diff_multiplicity() {
        let old = make_zset(&[("a", 1)]);
        let new = make_zset(&[("a", 3)]);

        let diff = old.diff(&new);

        assert_eq!(diff.get("a").copied(), Some(2)); // Multiplicity increased by 2
    }

    #[test]
    fn test_weight_transition_inserted() {
        assert_eq!(WeightTransition::compute(0, 1), WeightTransition::Inserted);
        assert_eq!(WeightTransition::compute(-1, 1), WeightTransition::Inserted);
    }

    #[test]
    fn test_weight_transition_deleted() {
        assert_eq!(WeightTransition::compute(1, 0), WeightTransition::Deleted);
        assert_eq!(WeightTransition::compute(2, -1), WeightTransition::Deleted);
    }

    #[test]
    fn test_weight_transition_unchanged() {
        assert_eq!(WeightTransition::compute(1, 1), WeightTransition::Unchanged);
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
        assert!(changes
            .iter()
            .any(|(k, t)| k == "a" && *t == WeightTransition::Deleted));
        assert!(changes
            .iter()
            .any(|(k, t)| k == "c" && *t == WeightTransition::Inserted));
        // b should NOT be in the list
        assert!(!changes.iter().any(|(k, _)| k == "b"));
    }

    #[test]
    fn test_membership_is_member() {
        let z = make_zset(&[("a", 1), ("b", 2), ("c", 0), ("d", -1)]);

        assert!(z.is_member("a"));
        assert!(z.is_member("b")); // weight > 1 still counts as member
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

        assert!(additions.contains(&SmolStr::new("d")));
        assert!(removals.contains(&SmolStr::new("a")));
        assert_eq!(additions.len(), 1);
        assert_eq!(removals.len(), 1);
    }

    #[test]
    fn test_membership_diff_ignores_weight_changes() {
        let old = make_zset(&[("a", 1)]);
        let new = make_zset(&[("a", 5)]); // Weight changed but still present

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
        assert_eq!(z.get("b"), Some(&1)); // 5 → 1
        assert!(!z.contains_key("c")); // 0 → removed
        assert!(!z.contains_key("d")); // -2 → removed
    }
}
