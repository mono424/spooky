use serde_json::{json, Value};
use std::sync::Mutex;
use surrealism::surrealism;

// 1. Declare Modules
mod converter;
mod engine;
mod persistence;
mod sanitizer;

use engine::Circuit;

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
    let updates = with_circuit(|circuit| {
        let result = circuit.ingest_record(table, operation, id, clean_record, hash);
        // C. Save (Inside lock to ensure consistency)
        persistence::save(circuit);
        result
    })?;

    Ok(serde_json::to_value(json!({ "updates": updates })).unwrap())
}

#[surrealism]
fn register_view(id: String, plan_val: Value, params: Value) -> Result<Value, &'static str> {
    // A. Parse
    let plan_json = match plan_val {
        Value::String(ref s) => s.as_str(),
        _ => return Err("Plan must be a string"),
    };

    // Use the existing converter, now cleaner to call
    // Note: converter might not be pub in mod?
    // If it's `mod converter`, we need `pub mod` or `use converter::...` if functions are pub.
    // Assuming converter functions are pub.
    let root_op_val = converter::convert_surql_to_dbsp(plan_json)
        .or_else(|_| serde_json::from_str(plan_json))
        .map_err(|_| "Invalid Query Plan")?;

    let root_op: engine::view::Operator =
        serde_json::from_value(root_op_val).map_err(|_| "Failed to map JSON to Operator")?;

    let safe_params = sanitizer::parse_params(params);

    // B. Run Engine
    let update = with_circuit(|circuit| {
        // Construct Plan struct here or inside engine
        let plan = engine::view::QueryPlan {
            id: id.clone(),
            root: root_op,
        };
        let initial_res = circuit.register_view(plan, safe_params);
        persistence::save(circuit);
        // Return initial update if needed
        json!({ "msg": "Registered", "id": id, "result": initial_res })
    })?;

    Ok(update)
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
