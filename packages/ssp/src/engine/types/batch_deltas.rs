use super::super::types::{FastHashSet, FastMap, ZSet};
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

    pub fn cleanup(&mut self) {
        self.membership.retain(|_, zset| {
            zset.retain(|_, w| *w != 0);
            !zset.is_empty()
        });
        self.content_updates
            .retain(|_, updates| !updates.is_empty());
    }

    /// Add a delta from an operation
    pub fn add(&mut self, table: &str, key: SmolStr, op: Operation) {
        let weight = op.weight();

        if weight != 0 {
            let zset = self.membership.entry(table.to_string()).or_default();
            let entry = zset.entry(key.clone()).or_insert(0);
            *entry += weight;

            if *entry == 0 {
                zset.remove(&key);
                // Also remove from content_updates - record no longer exists
                if let Some(updates) = self.content_updates.get_mut(table) {
                    updates.remove(&key);
                }
            }
        }

        //TODO: check if nessesary or just for logical correntniss but bad for performance
        //self.cleanup();

        if op.changes_content() {
            self.content_updates
                .entry(table.to_string())
                .or_default()
                .insert(key);
        }
    }

    /// Check if there are any changes
    pub fn is_empty(&self) -> bool {
        self.membership.is_empty() && self.content_updates.is_empty()
    }

    /// Get all tables that have changes
    pub fn changed_tables(&self) -> FastHashSet<String> {
        let mut tables = FastHashSet::default();

        for (table, zset) in &self.membership {
            if !zset.is_empty() {
                tables.insert(table.clone());
            }
        }

        for (table, updates) in &self.content_updates {
            if !updates.is_empty() {
                tables.insert(table.clone());
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
        assert!(bd.is_empty());
        assert!(bd.changed_tables().is_empty());
    }

    #[test]
    fn test_add_create() {
        let mut bd = BatchDeltas::new();
        bd.add("user", SmolStr::new("user:123"), Operation::Create);

        // Create: membership +1, content_updates +1
        assert_eq!(bd.membership.get("user").unwrap().get("user:123"), Some(&1));
        assert!(bd
            .content_updates
            .get("user")
            .unwrap()
            .contains(&SmolStr::new("user:123")));
        assert!(!bd.is_empty());
    }

    #[test]
    fn test_add_update_only() {
        let mut bd = BatchDeltas::new();
        bd.add("user", SmolStr::new("user:123"), Operation::Update);

        // Update: membership unchanged (weight=0), content_updates +1
        assert!(bd.membership.is_empty());
        assert!(bd
            .content_updates
            .get("user")
            .unwrap()
            .contains(&SmolStr::new("user:123")));
    }

    #[test]
    fn test_add_delete_only() {
        let mut bd = BatchDeltas::new();
        bd.add("user", SmolStr::new("user:123"), Operation::Delete);

        // Delete: membership -1, content_updates unchanged
        assert_eq!(
            bd.membership.get("user").unwrap().get("user:123"),
            Some(&-1)
        );
        assert!(bd.content_updates.is_empty());
    }

    #[test]
    fn test_create_then_delete_cancels() {
        let mut bd = BatchDeltas::new();
        bd.add("user", SmolStr::new("user:123"), Operation::Create);
        bd.add("user", SmolStr::new("user:123"), Operation::Delete);

        // Net weight: +1 + -1 = 0, key removed
        let zset = bd.membership.get("user").unwrap();
        assert!(zset.get("user:123").is_none());

        // content_updates also cleaned
        let updates = bd.content_updates.get("user").unwrap();
        assert!(!updates.contains(&SmolStr::new("user:123")));
    }

    #[test]
    fn test_multiple_tables() {
        let mut bd = BatchDeltas::new();
        bd.add("user", SmolStr::new("user:1"), Operation::Create);
        bd.add("thread", SmolStr::new("thread:1"), Operation::Create);
        bd.add("post", SmolStr::new("post:1"), Operation::Create);

        assert_eq!(bd.membership.len(), 3);
        assert_eq!(bd.content_updates.len(), 3);
    }

    #[test]
    fn test_changed_tables_create() {
        let mut bd = BatchDeltas::new();
        bd.add("user", SmolStr::new("user:123"), Operation::Create);

        let tables = bd.changed_tables();
        assert_eq!(tables.len(), 1);
        assert!(tables.contains(&"user".to_string()));
    }

    #[test]
    fn test_changed_tables_update_only() {
        let mut bd = BatchDeltas::new();
        bd.add("user", SmolStr::new("user:123"), Operation::Update);

        // Update-only: only in content_updates, not membership
        let tables = bd.changed_tables();
        assert_eq!(tables.len(), 1);
        assert!(tables.contains(&"user".to_string()));
    }

    #[test]
    fn test_changed_tables_delete_only() {
        let mut bd = BatchDeltas::new();
        bd.add("user", SmolStr::new("user:123"), Operation::Delete);

        // Delete-only: only in membership, not content_updates
        let tables = bd.changed_tables();
        assert_eq!(tables.len(), 1);
        assert!(tables.contains(&"user".to_string()));
    }

    #[test]
    fn test_changed_tables_excludes_empty() {
        let mut bd = BatchDeltas::new();
        bd.add("user", SmolStr::new("user:123"), Operation::Create);
        bd.add("user", SmolStr::new("user:123"), Operation::Delete);

        // Create + Delete = cancelled out, should NOT appear in changed_tables
        let tables = bd.changed_tables();
        assert!(tables.is_empty());
    }

    #[test]
    fn test_changed_tables_multiple() {
        let mut bd = BatchDeltas::new();
        bd.add("user", SmolStr::new("user:1"), Operation::Create);
        bd.add("thread", SmolStr::new("thread:1"), Operation::Update);
        bd.add("post", SmolStr::new("post:1"), Operation::Delete);

        let tables = bd.changed_tables();
        assert_eq!(tables.len(), 3);
        assert!(tables.contains(&"user".to_string()));
        assert!(tables.contains(&"thread".to_string()));
        assert!(tables.contains(&"post".to_string()));
    }

    #[test]
    fn test_changed_tables_no_duplicates() {
        let mut bd = BatchDeltas::new();
        // Same table, different operations
        bd.add("user", SmolStr::new("user:1"), Operation::Create);
        bd.add("user", SmolStr::new("user:2"), Operation::Update);
        bd.add("user", SmolStr::new("user:3"), Operation::Delete);

        let tables = bd.changed_tables();
        assert_eq!(tables.len(), 1); // Only "user" once
        assert!(tables.contains(&"user".to_string()));
    }

    #[test]
    fn test_is_empty() {
        let mut bd = BatchDeltas::new();
        assert!(bd.is_empty());

        bd.add("user", SmolStr::new("user:123"), Operation::Create);
        assert!(!bd.is_empty());
    }
}
