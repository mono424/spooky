use rustc_hash::FxHasher;
use smol_str::SmolStr;
use std::collections::HashSet;
use std::hash::BuildHasherDefault;

pub type Weight = i64;
pub type RowKey = SmolStr;
pub type FastMap<K, V> = std::collections::HashMap<K, V, BuildHasherDefault<FxHasher>>;
pub type FastHashSet<T> = HashSet<T, BuildHasherDefault<FxHasher>>;
pub type ZSet = FastMap<RowKey, Weight>;

#[derive(Debug)]
pub enum SpookyDbError {
    Redb(redb::Error),
    Serialization(String),
    /// Table name contains ':' or key format is otherwise invalid.
    InvalidKey(String),
}

impl std::fmt::Display for SpookyDbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SpookyDbError::Redb(e) => write!(f, "redb error: {}", e),
            SpookyDbError::Serialization(s) => write!(f, "serialization error: {}", s),
            SpookyDbError::InvalidKey(s) => write!(f, "invalid key: {}", s),
        }
    }
}

impl std::error::Error for SpookyDbError {}

impl From<redb::Error> for SpookyDbError {
    fn from(e: redb::Error) -> Self {
        SpookyDbError::Redb(e)
    }
}

impl From<redb::DatabaseError> for SpookyDbError {
    fn from(e: redb::DatabaseError) -> Self {
        SpookyDbError::Redb(e.into())
    }
}

impl From<redb::TransactionError> for SpookyDbError {
    fn from(e: redb::TransactionError) -> Self {
        SpookyDbError::Redb(e.into())
    }
}

impl From<redb::TableError> for SpookyDbError {
    fn from(e: redb::TableError) -> Self {
        SpookyDbError::Redb(e.into())
    }
}

impl From<redb::CommitError> for SpookyDbError {
    fn from(e: redb::CommitError) -> Self {
        SpookyDbError::Redb(e.into())
    }
}

impl From<redb::StorageError> for SpookyDbError {
    fn from(e: redb::StorageError) -> Self {
        SpookyDbError::Redb(e.into())
    }
}

impl From<crate::error::RecordError> for SpookyDbError {
    fn from(e: crate::error::RecordError) -> Self {
        SpookyDbError::Serialization(e.to_string())
    }
}

/// A single mutation ready for `apply_batch`.
///
/// `data` MUST be pre-serialized SpookyRecord bytes (from `from_cbor` /
/// `serialize_into`). Serialization happens BEFORE `begin_write()` to
/// minimize write lock hold time.
pub struct DbMutation {
    pub table: SmolStr,
    pub id: SmolStr,
    pub op: Operation,
    /// `None` for `Delete`; `Some(bytes)` for `Create` / `Update`.
    pub data: Option<Vec<u8>>,
    /// Explicit version. If `None`, VERSION_TABLE entry is left unchanged.
    pub version: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    /// Record did not exist before. ZSet weight += 1.
    Create,
    /// Record existed, bytes replaced. ZSet weight unchanged.
    Update,
    /// Record removed. ZSet weight -= 1 (entry removed at 0).
    Delete,
}

impl Operation {
    /// Weight delta this operation contributes to the ZSet.
    pub fn weight(self) -> i64 {
        match self {
            Operation::Create => 1,
            Operation::Delete => -1,
            Operation::Update => 0,
        }
    }
}

/// Return value of `apply_batch`. Contains all per-table deltas accumulated
/// in a single pass — no extra allocations after the batch commit.
pub struct BatchMutationResult {
    /// Per-table ZSet weight deltas (Create = +1, Delete = -1, Update = 0).
    /// Key: table name → ZSet<record_id, weight_delta>.
    pub membership_deltas: FastMap<SmolStr, ZSet>,
    /// Per-table set of record IDs whose content was written (Create or Update).
    pub content_updates: FastMap<SmolStr, FastHashSet<SmolStr>>,
    /// Tables that had at least one mutation (deduplicated).
    pub changed_tables: Vec<SmolStr>,
}

/// One record for `bulk_load`. `data` must be pre-serialized SpookyRecord bytes.
pub struct BulkRecord {
    pub table: SmolStr,
    pub id: SmolStr,
    pub data: Vec<u8>,
}
