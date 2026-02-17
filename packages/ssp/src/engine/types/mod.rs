// Re-export constants and types from db_mod needed by engine components
pub use crate::db_mod::types::{
    TAG_NULL, TAG_BOOL, TAG_I64, TAG_F64, TAG_STR, TAG_NESTED_CBOR, TAG_U64,
    HEADER_SIZE, INDEX_ENTRY_SIZE,
    IndexEntry, FieldRef, FieldSlot, FieldIter
};

mod path;
mod zset;
mod circuit_types;
mod batch_deltas;

pub use path::Path;
// Use the unified SpookyValue from db_mod
pub use crate::db_mod::types::{SpookyValue, SpookyNumber};
pub use zset::{FastMap, FastHashSet, RowKey, VersionMap, Weight, ZSet, make_zset_key, parse_zset_key, ZSetOps, WeightTransition, ZSetMembershipOps};
pub use circuit_types::{Operation, Record, Delta};
pub use batch_deltas::BatchDeltas;
