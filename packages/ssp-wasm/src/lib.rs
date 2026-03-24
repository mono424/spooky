use serde::Serialize;
use serde_json::Value;
use ssp::circuit::{Change, ChangeSet, Circuit, Operation, ViewDelta};
use ssp::eval::normalize_record_id;
use ssp::types::Sp00kyValue;
use wasm_bindgen::prelude::*;

/// Version from Cargo.toml
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Called when WASM module is loaded
#[wasm_bindgen(start)]
pub fn init() {
    web_sys::console::log_1(&format!("[ssp-wasm] v{} loaded", VERSION).into());
}

#[wasm_bindgen]
pub struct Sp00kyProcessor {
    circuit: Circuit,
}

/// Per-record delta info (id + version).
#[derive(Serialize)]
struct WasmDeltaRecord(String, i64);

/// Granular delta: which records were added, removed, or content-updated.
#[derive(Serialize)]
struct WasmDelta {
    additions: Vec<WasmDeltaRecord>,
    removals: Vec<String>,
    updates: Vec<WasmDeltaRecord>,
}

/// Custom DTO for WASM output.
#[derive(Serialize)]
struct WasmViewUpdate {
    query_id: String,
    result_hash: String,
    result_data: Vec<(String, i64)>,
    delta: WasmDelta,
}

/// Transform a Vec<ViewDelta> to Vec<WasmViewUpdate> with versions from the store.
fn transform_deltas(deltas: &[ViewDelta], circuit: &Circuit) -> Vec<WasmViewUpdate> {
    deltas
        .iter()
        .map(|d| transform_single_delta(d, circuit))
        .collect()
}

/// Resolve version for a key from the store (defaults to 1).
fn version_for(circuit: &Circuit, key: &str) -> i64 {
    circuit.store.get_record_version_by_key(key).unwrap_or(1)
}

/// Transform a single ViewDelta to WasmViewUpdate.
fn transform_single_delta(delta: &ViewDelta, circuit: &Circuit) -> WasmViewUpdate {
    let result_data: Vec<(String, i64)> = delta
        .records
        .iter()
        .map(|key| (key.clone(), version_for(circuit, key)))
        .collect();

    let additions: Vec<WasmDeltaRecord> = delta
        .additions
        .iter()
        .map(|key| WasmDeltaRecord(key.clone(), version_for(circuit, key)))
        .collect();

    let removals: Vec<String> = delta.removals.clone();

    let updates: Vec<WasmDeltaRecord> = delta
        .updates
        .iter()
        .map(|key| WasmDeltaRecord(key.clone(), version_for(circuit, key)))
        .collect();

    WasmViewUpdate {
        query_id: delta.query_id.clone(),
        result_hash: delta.result_hash.clone(),
        result_data,
        delta: WasmDelta {
            additions,
            removals,
            updates,
        },
    }
}

// This is appended to the generated .d.ts file
#[wasm_bindgen(typescript_custom_section)]
const TS_APPEND_CONTENT: &'static str = r#"
export interface WasmViewUpdate {
  query_id: string;
  result_hash: string;
  result_data: [string, number][];
  delta: {
    additions: [string, number][];
    removals: string[];
    updates: [string, number][];
  };
}

export interface WasmViewConfig {
  id: string;
  surql: string;
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
impl Sp00kyProcessor {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Sp00kyProcessor {
        Sp00kyProcessor {
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

        // Normalize the record and convert to new Sp00kyValue
        let clean_record = ssp::sanitizer::normalize_record(record);
        let clean_sv: Sp00kyValue = clean_record.into();

        let record_id = clean_sv
            .get("id")
            .cloned()
            .map(normalize_record_id)
            .and_then(|v| match v {
                Sp00kyValue::Str(s) => Some(s.to_string()),
                _ => None,
            })
            .unwrap_or_else(|| {
                // Fallback: extract raw id from the passed `id` param,
                // stripping the table prefix if present (e.g. "thread:abc" → "abc").
                ssp::types::raw_id(&id).to_string()
            });

        let op_enum = Operation::from_str(&op).unwrap_or(Operation::Create);

        let change = match op_enum {
            Operation::Create => Change::create(&table, &record_id, clean_sv),
            Operation::Update => Change::update(&table, &record_id, clean_sv),
            Operation::Delete => Change::delete(&table, &record_id),
        };

        let changeset = ChangeSet {
            changes: vec![change],
        };

        let deltas = self.circuit.step(changeset);

        // Transform to include versions
        let wasm_updates = transform_deltas(&deltas, &self.circuit);

        let serializer = serde_wasm_bindgen::Serializer::new().serialize_maps_as_objects(true);
        Ok(wasm_updates.serialize(&serializer)?)
    }

    /// Register a new materialized view
    pub fn register_view(&mut self, config: JsValue) -> Result<JsValue, JsValue> {
        let config_val: Value = serde_wasm_bindgen::from_value(config)
            .map_err(|e| JsValue::from_str(&format!("Failed to parse config: {}", e)))?;

        let data = ssp::service::view::prepare_registration_dbsp(config_val)
            .map_err(|e| JsValue::from_str(&format!("Registration failed: {}", e)))?;

        let plan_id = data.plan.id.clone();
        let initial_delta = self
            .circuit
            .add_query(data.plan, data.safe_params, data.format);

        let wasm_result = match initial_delta {
            Some(ref delta) => transform_single_delta(delta, &self.circuit),
            None => WasmViewUpdate {
                query_id: plan_id,
                result_hash: String::new(),
                result_data: vec![],
                delta: WasmDelta {
                    additions: vec![],
                    removals: vec![],
                    updates: vec![],
                },
            },
        };

        let serializer = serde_wasm_bindgen::Serializer::new().serialize_maps_as_objects(true);
        Ok(wasm_result.serialize(&serializer)?)
    }

    /// Unregister a view by ID
    pub fn unregister_view(&mut self, id: String) {
        self.circuit.remove_query(&id);
    }

    /// Save the current circuit state as a JSON string
    pub fn save_state(&self) -> Result<String, JsValue> {
        self.circuit
            .save()
            .map_err(|e| JsValue::from_str(&format!("Failed to serialize state: {}", e)))
    }

    /// Load circuit state from a JSON string
    pub fn load_state(&mut self, state: String) -> Result<(), JsValue> {
        let circuit = Circuit::restore(&state)
            .map_err(|e| JsValue::from_str(&format!("Failed to deserialize state: {}", e)))?;

        self.circuit = circuit;
        Ok(())
    }
}
