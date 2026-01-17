use serde_json::Value;
use spooky_stream_processor::engine::Circuit;
use surrealism::imports::sql;

pub fn load() -> Circuit {
    eprintln!("DEBUG: load_state: Mock loading (Bypassing SQL)...");
    // match sql::<&str, Vec<Value>>("SELECT content FROM _spooky_module_state:dbsp") {
    //     Ok(results) => {
    //         if let Some(first) = results.first() {
    //             if let Some(content_str) = first.get("content").and_then(|v| v.as_str()) {
    //                 match serde_json::from_str::<Circuit>(content_str) {
    //                     Ok(state) => return state,
    //                     Err(e) => eprintln!("DEBUG: load_state: Deserialization failed: {}", e),
    //                 }
    //             }
    //         }
    //     }
    //     Err(e) => eprintln!("DEBUG: load_state: SQL Error: {:?}", e),
    // }
    Circuit::new()
}

pub fn save(_circuit: &Circuit) {
    eprintln!("DEBUG: save_state: Mock saving (Bypassing SQL)...");
}

pub fn clear() {
    let _ = sql::<&str, Vec<Value>>("DELETE _spooky_module_state:dbsp");
}

pub fn apply_incantation_update(id: &str, hash: &str, _data: &[(String, u64)]) {
    eprintln!(
        "DEBUG: apply_incantation_update: Mock update for {} hash {} (Bypassing SQL)...",
        id, hash
    );
}

pub fn upsert_incantation(
    id: &str,
    hash: &str,
    _data: &[(String, u64)],
    _client_id: &str,
    _surrealql: &str,
    _params: &Value,
    _ttl: &str,
    _last_active_at: &str,
) {
    eprintln!(
        "DEBUG: upsert_incantation: Mock upsert for {} hash {} (Bypassing SQL)...",
        id, hash
    );
}
