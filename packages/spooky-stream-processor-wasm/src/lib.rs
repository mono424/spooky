use serde_json::Value;
use spooky_stream_processor::{Circuit, MaterializedViewUpdate, QueryPlan, StreamProcessor};
use wasm_bindgen::prelude::*;

// When the `wee_alloc` feature is enabled, use `wee_alloc` as the global
// allocator.
#[cfg(feature = "wee_alloc")]
#[global_allocator]
static ALLOC: wee_alloc::WeeAlloc = wee_alloc::WeeAlloc::INIT;

#[wasm_bindgen]
pub struct SpookyProcessor {
    circuit: Circuit,
}

#[wasm_bindgen]
impl SpookyProcessor {
    #[wasm_bindgen(constructor)]
    pub fn new() -> SpookyProcessor {
        SpookyProcessor {
            circuit: Circuit::new(),
        }
    }

    /// Ingest a record into the stream processor
    pub fn ingest(
        &mut self,
        table: String,
        op: String,
        id: String,
        record: JsValue,
        hash: String,
    ) -> Result<JsValue, JsValue> {
        let record: Value = serde_wasm_bindgen::from_value(record)
            .map_err(|e| JsValue::from_str(&format!("Failed to parse record: {}", e)))?;

        let updates = self.circuit.ingest_record(table, op, id, record, hash);

        Ok(serde_wasm_bindgen::to_value(&updates)?)
    }

    /// Register a new materialized view
    pub fn register_view(&mut self, plan: JsValue, params: JsValue) -> Result<JsValue, JsValue> {
        let plan: QueryPlan = serde_wasm_bindgen::from_value(plan)
            .map_err(|e| JsValue::from_str(&format!("Failed to parse query plan: {}", e)))?;

        let params: Option<Value> = if params.is_null() || params.is_undefined() {
            None
        } else {
            Some(
                serde_wasm_bindgen::from_value(params)
                    .map_err(|e| JsValue::from_str(&format!("Failed to parse params: {}", e)))?,
            )
        };

        let initial_update = self.circuit.register_view(plan, params);

        Ok(serde_wasm_bindgen::to_value(&initial_update)?)
    }

    /// Unregister a view by ID
    pub fn unregister_view(&mut self, id: String) {
        self.circuit.unregister_view(&id);
    }
}
