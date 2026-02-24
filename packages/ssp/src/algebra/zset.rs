use super::weight::Weight;
use std::collections::HashMap;

/// A row key is a string in the format "table:id".
pub type RowKey = String;

/// A Z-set: a map from keys to integer weights.
///
/// Z-sets are the core data structure of DBSP. They generalize multisets
/// by allowing negative weights (representing deletions). A Z-set forms:
/// - An abelian group under pointwise addition
/// - A ring when combined with the join multiplication
///
/// Convention: zero-weight entries are removed from the map to keep it sparse.
pub type ZSet = HashMap<RowKey, Weight>;

/// Core Z-set algebra operations following DBSP semantics.
pub trait ZSetOps {
    /// Pointwise addition: self[k] += other[k] for all k.
    /// Removes entries where the resulting weight is zero.
    fn add(&mut self, other: &ZSet);

    /// Pointwise subtraction (differentiation): result[k] = other[k] - self[k].
    /// Only includes non-zero differences.
    fn diff(&self, other: &ZSet) -> ZSet;

    /// Group inverse (negation): result[k] = -self[k] for all k.
    fn negate(&self) -> ZSet;

    /// Positive support: returns a new Z-set containing only entries with weight > 0.
    fn positive(&self) -> ZSet;

    /// Check if a key is present (weight > 0).
    fn is_present(&self, key: &str) -> bool;
}

impl ZSetOps for ZSet {
    fn add(&mut self, other: &ZSet) {
        for (key, &weight) in other {
            let entry = self.entry(key.clone()).or_insert(0);
            *entry += weight;
            if *entry == 0 {
                self.remove(key);
            }
        }
    }

    fn diff(&self, other: &ZSet) -> ZSet {
        let mut result = HashMap::new();

        // Keys in other: result[k] = other[k] - self[k]
        for (key, &new_weight) in other {
            let old_weight = self.get(key).copied().unwrap_or(0);
            let d = new_weight - old_weight;
            if d != 0 {
                result.insert(key.clone(), d);
            }
        }

        // Keys only in self (removed from other): result[k] = 0 - self[k] = -self[k]
        for (key, &old_weight) in self {
            if !other.contains_key(key) {
                result.insert(key.clone(), -old_weight);
            }
        }

        result
    }

    fn negate(&self) -> ZSet {
        self.iter().map(|(k, &w)| (k.clone(), -w)).collect()
    }

    fn positive(&self) -> ZSet {
        self.iter()
            .filter(|(_, &w)| w > 0)
            .map(|(k, &w)| (k.clone(), w))
            .collect()
    }

    fn is_present(&self, key: &str) -> bool {
        self.get(key).map(|&w| w > 0).unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zset(items: &[(&str, i64)]) -> ZSet {
        items.iter().map(|(k, w)| (k.to_string(), *w)).collect()
    }

    #[test]
    fn add_combines_weights() {
        let mut a = zset(&[("x", 1), ("y", 2)]);
        a.add(&zset(&[("y", -1), ("z", 3)]));
        assert_eq!(a.get("x"), Some(&1));
        assert_eq!(a.get("y"), Some(&1)); // 2 + (-1) = 1
        assert_eq!(a.get("z"), Some(&3));
    }

    #[test]
    fn add_removes_zero_weight_entries() {
        let mut a = zset(&[("x", 1)]);
        a.add(&zset(&[("x", -1)]));
        assert!(!a.contains_key("x")); // weight 0 → removed from map
    }

    #[test]
    fn diff_computes_pointwise_subtraction() {
        let old = zset(&[("a", 1), ("b", 2)]);
        let new = zset(&[("b", 2), ("c", 3)]);
        let d = old.diff(&new);
        assert_eq!(d.get("a"), Some(&-1)); // removed
        assert!(!d.contains_key("b")); // unchanged → not in diff
        assert_eq!(d.get("c"), Some(&3)); // added
    }

    #[test]
    fn negate_inverts_all_weights() {
        let z = zset(&[("a", 3), ("b", -2)]);
        let n = z.negate();
        assert_eq!(n.get("a"), Some(&-3));
        assert_eq!(n.get("b"), Some(&2));
    }

    #[test]
    fn add_negate_is_identity() {
        // a + (-a) = empty (group inverse property)
        let a = zset(&[("x", 5), ("y", -3)]);
        let mut result = a.clone();
        result.add(&a.negate());
        assert!(result.is_empty());
    }

    #[test]
    fn positive_returns_only_positive_weights() {
        let z = zset(&[("a", 1), ("b", -1), ("c", 0), ("d", 3)]);
        let p = z.positive();
        assert_eq!(p.len(), 2);
        assert_eq!(p.get("a"), Some(&1));
        assert_eq!(p.get("d"), Some(&3));
    }

    #[test]
    fn is_present_checks_positive_weight() {
        let z = zset(&[("a", 1), ("b", 0), ("c", -1)]);
        assert!(z.is_present("a"));
        assert!(!z.is_present("b"));
        assert!(!z.is_present("c"));
        assert!(!z.is_present("nonexistent"));
    }
}
