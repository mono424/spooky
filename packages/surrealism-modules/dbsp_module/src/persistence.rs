use crate::engine::Circuit;
use serde_json::Value;
use surrealism::imports::sql;

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
        }
        Err(e) => eprintln!("DEBUG: load_state: SQL Error: {:?}", e),
    }
    Circuit::new()
}

pub fn save(circuit: &Circuit) {
    if let Ok(content) = serde_json::to_string(circuit) {
        // Escape backslashes first, then single quotes
        let escaped_content = content.replace("\\", "\\\\").replace("'", "\\'");
        let sql_query = format!(
            "{{ LET $ign = UPSERT _spooky_module_state:dbsp SET content = '{}'; RETURN []; }}",
            escaped_content
        );

        // Use Vec<Value> and return an empty array to match standard SQL binding expectations
        match sql::<String, Vec<Value>>(sql_query) {
            Ok(_) => {} // Success
            Err(e) => eprintln!("DEBUG: save_state: SQL Error: {:?}", e),
        }
    }
}

pub fn clear() {
    let _ = sql::<&str, Vec<Value>>("DELETE _spooky_module_state:dbsp");
}

pub fn apply_incantation_update(id: &str, hash: &str, tree: &crate::engine::view::IdTree) {
    if let Ok(tree_json) = serde_json::to_string(tree) {
        // Handle ID: If it already starts with table prefix, use it as is (but ensured string).
        // Otherwise preped.
        let full_id = if id.starts_with("_spooky_incantation:") {
            id.to_string()
        } else {
            format!("_spooky_incantation:{}", id)
        };

        // Escape content safely
        let escaped_tree = tree_json.replace("\\", "\\\\").replace("'", "\\'");

        // Use <record> cast with single quotes to safely handle any characters in the ID
        let sql_query = format!(
            "{{ LET $ign = UPDATE <record>'{}' SET hash = '{}', tree = {}; RETURN []; }}",
            full_id, hash, escaped_tree
        );

        match sql::<String, Vec<Value>>(sql_query) {
            Ok(_) => {} // Success
            Err(e) => eprintln!(
                "DEBUG: apply_incantation_update: SQL Error for {}: {:?}",
                id, e
            ),
        }
    }
}
