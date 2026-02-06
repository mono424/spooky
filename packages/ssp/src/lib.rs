// src/lib.rs

#[cfg(not(target_arch = "wasm32"))]
//#[global_allocator]
//static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;
pub mod converter;
pub mod engine;
pub mod logging;
pub mod sanitizer;
pub mod service;

#[cfg(all(feature = "parallel", not(target_arch = "wasm32")))]
pub use rayon::prelude::*;

// Re-export commonly used types for convenience
pub use engine::circuit::Circuit;
pub use engine::operators::{JoinCondition, Operator, OrderSpec, Predicate, Projection};
pub use engine::types::{FastMap, Path, RowKey, SpookyValue, VersionMap, Weight, ZSet};
pub use engine::update::{MaterializedViewUpdate, ViewResultFormat, ViewUpdate};
pub use engine::view::QueryPlan;
