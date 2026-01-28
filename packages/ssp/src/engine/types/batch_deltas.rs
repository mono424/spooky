use super::super::types::{FastMap, ZSet};
use smol_str::SmolStr;
use super::circuit_types::Operation;

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
    /// Map: table -> list of updated keys
    pub content_updates: FastMap<String, Vec<SmolStr>>,
}

impl BatchDeltas {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a delta from an operation
    pub fn add(&mut self, table: &str, key: SmolStr, op: Operation) {
        let weight = op.weight();
        
        // Track membership changes (weight != 0)
        if weight != 0 {
            let zset = self.membership.entry(table.to_string()).or_default();
            *zset.entry(key.clone()).or_insert(0) += weight;
        }
        
        // Track content changes (Create and Update)
        if op.changes_content() {
            self.content_updates
                .entry(table.to_string())
                .or_default()
                .push(key);
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
