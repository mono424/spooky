use serde_json::{json, Value};
use std::sync::Mutex;
use surrealism::surrealism;

// 1. Declare Modules
mod persistence;

use spooky_stream_processor::{LazyCircuit, MaterializedViewUpdate};

// 2. Global State Wrapper
lazy_static::lazy_static! {
    static ref CIRCUIT: Mutex<Option<LazyCircuit>> = Mutex::new(None);
}

// Helper to get circuit access
pub fn with_circuit<F, R>(f: F) -> Result<R, String>
where
    F: FnOnce(&mut LazyCircuit) -> R,
{
    // A. Load State (From local variable or DB)
    // NOTE: In SurrealKV Native mode, we don't load "Data", just Views.
    let mut circuit = persistence::load(); // This loads LazyCircuit

    let res = f(&mut circuit);

    // B. Save State (Only if views changed? Standard "save" handles logic)
    persistence::save(&circuit);

    Ok(res)
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
        let store = persistence::SurrealStore::new();
        let updates = circuit.ingest_record(&store, table, operation, id, clean_record, hash);

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
    })
    .map_err(|e| {
        eprintln!("DEBUG: ingest failed: {}", e);
        "Ingest Error"
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

    let _result = with_circuit(|circuit| {
        let store = persistence::SurrealStore::new();
        let plan = data.plan;
        let initial_res = circuit.register_view(&store, plan.clone(), data.safe_params);

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

        // Return nothing, just updating internal state
        Value::Null
    })
    .map_err(|e| {
        eprintln!("DEBUG: register_view failed: {}", e);
        "Register View Error"
    })?;

    Ok(Value::Null)
}

#[surrealism]
fn unregister_view(id: String) -> Result<Value, &'static str> {
    let _ = with_circuit(|circuit| {
        circuit.unregister_view(&id);
        persistence::save(circuit);
    })
    .map_err(|e| {
        eprintln!("DEBUG: unregister_view failed: {}", e);
        "Unregister View Error"
    })?;
    Ok(Value::Null)
}

#[surrealism]
fn reset(_val: Value) -> Result<Value, &'static str> {
    let mut lock = CIRCUIT.lock().map_err(|_| "Failed to lock")?;
    *lock = Some(LazyCircuit::new());
    persistence::clear();
    Ok(Value::Null)
}

#[surrealism]
fn save_state(_val: Value) -> Result<Value, &'static str> {
    let _ = with_circuit(|circuit| {
        persistence::save(circuit);
    })
    .map_err(|e| {
        eprintln!("DEBUG: save_state failed: {}", e);
        "Save Error"
    })?;
    Ok(Value::Null)
}
