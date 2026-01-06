pub mod converter;
pub mod engine;
pub mod sanitizer;
pub mod service;

use serde_json::Value;

pub use engine::lazy_circuit::LazyCircuit;
pub use engine::standard_circuit::StandardCircuit;
// pub use engine::circuit::Circuit; // Deprecated/Alias
pub use engine::view::MaterializedViewUpdate;
pub use engine::view::QueryPlan;

use crate::engine::store::Store;

pub trait StreamProcessor {
    fn ingest_record(
        &mut self,
        store: &dyn Store,
        table: String,
        op: String,
        id: String,
        record: Value,
        hash: String,
    ) -> Vec<MaterializedViewUpdate>;

    fn register_view(
        &mut self,
        store: &dyn Store,
        plan: QueryPlan,
        params: Option<Value>,
    ) -> Option<MaterializedViewUpdate>;

    fn unregister_view(&mut self, id: &str);
}
