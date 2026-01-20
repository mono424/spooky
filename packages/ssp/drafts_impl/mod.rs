pub mod circuit;
pub mod eval;
pub mod metadata;
pub mod operators;
pub mod types;
pub mod update;
pub mod view;

// Public re-exports (maintain backwards compatibility)
pub use circuit::Circuit;
pub use view::QueryPlan;

// Re-export types that were previously in view.rs
pub use types::{FastMap, Path, RowKey, SpookyValue, Weight, ZSet};
pub use operators::{JoinCondition, Operator, OrderSpec, Predicate, Projection};

// Re-export metadata types
pub use metadata::{
    BatchMeta, MetadataProcessor, RecordMeta, VersionStrategy, ViewMetadataState,
};

// Re-export update types
pub use update::{
    build_update, compute_flat_hash, DeltaEvent, DeltaRecord, MaterializedViewUpdate,
    RawViewResult, StreamingUpdate, ViewDelta, ViewResultFormat, ViewUpdate,
};
