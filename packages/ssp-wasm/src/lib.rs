use serde::Serialize;
use serde_json::Value;
use ssp::{Circuit, ViewUpdate};
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

#[wasm_bindgen(typescript_custom_section)]
const TS_APPEND_CONTENT: &'static str = r#"
export interface WasmStreamUpdate {
  query_id: string;
  result_hash: string;
  result_data: [string, number][];
}

export interface WasmIncantationConfig {
  id: string;
  surrealQL: string;
  params?: Record<string, any>;
  clientId: string;
  ttl: string;
  lastActiveAt: string;
}

export interface WasmIngestItem {
  table: string;
  op: string;
  id: string;
  record: any;
}
"#;

#[wasm_bindgen]
impl SpookyProcessor {
    #[wasm_bindgen(constructor)]
    pub fn new() -> SpookyProcessor {
        SpookyProcessor {
            circuit: Circuit::new(),
        }
    }

    /// Ingest a record into the stream processor
    /// is_optimistic: true = local mutation (increment versions), false = remote sync (keep versions)
    pub fn ingest(
        &mut self,
        table: String,
        op: String,
        id: String,
        record: JsValue,
        is_optimistic: bool,
    ) -> Result<JsValue, JsValue> {
        let record: Value = serde_wasm_bindgen::from_value(record)
            .map_err(|e| JsValue::from_str(&format!("Failed to parse record: {}", e)))?;

        // internal preparation (hash calc)
        let (clean_record, hash) = ssp::service::ingest::prepare(record);

        let updates =
            self.circuit
                .ingest_record(&table, &op, &id, clean_record.into(), &hash, is_optimistic);

        // Use Serializer with serialize_maps_as_objects(true) to output plain JS objects
        // instead of JS Map objects (which stringify as {} for HashMap)
        let serializer = serde_wasm_bindgen::Serializer::new().serialize_maps_as_objects(true);
        Ok(updates.serialize(&serializer)?)
    }

    /// Ingest multiple records into the stream processor in a single batch.
    /// This is more efficient than calling ingest() multiple times as it:
    /// 1. Processes all records together
    /// 2. Emits a single set of view updates
    /// is_optimistic: true = local mutation (increment versions), false = remote sync (keep versions)
    pub fn ingest_batch(
        &mut self,
        batch: JsValue, // Array of { table, op, id, record }
        is_optimistic: bool,
    ) -> Result<JsValue, JsValue> {
        let batch_array: Vec<Value> = serde_wasm_bindgen::from_value(batch)
            .map_err(|e| JsValue::from_str(&format!("Failed to parse batch: {}", e)))?;

        let mut prepared_batch: Vec<(String, String, String, Value, String)> =
            Vec::with_capacity(batch_array.len());

        for item in batch_array {
            let table = item
                .get("table")
                .and_then(|v| v.as_str())
                .ok_or_else(|| JsValue::from_str("Missing 'table' field"))?
                .to_string();
            let op = item
                .get("op")
                .and_then(|v| v.as_str())
                .ok_or_else(|| JsValue::from_str("Missing 'op' field"))?
                .to_string();
            let id = item
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| JsValue::from_str("Missing 'id' field"))?
                .to_string();
            let record = item.get("record").cloned().unwrap_or(Value::Null);

            let (clean_record, hash) = ssp::service::ingest::prepare(record);
            prepared_batch.push((table, op, id, clean_record.into(), hash));
        }

        let updates = self.circuit.ingest_batch(prepared_batch, is_optimistic);

        let serializer = serde_wasm_bindgen::Serializer::new().serialize_maps_as_objects(true);
        Ok(updates.serialize(&serializer)?)
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

        let data = ssp::service::view::prepare_registration(config_val)
            .map_err(|e| JsValue::from_str(&format!("Registration failed: {}", e)))?;

        // Extract plan ID before moving the plan
        let plan_id = data.plan.id.clone();
        let initial_update = self
            .circuit
            .register_view(data.plan, data.safe_params, data.format);

        // If None, return default empty result
        let result = initial_update
            .unwrap_or_else(|| ViewUpdate::Flat(ssp::service::view::default_result(&plan_id)));

        // Use Serializer with serialize_maps_as_objects(true) to output plain JS objects
        let serializer = serde_wasm_bindgen::Serializer::new().serialize_maps_as_objects(true);
        Ok(result.serialize(&serializer)?)
    }

    /// Unregister a view by ID
    pub fn unregister_view(&mut self, id: String) {
        self.circuit.unregister_view(&id);
    }

    /// Explicitly set the version of a record for a specific view
    pub fn set_record_version(
        &mut self,
        incantation_id: String,
        record_id: String,
        version: f64, // JS numbers are f64, cast to u64
    ) -> Result<JsValue, JsValue> {
        let ver_u64 = version as u64;
        let update = self
            .circuit
            .set_record_version(&incantation_id, &record_id, ver_u64);

        if let Some(up) = update {
            let serializer = serde_wasm_bindgen::Serializer::new().serialize_maps_as_objects(true);
            Ok(up.serialize(&serializer)?)
        } else {
            Ok(JsValue::NULL)
        }
    }

    /// Save the current circuit state as a JSON string
    pub fn save_state(&self) -> Result<String, JsValue> {
        serde_json::to_string(&self.circuit)
            .map_err(|e| JsValue::from_str(&format!("Failed to serialize state: {}", e)))
    }

    /// Load circuit state from a JSON string
    pub fn load_state(&mut self, state: String) -> Result<(), JsValue> {
        let circuit: Circuit = serde_json::from_str(&state)
            .map_err(|e| JsValue::from_str(&format!("Failed to deserialize state: {}", e)))?;

        // The circuit needs to rebuild dependency graph after deserialization
        self.circuit = circuit;
        self.circuit.rebuild_dependency_graph();

        Ok(())
    }
}
