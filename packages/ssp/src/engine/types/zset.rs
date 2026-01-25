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

    fn make_zset(items: &[(&str, i64)]) -> ZSet {
        items.iter().map(|(k, w)| (SmolStr::new(*k), *w)).collect()
    }
    
    #[test]
    fn test_zset_diff_simple() {
        let old = make_zset(&[("a", 1), ("b", 1)]);
        let new = make_zset(&[("b", 1), ("c", 1)]);
        
        let diff = old.diff(&new);
        
        assert_eq!(diff.get("a").copied(), Some(-1));  // Removed
        assert_eq!(diff.get("b").copied(), None);      // Unchanged
        assert_eq!(diff.get("c").copied(), Some(1));   // Added
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
