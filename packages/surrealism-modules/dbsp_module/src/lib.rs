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
    let (clean_record, hash) = spooky_stream_processor::service::ingest::prepare(record);

    // B. Run Engine
    let _ = with_circuit(|circuit| {
        let updates = circuit.ingest_record(table, operation, id, clean_record, hash);

        // C. Apply Updates Directly (Side Effects)
        for update in updates {
            persistence::apply_incantation_update(
                &update.query_id,
                &update.result_hash,
                &update.tree,
            );
        }

        // D. Save State
        persistence::save(circuit);
        Vec::<MaterializedViewUpdate>::new()
    })?;

    // Return success but no updates payload (managed internally now)
    Ok(serde_json::to_value(json!({ "updates": [] })).unwrap())
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
        let initial_res = circuit.register_view(plan.clone(), data.safe_params);

        // Standard default result handling
        let res = initial_res
            .unwrap_or_else(|| spooky_stream_processor::service::view::default_result(&plan.id));

        // Extract result data
        let hash = res.result_hash.clone();
        let tree = res.tree.clone();

        // Persist to SurrealDB directly using prepared metadata
        // Note: We need to extract fields from metadata map for the specific persistence signature or update persistence to take map.
        // Existing persistence signature takes individual args. Let's unpack data.metadata.
        // data.metadata["id"] etc.

        let m = &data.metadata;
        persistence::upsert_incantation(
            m["id"].as_str().unwrap(),
            &hash,
            &tree,
            m["clientId"].as_str().unwrap(),
            m["surrealQL"].as_str().unwrap(),
            &m["safe_params"], // Use the safe params we stored
            m["ttl"].as_str().unwrap(),
            m["lastActiveAt"].as_str().unwrap(),
        );

        persistence::save(circuit);

        // Return hash and tree
        json!({
            "hash": hash,
            "tree": tree
        })
    })?;

    Ok(result)
}

#[surrealism]
fn unregister_view(id: String) -> Result<Value, &'static str> {
    let _ = with_circuit(|circuit| {
        circuit.unregister_view(&id);
        persistence::save(circuit);
    })?;
    Ok(json!({ "msg": "Unregistered", "id": id }))
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
