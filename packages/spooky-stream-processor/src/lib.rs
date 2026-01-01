pub mod converter;
pub mod engine;
pub mod sanitizer;

use serde_json::Value;

pub use engine::circuit::Circuit;
pub use engine::view::MaterializedViewUpdate;
pub use engine::view::QueryPlan;

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
