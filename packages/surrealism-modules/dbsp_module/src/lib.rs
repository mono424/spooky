use serde_json::{json, Value};
use std::sync::Mutex;
use surrealism::surrealism;

// 1. Declare Modules
mod persistence;

use spooky_stream_processor::{Circuit, MaterializedViewUpdate};

// 2. Global State Wrapper
lazy_static::lazy_static! {
    static ref CIRCUIT: Mutex<Option<Circuit>> = Mutex::new(None);
}

// Helper to get circuit access
fn with_circuit<F, R>(f: F) -> Result<R, &'static str>
where
    F: FnOnce(&mut Circuit) -> R,
{
    let mut lock = CIRCUIT.lock().map_err(|_| "Failed to lock")?;
    if lock.is_none() {
        *lock = Some(persistence::load());
    }
    Ok(f(lock.as_mut().unwrap()))
}

// 3. Clean Macros

#[surrealism]
fn ingest(
    table: String,
    operation: String,
    id: String,
    record: Value,
) -> Result<Value, &'static str> {
    // A. Prepare (Normalize & Hash) using centralized logic
    // Use ssp alias correctly
    let (clean_record, hash) = spooky_stream_processor::service::ingest::prepare(record);

    // B. Run Engine
    let _ = with_circuit(|circuit| {
        // Pass borrowed strings and add is_optimistic=true
        let updates =
            circuit.ingest_record(&table, &operation, &id, clean_record.into(), &hash, true);

        // C. Apply Updates Directly (Side Effects)
        for update in updates {
            // Unpack ViewUpdate enum
            match update {
                spooky_stream_processor::ViewUpdate::Flat(flat)
                | spooky_stream_processor::ViewUpdate::Tree(flat) => {
                    persistence::apply_incantation_update(
                        &flat.query_id,
                        &flat.result_hash,
                        &flat.result_data,
                    );
                }
                spooky_stream_processor::ViewUpdate::Streaming(_) => {
                    // Streaming not supported for persistence yet
                    eprintln!("DEBUG: Streaming update ignored for persistence");
                }
            }
        }

        // D. Save State
        persistence::save(circuit);
        Vec::<MaterializedViewUpdate>::new()
    })?;

    // Return success but no updates payload (managed internally now)
    Ok(Value::Null)
}

#[surrealism]
fn version(_args: Value) -> Result<Value, &'static str> {
    Ok(json!("0.1.3-refactor")) // Increment for visibility
}

#[surrealism]
fn register_view(config: Value) -> Result<Value, &'static str> {
    // Use centralized preparation
    let data =
        spooky_stream_processor::service::view::prepare_registration(config).map_err(|e| {
            eprintln!("DEBUG: prepare_registration failed: {}", e);
            "Invalid Configuration"
        })?;

    let result = with_circuit(|circuit| {
        let plan = data.plan;
        // Pass None for format (defaults to Flat)
        let initial_res = circuit.register_view(plan.clone(), data.safe_params, None);

        // Standard default result handling. Default result is now typically Flat.
        // We manually construct a Flat enum wrapper for the default.
        // Need to import ViewUpdate to construct it? Or just use helper.
        // Let's assume we can use the unwrapped value if present.

        // Helper to extract data from ViewUpdate
        let extract =
            |u: spooky_stream_processor::ViewUpdate| -> (String, String, Vec<(String, u64)>) {
                match u {
                    spooky_stream_processor::ViewUpdate::Flat(f)
                    | spooky_stream_processor::ViewUpdate::Tree(f) => {
                        (f.query_id, f.result_hash, f.result_data)
                    }
                    _ => (String::new(), String::new(), Vec::new()),
                }
            };

        let (query_id, hash, result_data) = if let Some(u) = initial_res {
            extract(u)
        } else {
            let def = spooky_stream_processor::service::view::default_result(&plan.id);
            (def.query_id, def.result_hash, def.result_data)
        };

        // Persist to SurrealDB directly using prepared metadata
        // Note: We need to extract fields from metadata map for the specific persistence signature or update persistence to take map.
        // Existing persistence signature takes individual args. Let's unpack data.metadata.
        // data.metadata["id"] etc.

        let m = &data.metadata;
        persistence::upsert_incantation(
            m["id"].as_str().unwrap(),
            &hash,
            &result_data,
            m["clientId"].as_str().unwrap(),
            m["surrealQL"].as_str().unwrap(),
            &m["safe_params"], // Use the safe params we stored
            m["ttl"].as_str().unwrap(),
            m["lastActiveAt"].as_str().unwrap(),
        );

        persistence::save(circuit);

        // Return nothing, just updating internal state
        Value::Null
    })?;

    Ok(Value::Null)
}

#[surrealism]
fn unregister_view(id: String) -> Result<Value, &'static str> {
    let _ = with_circuit(|circuit| {
        circuit.unregister_view(&id);
        persistence::save(circuit);
    })?;
    Ok(Value::Null)
}

#[surrealism]
fn reset(_val: Value) -> Result<Value, &'static str> {
    let mut lock = CIRCUIT.lock().map_err(|_| "Failed to lock")?;
    *lock = Some(Circuit::new());
    persistence::clear();
    Ok(Value::Null)
}

#[surrealism]
fn save_state(_val: Value) -> Result<Value, &'static str> {
    let _ = with_circuit(|circuit| {
        persistence::save(circuit);
    })?;
    Ok(Value::Null)
}
