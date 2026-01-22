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
            Operation::Create | Operation::Update => 1,
            Operation::Delete => -1,
        }
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
}

impl Delta {
    pub fn new(table: SmolStr, key: SmolStr, weight: i64) -> Self {
        Self { table, key, weight }
    }

    /// Create a delta from an operation
    #[inline]
    pub fn from_operation(table: SmolStr, key: SmolStr, op: Operation) -> Self {
        Self {
            table,
            key,
            weight: op.weight(),
        }
    }
}
