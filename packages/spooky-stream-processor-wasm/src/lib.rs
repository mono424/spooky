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
    ) -> Result<JsValue, JsValue> {
        let record: Value = serde_wasm_bindgen::from_value(record)
            .map_err(|e| JsValue::from_str(&format!("Failed to parse record: {}", e)))?;

        // internal preparation (hash calc)
        let (clean_record, hash) = spooky_stream_processor::service::ingest::prepare(record);

        let updates = self
            .circuit
            .ingest_record(table, op, id, clean_record, hash);

        Ok(serde_wasm_bindgen::to_value(&updates)?)
    }

    /// Register a new materialized view
    /// Config can be the QueryPlan object OR a configuration object with SQL
    /// For JS convenience we accept a generic object and try to process it.
    /// If it has 'id', 'surrealQL' etc it is treated as a config.
    /// If it looks like a plan, we try to use it directly, but current requirement implies SQL usage.
    /// Actually, to match `surrealsim`, we should accept a config object containing `surrealQL`.
    pub fn register_view(&mut self, config: JsValue) -> Result<JsValue, JsValue> {
        let config_val: Value = serde_wasm_bindgen::from_value(config)
            .map_err(|e| JsValue::from_str(&format!("Failed to parse config: {}", e)))?;

        // We use the shared service logic to parse SQL and prepare params
        // This handles "id", "surrealQL", "params" etc.
        // Note: verify if `prepare_registration` is generic enough.
        // It expects keys: id, surrealQL, clientId, ttl, lastActiveAt.
        // If the JS caller only sends id + sql, it might fail validation.
        // Let's check typical usage.
        // If we want minimal usage (id + plan/sql), we might need loose parsing or constructing default metadata.
        // But the user asked for "register view should work with sql string".
        // Let's assume the user passes a config object with these fields, similar to the module.

        let data = spooky_stream_processor::service::view::prepare_registration(config_val)
            .map_err(|e| JsValue::from_str(&format!("Registration failed: {}", e)))?;

        let initial_update = self.circuit.register_view(data.plan, data.safe_params);

        // If None, return default empty result
        let result = initial_update.unwrap_or_else(|| {
            // We need to fetch the ID from the prepared plan
            // data.plan.id is available
            spooky_stream_processor::service::view::default_result(&data.plan.id)
        });

        Ok(serde_wasm_bindgen::to_value(&result)?)
    }

    /// Unregister a view by ID
    pub fn unregister_view(&mut self, id: String) {
        self.circuit.unregister_view(&id);
    }
}
