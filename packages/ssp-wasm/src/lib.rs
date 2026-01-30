use serde::Serialize;
use serde_json::Value; // Keep Value import
use ssp::engine::circuit::dto::BatchEntry;
use ssp::engine::eval::normalize_record_id;
use ssp::engine::types::Operation;
use ssp::engine::types::SpookyValue;
use ssp::{Circuit, ViewUpdate};
use wasm_bindgen::prelude::*;

/// Version from Cargo.toml
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Called when WASM module is loaded
#[wasm_bindgen(start)]
pub fn init() {
    web_sys::console::log_1(&format!("[ssp-wasm] v{} loaded", VERSION).into());
}

#[wasm_bindgen]
pub struct SpookyProcessor {
    circuit: Circuit,
}

/// Custom DTO for WASM output with [[record-id, version], ...] format
/// Custom DTO for WASM output with [[record-id, version], ...] format
#[derive(Serialize)]
struct WasmViewUpdate {
    query_id: String,
    result_hash: String,
    result_data: Vec<(String, i64)>,
    #[serde(skip_serializing_if = "Option::is_none")]
    delta: Option<WasmStreamingUpdate>,
}

#[derive(Serialize)]
struct WasmStreamingUpdate {
    view_id: String,
    records: Vec<WasmDeltaRecord>,
}

#[derive(Serialize)]
struct WasmDeltaRecord {
    id: String,
    event: ssp::engine::update::DeltaEvent,
    version: i64,
}

/// Transform ViewUpdate to WasmViewUpdate with versions extracted from circuit database
fn transform_updates(updates: &[ViewUpdate], circuit: &Circuit) -> Vec<WasmViewUpdate> {
    updates
        .iter()
        .map(|update| transform_single_update(update.clone(), circuit))
        .collect()
}

/// Transform a single ViewUpdate
fn transform_single_update(update: ViewUpdate, circuit: &Circuit) -> WasmViewUpdate {
    match update {
        ViewUpdate::Flat(m) | ViewUpdate::Tree(m) => {
            let result_data: Vec<(String, i64)> = m
                .result_data
                .iter()
                .map(|id| {
                    let table_name = id.split(':').next().unwrap_or("");
                    let version = circuit
                        .db
                        .get_table(table_name)
                        .and_then(|t| t.get_record_version(id))
                        .unwrap_or(1);
                    (id.to_string(), version)
                })
                .collect();

            WasmViewUpdate {
                query_id: m.query_id.clone(),
                result_hash: m.result_hash.clone(),
                result_data,
                delta: None,
            }
        }
        ViewUpdate::Streaming(s) => {
            let records: Vec<WasmDeltaRecord> = s
                .records
                .iter()
                .map(|rec| {
                    let table_name = rec.id.split(':').next().unwrap_or("");
                    let version = circuit
                        .db
                        .get_table(table_name)
                        .and_then(|t| t.get_record_version(&rec.id))
                        .unwrap_or(1);

                    WasmDeltaRecord {
                        id: rec.id.to_string(),
                        event: rec.event.clone(),
                        version,
                    }
                })
                .collect();

            WasmViewUpdate {
                query_id: s.view_id.clone(),
                result_hash: String::new(),
                result_data: vec![],
                delta: Some(WasmStreamingUpdate {
                    view_id: s.view_id.clone(),
                    records,
                }),
            }
        }
    }
}

// This is appended to the generated .d.ts file
#[wasm_bindgen(typescript_custom_section)]
const TS_APPEND_CONTENT: &'static str = r#"
export interface WasmStreamUpdate {
  query_id: string;
  result_hash: string;
  result_data: [string, number][];
  delta?: {
    view_id: string;
    records: { id: string; event: any; version: number }[];
  };
}


export interface WasmIncantationConfig {
  id: string;
  sql: string;
  params?: Record<string, any>;
  clientId: string;
  ttl: string;
  lastActiveAt: string;
  safe_params?: Record<string, any>;
  format?: 'flat' | 'tree' | 'streaming';
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
    pub fn ingest(
        &mut self,
        table: String,
        op: String,
        id: String,
        record: JsValue,
        //_is_optimistic: bool, // Ignored in new API? Or todo? New API doesn't seem to take it in ingest_single
    ) -> Result<JsValue, JsValue> {
        let record: Value = serde_wasm_bindgen::from_value(record)
            .map_err(|e| JsValue::from_str(&format!("Failed to parse record: {}", e)))?;

        // internal preparation (hash calc)
        let (clean_record, _hash) = ssp::service::ingest::prepare(record);

        let record_id = clean_record
            .get("id")
            .cloned()
            .map(normalize_record_id)
            .and_then(|v| match v {
                SpookyValue::Str(s) => Some(s.to_string()),
                _ => None,
            })
            .unwrap_or_else(|| {
                tracing::warn!(
                    target: "ssp::ingest",
                    table = table,
                    "Could not extract record ID from clean_record"
                );
                // This fallback should rarely/never happen now
                format!("{}:{}", table, id)
            });

        let op_enum = Operation::from_str(&op).unwrap_or(Operation::Create);

        // Convert clean_record (Value) to SpookyValue
        let data: SpookyValue = clean_record.into();

        let entry = BatchEntry::new(&table, op_enum, record_id, data);

        let updates = self.circuit.ingest_single(entry);

        // Transform to include versions
        let wasm_updates = transform_updates(&updates, &self.circuit);

        let serializer = serde_wasm_bindgen::Serializer::new().serialize_maps_as_objects(true);
        Ok(wasm_updates.serialize(&serializer)?)
    }

    /// Ingest multiple records into the stream processor in a single batch.
    pub fn ingest_batch(
        &mut self,
        batch: JsValue, // Array of { table, op, id, record }
        _is_optimistic: bool,
    ) -> Result<JsValue, JsValue> {
        let batch_array: Vec<Value> = serde_wasm_bindgen::from_value(batch)
            .map_err(|e| JsValue::from_str(&format!("Failed to parse batch: {}", e)))?;

        let mut entries: Vec<BatchEntry> = Vec::with_capacity(batch_array.len());

        for item in batch_array {
            let table = item
                .get("table")
                .and_then(|v| v.as_str())
                .ok_or_else(|| JsValue::from_str("Missing 'table' field"))?
                .to_string();
            let op_str = item
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

            let (clean_record, _hash) = ssp::service::ingest::prepare(record);

            let record_id = clean_record
                .get("id")
                .cloned()
                .map(normalize_record_id)
                .and_then(|v| match v {
                    SpookyValue::Str(s) => Some(s.to_string()),
                    _ => None,
                })
                .unwrap_or_else(|| {
                    tracing::warn!(
                        target: "ssp::ingest",
                        table = table,
                        "Could not extract record ID from clean_record"
                    );
                    // This fallback should rarely/never happen now
                    format!("{}:{}", table, id)
                });

            let op_enum = Operation::from_str(&op_str).unwrap_or(Operation::Create);
            let data: SpookyValue = clean_record.into();

            entries.push(BatchEntry::new(&table, op_enum, record_id, data));
        }

        let updates = self.circuit.ingest_batch(entries);

        // Transform to include versions
        let wasm_updates = transform_updates(&updates, &self.circuit);

        let serializer = serde_wasm_bindgen::Serializer::new().serialize_maps_as_objects(true);
        Ok(wasm_updates.serialize(&serializer)?)
    }

    /// Register a new materialized view
    pub fn register_view(&mut self, config: JsValue) -> Result<JsValue, JsValue> {
        let config_val: Value = serde_wasm_bindgen::from_value(config)
            .map_err(|e| JsValue::from_str(&format!("Failed to parse config: {}", e)))?;

        let data = ssp::service::view::prepare_registration(config_val)
            .map_err(|e| JsValue::from_str(&format!("Registration failed: {}", e)))?;

        let plan_id = data.plan.id.clone();
        let initial_update = self
            .circuit
            .register_view(data.plan, data.safe_params, data.format);

        // If None, return default empty result
        let result = initial_update
            .unwrap_or_else(|| ViewUpdate::Flat(ssp::service::view::default_result(&plan_id)));

        // Transform to include versions
        let wasm_result = transform_single_update(result, &self.circuit);

        let serializer = serde_wasm_bindgen::Serializer::new().serialize_maps_as_objects(true);
        Ok(wasm_result.serialize(&serializer)?)
    }

    /// Unregister a view by ID
    pub fn unregister_view(&mut self, id: String) {
        self.circuit.unregister_view(&id);
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
        self.circuit.rebuild_dependency_list();

        Ok(())
    }
}
