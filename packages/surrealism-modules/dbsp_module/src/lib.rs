use serde_json::{json, Value};
use std::sync::Mutex;
use surrealism::surrealism;

// 1. Declare Modules
// 1. Declare Modules
mod persistence;

use engine::Circuit;
use spooky_stream_processor::{converter, engine, sanitizer};

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
    // A. Sanitize
    let clean_record = sanitizer::normalize_record(record);

    // Hash the record for integrity/verification
    let hash = blake3::hash(clean_record.to_string().as_bytes())
        .to_hex()
        .to_string();

    // B. Run Engine
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
        Vec::<crate::engine::view::MaterializedViewUpdate>::new()
    })?;

    // Return success but no updates payload (managed internally now)
    Ok(serde_json::to_value(json!({ "updates": [] })).unwrap())
}

#[surrealism]
fn version(_args: Value) -> Result<Value, &'static str> {
    Ok(json!("0.1.1-debug")) // Increment this to verify new build is loaded
}

#[surrealism]
fn register_view(config: Value) -> Result<Value, &'static str> {
    eprintln!("DEBUG: register_view START v0.1.1-debug");
    eprintln!("DEBUG: Received config: {}", config);

    // A. Unpack Config
    let id = config
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or("Missing or invalid 'id'")?
        .to_string();
    let surrealql = config
        .get("surrealQL")
        .and_then(|v| v.as_str())
        .ok_or("Missing or invalid 'surrealQL'")?
        .to_string();
    let params = config.get("params").cloned().unwrap_or(json!({}));
    let client_id = config
        .get("clientId")
        .and_then(|v| v.as_str())
        .ok_or("Missing or invalid 'clientId'")?
        .to_string();
    // Assuming ttl and lastActiveAt are passed formatted as strings (even if Duration/Datetime in Surreal)
    let ttl = config
        .get("ttl")
        .and_then(|v| v.as_str())
        .ok_or("Missing or invalid 'ttl'")?
        .to_string();
    let last_active_at = config
        .get("lastActiveAt")
        .and_then(|v| v.as_str())
        .ok_or("Missing or invalid 'lastActiveAt'")?
        .to_string();

    // B. Parse Query Plan
    let root_op_val = converter::convert_surql_to_dbsp(&surrealql)
        .or_else(|_| serde_json::from_str(&surrealql))
        .map_err(|_| "Invalid Query Plan")?;

    let root_op: engine::view::Operator =
        serde_json::from_value(root_op_val).map_err(|_| "Failed to map JSON to Operator")?;

    let safe_params = sanitizer::parse_params(params.clone());

    // C. Run Engine & Persist
    let result = with_circuit(|circuit| {
        let plan = engine::view::QueryPlan {
            id: id.clone(),
            root: root_op,
        };
        let initial_res = circuit.register_view(plan, safe_params);

        // Unwrap the Option
        let res = initial_res.expect("Failed to get initial view result");

        // Extract result data
        let hash = res.result_hash.clone();
        let tree = res.tree.clone();

        // Persist to SurrealDB directly
        persistence::upsert_incantation(
            &id,
            &hash,
            &tree,
            &client_id,
            &surrealql,
            &params,
            &ttl,
            &last_active_at,
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
