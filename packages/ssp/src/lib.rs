// src/lib.rs

#[cfg(not(target_arch = "wasm32"))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

pub mod converter;
pub mod engine; // <--- Das ist wichtig für den Test
                // pub mod repro_test; // Commented out: file not found
pub mod sanitizer;
pub mod logging;
pub mod service;

#[cfg(all(feature = "parallel", not(target_arch = "wasm32")))]
pub use rayon::prelude::*;

// Falls du noch StreamProcessor Traits hast, müssen die auch hier sein:
pub use engine::circuit::Circuit;
pub use engine::update::{MaterializedViewUpdate, ViewResultFormat, ViewUpdate};
pub use engine::view::QueryPlan;
use serde_json::Value;

pub trait StreamProcessor: Send + Sync {
    fn ingest_record(
        &mut self,
        table: &str,
        op: &str,
        id: &str,
        record: Value,
        hash: &str,
        is_optimistic: bool,
    ) -> Vec<ViewUpdate>;

    fn ingest_batch(
        &mut self,
        batch: Vec<(String, String, String, Value, String)>,
        is_optimistic: bool,
    ) -> Vec<ViewUpdate>;

    fn register_view(
        &mut self,
        plan: QueryPlan,
        params: Option<Value>,
        format: Option<ViewResultFormat>,
    ) -> Option<ViewUpdate>;

    fn unregister_view(&mut self, id: &str);
}
