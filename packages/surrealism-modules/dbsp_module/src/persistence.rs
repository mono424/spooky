use crate::engine::Circuit;
use surrealism::imports::sql;
use serde_json::Value;

pub fn load() -> Circuit {
    eprintln!("DEBUG: load_state: Loading from DB...");
    // SELECT content FROM _spooky_module_state WHERE id = 'dbsp'
    match sql::<&str, Vec<Value>>("SELECT content FROM _spooky_module_state:dbsp") {
        Ok(results) => {
            if let Some(first) = results.first() {
                if let Some(content_str) = first.get("content").and_then(|v| v.as_str()) {
                    match serde_json::from_str::<Circuit>(content_str) {
                        Ok(state) => return state,
                        Err(e) => eprintln!("DEBUG: load_state: Deserialization failed: {}", e),
                    }
                }
            }
        },
        Err(e) => eprintln!("DEBUG: load_state: SQL Error: {:?}", e),
    }
    Circuit::new()
}

pub fn save(circuit: &Circuit) {
    if let Ok(content) = serde_json::to_string(circuit) {
        // Escape backslashes first, then single quotes
        let escaped_content = content.replace("\\", "\\\\").replace("'", "\\'"); 
        let sql_query = format!("{{ LET $ign = UPSERT _spooky_module_state:dbsp SET content = '{}'; RETURN []; }}", escaped_content);
        
        // Use Vec<Value> and return an empty array to match standard SQL binding expectations
        match sql::<String, Vec<Value>>(sql_query) {
             Ok(_) => {}, // Success
             Err(e) => eprintln!("DEBUG: save_state: SQL Error: {:?}", e),
        }
    }
}

pub fn clear() {
    let _ = sql::<&str, Vec<Value>>("DELETE _spooky_module_state:dbsp");
}
