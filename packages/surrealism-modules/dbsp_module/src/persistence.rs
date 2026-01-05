use serde_json::Value;
use spooky_stream_processor::engine::store::Store;
use spooky_stream_processor::LazyCircuit;

#[cfg(not(test))]
use surrealism::imports::sql;

#[cfg(test)]
fn sql<I, O>(_input: I) -> Result<O, String> {
    Err("SQL not available in native tests".to_string())
}

pub struct SurrealStore;

impl SurrealStore {
    pub fn new() -> Self {
        Self
    }
}

impl Store for SurrealStore {
    fn get(&self, table: &str, id: &str) -> Option<Value> {
        // Query: SELECT * FROM type::table($table):$id
        // NOTE: id might already be scoped?
        // In Spooky, ids are usually just strings.
        // If id contains ':', it is fully qualified.
        // Let's assume input id is fully qualified OR we try to construct it.
        // But `store.rs` definition: `get(table, id)`.

        let query = format!("SELECT * FROM type::table(\"{}\"):\"{}\"", table, id);
        match sql::<String, Vec<Value>>(query) {
            Ok(mut results) => {
                if let Some(val) = results.pop() {
                    // Check if it is not null/empty
                    if val.is_null() {
                        None
                    } else {
                        Some(val)
                    }
                } else {
                    None
                }
            }
            Err(e) => {
                eprintln!("DEBUG: SurrealStore::get failed: {:?}", e);
                None
            }
        }
    }

    fn get_by_field(&self, table: &str, field: &str, value: &Value) -> Vec<Value> {
        // Query: SELECT * FROM type::table($table) WHERE field = value
        // Need to escape/format value correctly.
        // Best way: Use JSON representation of value.
        let val_str = serde_json::to_string(value).unwrap_or("null".to_string());
        // Simple sanitization or use parameter binding if available (surrealism macro might support it, but direct sql here).
        // For now, simple textual substitution (assuming internal trusted usage).

        let query = format!(
            "SELECT * FROM type::table(\"{}\") WHERE {} = {}",
            table, field, val_str
        );

        match sql::<String, Vec<Value>>(query) {
            Ok(results) => results,
            Err(e) => {
                eprintln!("DEBUG: SurrealStore::get_by_field failed: {:?}", e);
                Vec::new()
            }
        }
    }
}

pub fn load() -> LazyCircuit {
    eprintln!("DEBUG: load_state: Loading from DB...");
    // SELECT content FROM _spooky_module_state WHERE id = 'dbsp'
    match sql::<&str, Vec<Value>>("SELECT content FROM _spooky_module_state:dbsp") {
        Ok(results) => {
            if let Some(first) = results.first() {
                if let Some(content_str) = first.get("content").and_then(|v| v.as_str()) {
                    match serde_json::from_str::<LazyCircuit>(content_str) {
                        Ok(state) => return state,
                        Err(e) => eprintln!("DEBUG: load_state: Deserialization failed: {}", e),
                    }
                }
            }
        }
        Err(e) => eprintln!("DEBUG: load_state: SQL Error: {:?}", e),
    }
    LazyCircuit::new()
}

pub fn save(circuit: &LazyCircuit) {
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

pub fn apply_incantation_update(
    id: &str,
    hash: &str,
    tree: &spooky_stream_processor::engine::view::IdTree,
) {
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

pub fn upsert_incantation(
    id: &str,
    hash: &str,
    tree: &spooky_stream_processor::engine::view::IdTree,
    client_id: &str,
    surrealql: &str,
    params: &Value,
    ttl: &str,
    last_active_at: &str,
) {
    if let Ok(tree_json) = serde_json::to_string(tree) {
        let full_id = if id.starts_with("_spooky_incantation:") {
            id.to_string()
        } else {
            format!("_spooky_incantation:{}", id)
        };

        let escaped_tree = tree_json.replace("\\", "\\\\").replace("'", "\\'");
        let escaped_query = surrealql.replace("\\", "\\\\").replace("'", "\\'");
        let params_json = serde_json::to_string(params).unwrap_or("{}".to_string());
        let escaped_params = params_json.replace("\\", "\\\\").replace("'", "\\'");

        // Ensure values are safe for SQL string injection
        let escaped_client_id = client_id.replace("\\", "\\\\").replace("'", "\\'");
        let escaped_ttl = ttl.replace("\\", "\\\\").replace("'", "\\'");
        let escaped_last_active = last_active_at.replace("\\", "\\\\").replace("'", "\\'");

        let sql_query = format!(
            "{{ LET $ign = UPSERT <record>'{}' SET hash = '{}', tree = {}, clientId = '{}', surrealQL = '{}', params = {}, ttl = <duration>'{}', lastActiveAt = <datetime>'{}'; RETURN []; }}",
            full_id, hash, escaped_tree, escaped_client_id, escaped_query, escaped_params, escaped_ttl, escaped_last_active
        );

        match sql::<String, Vec<Value>>(sql_query) {
            Ok(_) => {} // Success
            Err(e) => eprintln!("DEBUG: upsert_incantation: SQL Error for {}: {:?}", id, e),
        }
    }
}
