use super::spooky_value::SpookyValue;
use smol_str::SmolStr;

/// Operation type for record mutations
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Create,
    Update,
    Delete,
}

impl Operation {
    /// Convert from string representation
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "create" => Some(Operation::Create),
            "update" => Some(Operation::Update),
            "delete" => Some(Operation::Delete),
            _ => None,
        }
    }

    /// Get weight for ZSet delta
    #[inline]
    pub fn weight(&self) -> i64 {
        match self {
            Operation::Create => 1,
            Operation::Update => 0, // No membership change
            Operation::Delete => -1,
        }
    }

    /// Does this operation change record content?
    #[inline]
    pub fn changes_content(&self) -> bool {
        matches!(self, Operation::Create | Operation::Update)
    }

    /// Does this operation change set membership?
    #[inline]
    pub fn changes_membership(&self) -> bool {
        matches!(self, Operation::Create | Operation::Delete)
    }

    /// Is this an addition (Create or Update)?
    #[inline]
    pub fn is_additive(&self) -> bool {
        matches!(self, Operation::Create | Operation::Update)
    }
}

/// A single record to be ingested
#[derive(Debug, Clone)]
pub struct Record {
    pub table: SmolStr,
    pub id: SmolStr,
    pub data: SpookyValue,
    pub hash: String,
}

impl Record {
    pub fn new(table: SmolStr, id: SmolStr, data: SpookyValue, hash: String) -> Self {
        Self {
            table,
            id,
            data,
            hash,
        }
    }
}

/// A single ZSet delta (change)
#[derive(Debug, Clone)]
pub struct Delta {
    pub table: SmolStr,
    pub key: SmolStr,
    pub weight: i64,
    /// True if the record content was modified (Create or Update)
    pub content_changed: bool,
}

impl Delta {
    pub fn new(table: SmolStr, key: SmolStr, weight: i64) -> Self {
        Self {
            table,
            key,
            weight,
            content_changed: weight >= 0, // Create/Update change content
        }
    }

    /// Create a delta from an operation
    #[inline]
    pub fn from_operation(table: SmolStr, key: SmolStr, op: Operation) -> Self {
        Self {
            table,
            key,
            weight: op.weight(),
            content_changed: op.changes_content(),
        }
    }

    /// Create a content-only update delta (weight=0, content_changed=true)
    #[inline]
    pub fn content_update(table: SmolStr, key: SmolStr) -> Self {
        Self {
            table,
            key,
            weight: 0,
            content_changed: true,
        }
    }
}

#[cfg(test)]
mod circuit_types_tests {
    use super::*;

    #[test]
    fn test_operation_from_str() {
        let op_create = Operation::from_str("create");
        let op_update = Operation::from_str("update");
        let op_delete = Operation::from_str("delete");

        let op_create_upercase = Operation::from_str("CREATE");
        let op_update_upercase = Operation::from_str("Update");
        let op_delete_upercase = Operation::from_str("DelEtE");

        let op_invalid = Operation::from_str("foo");

        assert_eq!(op_create, Some(Operation::Create));
        assert_eq!(op_update, Some(Operation::Update));
        assert_eq!(op_delete, Some(Operation::Delete));

        assert_eq!(op_create_upercase, Some(Operation::Create));
        assert_eq!(op_update_upercase, Some(Operation::Update));
        assert_eq!(op_delete_upercase, Some(Operation::Delete));

        assert!(op_invalid.is_none());
    }

    #[test]
    fn test_operation_weight() {
        let op_create = Operation::from_str("create");
        let op_update = Operation::from_str("update");
        let op_delete = Operation::from_str("delete");

        assert_eq!(op_create.unwrap().weight(), 1);
        assert_eq!(op_update.unwrap().weight(), 0);
        assert_eq!(op_delete.unwrap().weight(), -1);
    }

    #[test]
    fn test_operation_changes_content() {
        let op_create = Operation::from_str("create");
        let op_update = Operation::from_str("update");
        let op_delete = Operation::from_str("delete");

        assert_eq!(op_create.unwrap().changes_content(), true);
        assert_eq!(op_update.unwrap().changes_content(), true);
        assert_eq!(op_delete.unwrap().changes_content(), false);
    }

    #[test]
    fn test_operation_changes_membership() {
        let op_create = Operation::from_str("create");
        let op_update = Operation::from_str("update");
        let op_delete = Operation::from_str("delete");

        assert_eq!(op_create.unwrap().changes_membership(), true);
        assert_eq!(op_update.unwrap().changes_membership(), false);
        assert_eq!(op_delete.unwrap().changes_membership(), true);
    }

    #[test]
    fn test_delta_new() {
        let delta_create = Delta::new(SmolStr::new("user"), SmolStr::new("user:kljdj34jk3"), 1);
        let delta_update = Delta::new(SmolStr::new("user"), SmolStr::new("user:kljdj34jk3"), 0);
        let delta_delete = Delta::new(SmolStr::new("user"), SmolStr::new("user:kljdj34jk3"), -1);

        assert!(delta_create.content_changed);
        assert!(delta_update.content_changed);
        assert!(!delta_delete.content_changed);
    }

    #[test]
    fn test_delta_from_operation() {
        let delta_create = Delta::from_operation(
            SmolStr::new("user"),
            SmolStr::new("user:kljdj34jk3"),
            Operation::Create,
        );
        let delta_update = Delta::from_operation(
            SmolStr::new("user"),
            SmolStr::new("user:kljdj34jk3"),
            Operation::Update,
        );
        let delta_delete = Delta::from_operation(
            SmolStr::new("user"),
            SmolStr::new("user:kljdj34jk3"),
            Operation::Delete,
        );

        assert!(delta_create.content_changed);
        assert!(delta_update.content_changed);
        assert!(!delta_delete.content_changed);
    }

    #[test]
    fn test_delta_content_update() {
        let delta = Delta::content_update(SmolStr::new("user"), SmolStr::new("user:kljdj34jk3"));
        assert!(delta.content_changed);
    }
}
