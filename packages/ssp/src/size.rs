//! Heap-footprint estimation for the circuit.
//!
//! The SSP holds every syncable table in memory for the whole process
//! lifetime, so "how many bytes is the circuit" is the number that decides
//! whether a tenant fits under its cgroup cap. Nothing reported that before:
//! the only memory signal was the control plane's `docker stats` scrape, which
//! gives a single process-wide number with no way to attribute it to a table,
//! a view, or an operator.
//!
//! These are **estimates**, deliberately. An allocator-accurate number would
//! mean a `deepsize`-style derive on every type plus a wasm story for the
//! browser and Durable Object builds, and it still would not attribute
//! anything. What matters here is that the numbers are *comparable*: the same
//! estimator run before and after a change tells you which component moved and
//! by how much. Treat absolute values as indicative and deltas as real.
//!
//! Everything here is pure computation over `std` collections, so it works
//! identically on wasm32 — which is the point, since that is exactly where a
//! `/proc`-based RSS reading is unavailable.

use crate::algebra::ZSet;

/// Approximate heap cost of a `std::collections::HashMap`'s bucket array.
///
/// hashbrown holds a 7/8 load factor and rounds the bucket count up to a power
/// of two, with one control byte per bucket alongside the `(K, V)` slot. Takes
/// a capacity rather than the map itself so callers can use it for any K/V.
///
/// `HashMap::capacity()` is already the *derated* figure (elements storable
/// before a resize, i.e. `buckets * 7/8`), so inverting it recovers the bucket
/// count exactly. Rounding up an extra step here would double the reported
/// size of every exactly-full table.
pub fn map_table_bytes<K, V>(capacity: usize) -> usize {
    if capacity == 0 {
        return 0;
    }
    let buckets = (capacity * 8 / 7).max(1).next_power_of_two();
    buckets * (std::mem::size_of::<(K, V)>() + 1)
}

/// Approximate heap cost of a `Vec<T>`'s backing buffer.
pub fn vec_bytes<T>(capacity: usize) -> usize {
    capacity * std::mem::size_of::<T>()
}

/// Heap bytes held by a Z-set.
pub fn zset_bytes(zset: &ZSet) -> usize {
    // Only the bucket array is charged here. The keys are `Arc<str>` clones of
    // one shared allocation, so counting their bytes in every Z-set that holds
    // them would multiply a cost that is now paid once.
    map_table_bytes::<crate::algebra::RowKey, i64>(zset.capacity())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_structures_cost_nothing() {
        assert_eq!(map_table_bytes::<String, i64>(0), 0);
        assert_eq!(vec_bytes::<u64>(0), 0);
        assert_eq!(zset_bytes(&ZSet::new()), 0);
    }

    /// Keys are shared `Arc<str>` clones, so a Z-set is charged for its bucket
    /// array only — counting the key bytes again in every Z-set that holds a
    /// row would multiply a cost that is now paid once.
    #[test]
    fn zset_bytes_charges_the_bucket_array_not_the_shared_keys() {
        let mut z = ZSet::new();
        z.insert("thread:abcdefghij".into(), 1);
        assert!(zset_bytes(&z) > 0);

        // A very long key must not change the reported size: it is shared.
        let mut long = ZSet::new();
        long.insert("thread:".to_string().repeat(50).into(), 1);
        assert_eq!(zset_bytes(&z), zset_bytes(&long));
    }

    #[test]
    fn map_table_bytes_grows_with_capacity() {
        let small = map_table_bytes::<String, i64>(8);
        let large = map_table_bytes::<String, i64>(8192);
        assert!(large > small * 100);
    }

    /// Regression guard for a real overcount: `HashMap::capacity()` is already
    /// derated to 7/8 of the bucket count, so rounding it up an extra step
    /// doubled the reported size of every exactly-full table (a 16-bucket map
    /// was billed as 32). Anchor the inversion at known hashbrown sizes.
    #[test]
    fn map_table_bytes_recovers_exact_bucket_count() {
        let slot = std::mem::size_of::<(String, i64)>() + 1;
        // A HashMap that reports capacity 14 is a 16-bucket table.
        assert_eq!(map_table_bytes::<String, i64>(14), 16 * slot);
        // Capacity 7 is 8 buckets; capacity 3 is 4.
        assert_eq!(map_table_bytes::<String, i64>(7), 8 * slot);
        assert_eq!(map_table_bytes::<String, i64>(3), 4 * slot);
        assert_eq!(map_table_bytes::<String, i64>(1), 1 * slot);
    }

    /// The inversion must agree with what `HashMap` actually reports, not just
    /// with hand-picked constants.
    #[test]
    fn map_table_bytes_matches_real_hashmap_capacities() {
        use std::collections::HashMap;
        for n in [1usize, 3, 7, 12, 14, 100, 1000] {
            let mut m: HashMap<String, i64> = HashMap::new();
            for i in 0..n {
                m.insert(format!("k{i}"), i as i64);
            }
            let cap = m.capacity();
            let buckets = (cap * 8 / 7).max(1).next_power_of_two();
            assert!(
                buckets >= cap && buckets < cap * 2 + 2,
                "n={n} cap={cap} inferred buckets={buckets} is not a plausible table size"
            );
        }
    }
}

/// Bucket-array cost for a `hashbrown::HashTable<T>`, which stores `T` alone
/// with no separate key.
pub fn map_table_bytes_for<T>(capacity: usize) -> usize {
    if capacity == 0 {
        return 0;
    }
    let buckets = (capacity * 8 / 7).max(1).next_power_of_two();
    buckets * (std::mem::size_of::<T>() + 1)
}
