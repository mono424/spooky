pub mod circuit;
pub mod eval;
pub mod operators;
pub mod types;
pub mod update;
pub mod view;

// Public re-exports (maintain backwards compatibility)
pub use circuit::Circuit;
pub use view::QueryPlan;

// Re-export types that were previously in view.rs
pub use types::{FastMap, Path, RowKey, SpookyValue, VersionMap, Weight, ZSet};
pub use operators::{JoinCondition, Operator, OrderSpec, Predicate, Projection};
