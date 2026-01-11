// src/lib.rs

#[cfg(not(target_arch = "wasm32"))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

pub mod converter;
pub mod engine; // <--- Das ist wichtig für den Test
pub mod sanitizer;
pub mod service;

// Falls du noch StreamProcessor Traits hast, müssen die auch hier sein:
pub use engine::circuit::Circuit;
pub use engine::view::MaterializedViewUpdate;
pub use engine::view::QueryPlan;
use serde_json::Value;

pub trait StreamProcessor: Send + Sync {
    fn ingest_record(
        &mut self,
        table: String,
        op: String,
        id: String,
        record: Value,
        hash: String,
    ) -> Vec<MaterializedViewUpdate>;

    fn register_view(
        &mut self,
        plan: QueryPlan,
        params: Option<Value>,
    ) -> Option<MaterializedViewUpdate>;

    fn unregister_view(&mut self, id: &str);
}
