use super::super::types::{FastMap, FastHashSet, ZSet};
use super::circuit_types::Operation;
use smol_str::SmolStr;

/// Batch deltas with separate tracking for content updates.
///
/// This struct separates:
/// - Membership changes (weight != 0): Records entering/leaving the set
/// - Content updates (weight = 0): Records that changed but stayed in the set
#[derive(Debug, Clone, Default)]
pub struct BatchDeltas {
    /// ZSet membership deltas (weight != 0)
    pub membership: FastMap<String, ZSet>,

    /// Keys with content changes (including weight=0 updates)
    /// Map: table -> set of updated keys
    pub content_updates: FastMap<String, FastHashSet<SmolStr>>,
}

impl BatchDeltas {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a delta from an operation
    pub fn add(&mut self, table: &str, key: SmolStr, op: Operation) {
        let weight = op.weight();

        if weight != 0 {
            let zset = self.membership.entry(table.to_string()).or_default();
            *zset.entry(key.clone()).or_insert(0) += weight;
        }

        if op.changes_content() {
            self.content_updates
                .entry(table.to_string())
                .or_default()
                .insert(key); // insert instead of push - no duplicates
        }
    }

    /// Check if there are any changes
    pub fn is_empty(&self) -> bool {
        self.membership.is_empty() && self.content_updates.is_empty()
    }

    /// Get all tables that have changes
    pub fn changed_tables(&self) -> Vec<String> {
        let mut tables: Vec<String> = self.membership.keys().cloned().collect();
        for table in self.content_updates.keys() {
            if !tables.contains(table) {
                tables.push(table.clone());
            }
        }
        tables
    }
}

#[cfg(test)]
mod batch_deltas_tests {
    use super::*;

    #[test]
    fn test_new_empty() {
        let bd = BatchDeltas::new();
        assert!(bd.membership.is_empty());
        assert!(bd.content_updates.is_empty());
    }

    #[test]
    fn test_batch_deltas_add() {
        let mut bd = BatchDeltas::new();
        bd.add("user", SmolStr::new("user:223k4bj3"), Operation::Create);
        println!("bd_create: {:?}", bd);
        //assert!(bd.membership.len() == 1);
        //assert!(bd.content_updates.len() == 1);
        bd.add("user", SmolStr::new("user:223k4bj3"), Operation::Update);
        println!("bd_update: {:?}", bd);
        //assert!(bd.membership.len() == 1);
        //assert!(bd.content_updates.len() == 1);
        bd.add("user", SmolStr::new("user:223k4bj3"), Operation::Delete);
        println!("bd_delete: {:?}", bd);
        //assert!(bd.membership.len() == 1);
        //assert!(bd.content_updates.len() == 1);
        bd.add("user", SmolStr::new("user:s0do89f"), Operation::Create);
        println!("bd_new_user: {:?}", bd);
    }
}
