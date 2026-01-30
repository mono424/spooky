use serde::Serialize;
use serde_json::Value; // Keep Value import
use ssp::engine::circuit::dto::BatchEntry;
use ssp::engine::eval::normalize_record_id;
use ssp::engine::types::Operation;
use ssp::engine::types::SpookyValue;
use ssp::{Circuit, ViewResultFormat, ViewUpdate};
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
        _ => {
            // Log warning or panic depending on strictness.
            // Since user said "we only get ViewUpdate::Streaiming", we'll just return empty or panic.
            // Returning empty safe default for other formats if they accidentally slip in.
            // But realistically should not happen if we register with Streaming.
            let query_id = update.query_id().to_string();
             WasmViewUpdate {
                query_id,
                result_hash: String::new(),
                result_data: vec![],
                delta: None,
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
    pub fn ingest_single(
        &mut self,
        table: String,
        op: String,
        id: String,
        record: JsValue,
    ) -> Result<JsValue, JsValue> {
        let record: Value = serde_wasm_bindgen::from_value(record)
            .map_err(|e| JsValue::from_str(&format!("Failed to parse record: {}", e)))?;
         web_sys::console::log_1(&format!("[ssp-wasm] ingest").into());
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



    /// Register a new materialized view
    pub fn register_view(&mut self, config: JsValue) -> Result<JsValue, JsValue> {
        let config_val: Value = serde_wasm_bindgen::from_value(config)
            .map_err(|e| JsValue::from_str(&format!("Failed to parse config: {}", e)))?;

        let data = ssp::service::view::prepare_registration(config_val)
            .map_err(|e| JsValue::from_str(&format!("Registration failed: {}", e)))?;

        let plan_id = data.plan.id.clone();
        
        // Force Streaming format regardless of what might have been parsed
        let initial_update = self
            .circuit
            .register_view(data.plan, data.safe_params, Some(ViewResultFormat::Streaming));

        // If None, return default empty result
        // For streaming, default might be empty records
        let result = initial_update.unwrap_or_else(|| {
             // Return simplified default for streaming? Or just re-use service default and let transform handle it?
             // service::default_result returns MaterializedViewUpdate (Flat).
             // Let's manually construct an empty Streaming update to be consistent.
             ViewUpdate::Streaming(ssp::engine::update::StreamingUpdate {
                 view_id: plan_id,
                 records: vec![],
             })
        });

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
