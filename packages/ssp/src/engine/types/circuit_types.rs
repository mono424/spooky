use smol_str::SmolStr;
use super::spooky_value::SpookyValue;

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
            Operation::Update => 0,  // No membership change
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
        Self { table, id, data, hash }
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
