//! Native port of `ssp-wasm`'s `Sp00kyProcessor`.
//!
//! This is a near line-for-line port of `packages/ssp-wasm/src/lib.rs` with
//! `JsValue` / `serde_wasm_bindgen` swapped for `serde_json::Value`, so the
//! same circuit logic is reachable from Dart over a C ABI (see `lib.rs`).

use anyhow::{anyhow, Result};
use serde::Serialize;
use serde_json::Value;
use ssp::circuit::{Change, ChangeSet, Circuit, Operation, ViewDelta};
use ssp::eval::normalize_record_id;
use ssp::types::Sp00kyValue;

/// Per-record delta info (id + version).
#[derive(Serialize)]
pub struct WasmDeltaRecord(pub String, pub i64);

/// Granular delta: which records were added, removed, or content-updated.
#[derive(Serialize)]
pub struct WasmDelta {
    pub additions: Vec<WasmDeltaRecord>,
    pub removals: Vec<String>,
    pub updates: Vec<WasmDeltaRecord>,
}

/// Custom DTO mirroring the WASM output (`WasmViewUpdate`).
#[derive(Serialize)]
pub struct WasmViewUpdate {
    pub query_id: String,
    pub result_hash: String,
    pub result_data: Vec<(String, i64)>,
    pub delta: WasmDelta,
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

/// Transform a Vec<ViewDelta> to Vec<WasmViewUpdate> with versions from the store.
fn transform_deltas(deltas: &[ViewDelta], circuit: &Circuit) -> Vec<WasmViewUpdate> {
    deltas
        .iter()
        .map(|d| transform_single_delta(d, circuit))
        .collect()
}

pub struct Processor {
    circuit: Circuit,
}

impl Processor {
    pub fn new() -> Processor {
        Processor {
            circuit: Circuit::new(),
        }
    }

    /// Ingest a record change into the stream processor.
    pub fn ingest(
        &mut self,
        table: &str,
        op: &str,
        id: &str,
        record: Value,
    ) -> Result<Vec<WasmViewUpdate>> {
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
                // stripping the table prefix if present (e.g. "thread:abc" -> "abc").
                ssp::types::raw_id(id).to_string()
            });

        let op_enum = Operation::from_str(op).unwrap_or(Operation::Create);

        let change = match op_enum {
            Operation::Create => Change::create(table, &record_id, clean_sv),
            Operation::Update => Change::update(table, &record_id, clean_sv),
            Operation::Delete => Change::delete(table, &record_id),
        };

        let changeset = ChangeSet {
            changes: vec![change],
        };

        let deltas = self.circuit.step(changeset);

        Ok(transform_deltas(&deltas, &self.circuit))
    }

    /// Register a new materialized view.
    pub fn register_view(&mut self, config: Value) -> Result<WasmViewUpdate> {
        let data = ssp::service::view::prepare_registration_dbsp(
            config,
            self.circuit.permissions(),
            self.circuit.link_targets(),
            self.circuit.opaque_fields(),
        )
        .map_err(|e| anyhow!("Registration failed: {}", e))?;

        let plan_id = data.plan.id.clone();
        let initial_delta = self
            .circuit
            .add_query(data.plan, data.safe_params, data.format);

        let result = match initial_delta {
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

        Ok(result)
    }

    /// Unregister a view by ID.
    pub fn unregister_view(&mut self, id: &str) {
        self.circuit.remove_query(id);
    }

    /// Register a table's raw `PERMISSIONS FOR select WHERE <expr>` text.
    ///
    /// The browser client relies on the deployed circuit already being
    /// permissive; a native Dart client must instead seed permissions from the
    /// schema (the way the SSP server does at boot) so `register_view` does not
    /// hit the circuit's default-deny. See `Circuit::set_permission`.
    pub fn set_permission(&mut self, table: &str, where_text: &str) {
        self.circuit.set_permission(table, where_text);
    }

    /// Save the current circuit state as a JSON string.
    pub fn save_state(&self) -> Result<String> {
        self.circuit
            .save()
            .map_err(|e| anyhow!("Failed to serialize state: {}", e))
    }

    /// Load circuit state from a JSON string.
    pub fn load_state(&mut self, state: &str) -> Result<()> {
        let circuit =
            Circuit::restore(state).map_err(|e| anyhow!("Failed to deserialize state: {}", e))?;
        self.circuit = circuit;
        Ok(())
    }
}
