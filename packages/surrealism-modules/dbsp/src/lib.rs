use surrealism::surrealism;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;

// Note: This module uses a simulated in-memory implementation of the DBSP logic.
// The `dbsp` crate (v0.160+) relies on server-side async runtimes (Tokio/Actix)
// which are currently incompatible with the WASM target environment required here.
//
// Z-Set Architecture (Incremental Engine):
// - Data is represented as Z-Sets: Collection of (Data, Weight).
// - Weight: i64 (+1 for insertion, -1 for deletion).
// - Processing: Views consume Deltas (changes) and update their internal cache incrementally.

// --- Data Model ---

type Weight = i64;
type RowKey = String; 

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Row {
    data: Value,
}

// A Z-Set is a mapping from Data -> Weight
type ZSet = HashMap<RowKey, Weight>;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[allow(dead_code)]
struct Table {
    name: String,
    // The canonical state of the table is a Z-Set
    zset: ZSet,
}

impl Table {
    fn new(name: String) -> Self {
        Self {
            name,
            zset: HashMap::new(),
        }
    }

    /// Apply a delta to this table's state.
    fn apply_delta(&mut self, delta: &ZSet) {
        for (key, weight) in delta {
            let entry = self.zset.entry(key.clone()).or_insert(0);
            *entry += weight;
            if *entry == 0 {
                self.zset.remove(key);
            }
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Database {
    tables: HashMap<String, Table>,
}

impl Database {
    fn new() -> Self {
        Self {
            tables: HashMap::new(),
        }
    }

    fn ensure_table(&mut self, name: &str) -> &mut Table {
        self.tables
            .entry(name.to_string())
            .or_insert_with(|| Table::new(name.to_string()))
    }
}

// --- ID Tree Implementation ---

#[derive(Serialize, Deserialize, Clone, Debug)]
struct IdTree {
    hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    children: Option<HashMap<String, IdTree>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ids: Option<Vec<String>>,
}

// Helper to compute hash of a list of strings
fn compute_hash(items: &[String]) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    items.join(",").hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

impl IdTree {
    /// Recursively build the Radix Tree from a sorted list of IDs.
    fn build(ids: Vec<String>) -> Self {
        const THRESHOLD: usize = 100; // Max IDs per leaf node

        if ids.len() <= THRESHOLD {
            let hash = compute_hash(&ids);
            return IdTree {
                hash,
                children: None,
                ids: Some(ids),
            };
        }

        // Split by first character (Simple Radix)
        let mut groups: HashMap<String, Vec<String>> = HashMap::new();
        for id in ids {
            // Use first char as key, or empty if empty string
            let prefix = id.chars().next().map(|c| c.to_string()).unwrap_or_else(|| "".to_string());
            groups.entry(prefix).or_default().push(id);
        }

        let mut children = HashMap::new();
        let mut child_hashes = Vec::new();

        for (prefix, group_ids) in groups {
            let child_node = IdTree::build(group_ids);
            child_hashes.push(format!("{}:{}", prefix, child_node.hash));
            children.insert(prefix, child_node);
        }

        // Sort hashes to ensure deterministic parent hash
        child_hashes.sort();
        let hash = compute_hash(&child_hashes);

        IdTree {
            hash,
            children: Some(children),
            ids: None,
        }
    }
}


// --- View / Circuit Model ---

#[derive(Serialize, Deserialize, Clone, Debug)]
struct QueryPlan {
    id: String,
    source_table: String,
    filter_prefix: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct View {
    plan: QueryPlan,
    cache: ZSet, 
    last_hash: String,
}

impl View {
    fn new(plan: QueryPlan) -> Self {
        Self {
            plan,
            cache: HashMap::new(),
            last_hash: String::new(),
        }
    }

    /// Incrementally process a Delta from the circuit.
    fn process(&mut self, changed_table: &str, input_delta: &ZSet) -> Option<MaterializedViewUpdate> {
        if self.plan.source_table != changed_table {
            return None;
        }

        let mut view_delta: ZSet = HashMap::new();
        let mut has_changes = false;

        for (row_key, weight) in input_delta {
            // Check predicate
            let match_filter = match &self.plan.filter_prefix {
                Some(p) => row_key.starts_with(p), 
                None => true,
            };
            
            if match_filter {
                view_delta.insert(row_key.clone(), *weight);
                has_changes = true;
            }
        }

        if !has_changes {
            return None;
        }

        // Apply View Delta
        for (key, weight) in view_delta {
            let entry = self.cache.entry(key.clone()).or_insert(0);
            *entry += weight;
            if *entry == 0 {
                self.cache.remove(&key);
            }
        }

        // Compute Result Set
        let mut result_ids: Vec<String> = self.cache.keys().cloned().collect();
        result_ids.sort();
        
        let hash = compute_hash(&result_ids);

        if hash != self.last_hash {
            self.last_hash = hash.clone();
            
            // Build ID Tree
            let tree = IdTree::build(result_ids.clone());

            return Some(MaterializedViewUpdate {
                query_id: self.plan.id.clone(),
                result_hash: hash,
                result_ids,
                tree,
            });
        }

        None
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Circuit {
    db: Database,
    views: Vec<View>,
}

impl Circuit {
    fn new() -> Self {
        Self {
            db: Database::new(),
            views: Vec::new(),
        }
    }

    fn register_view(&mut self, plan: QueryPlan) {
        // Idempotent registration: if view exists, update/ignore? 
        // For simple mock, let's just replace or ignore if exists.
        if !self.views.iter().any(|v| v.plan.id == plan.id) {
            self.views.push(View::new(plan));
        }
    }

    fn unregister_view(&mut self, id: &str) {
        self.views.retain(|v| v.plan.id != id);
    }

    fn step(&mut self, table: String, delta: ZSet) -> Vec<MaterializedViewUpdate> {
        // 1. Update DB State
        let tb = self.db.ensure_table(&table);
        tb.apply_delta(&delta);

        // 2. Propagate Delta to Views
        let mut updates = Vec::new();
        for view in &mut self.views {
            if let Some(update) = view.process(&table, &delta) {
                updates.push(update);
            }
        }
        updates
    }
}

#[derive(Serialize, Deserialize, Debug)]
struct MaterializedViewUpdate {
    query_id: String,
    result_hash: String,
    result_ids: Vec<String>,
    tree: IdTree,
}

#[derive(Serialize, Deserialize)]
struct IngestResult {
    updates: Vec<MaterializedViewUpdate>,
    new_state: Circuit,
}

// --- Interface ---

fn row_to_key(id: &str, _record: &Value) -> String {
    id.to_string()
}

#[surrealism]
fn ingest(table: String, operation: String, id: String, record: Value, state: Value) -> Result<Value, &'static str> {
    let mut circuit: Circuit = if state.is_null() {
        Circuit::new()
    } else {
        serde_json::from_value(state).unwrap_or_else(|_| Circuit::new())
    };
    
    let key = row_to_key(&id, &record);
    let mut delta: ZSet = HashMap::new();

    match operation.as_str() {
        "CREATE" => { delta.insert(key, 1); },
        "DELETE" => { delta.insert(key, -1); },
        "UPDATE" => { delta.insert(key, 1); }, // Mocking upsert behavior
        _ => {}
    }

    let updates = circuit.step(table, delta);
    
    let result = IngestResult {
        updates,
        new_state: circuit,
    };

    serde_json::to_value(result).map_err(|_| "Failed to serialize result")
}

#[surrealism]
fn register_query(id: String, plan_json: String, state: Value) -> Result<Value, &'static str> {
    let mut circuit: Circuit = if state.is_null() {
        Circuit::new()
    } else {
        serde_json::from_value(state).unwrap_or_else(|_| Circuit::new())
    };
    
    let plan = if let Ok(mut parsed) = serde_json::from_str::<QueryPlan>(&plan_json) {
        parsed.id = id;
        parsed
    } else {
        QueryPlan {
            id,
            source_table: plan_json,
            filter_prefix: None,
        }
    };
    
    circuit.register_view(plan);
    
    let result = json!({
        "msg": format!("Registered view '{}'", circuit.views.last().unwrap().plan.id),
        "new_state": circuit
    });
    
    Ok(result)
}

#[surrealism]
fn unregister_query(id: String, state: Value) -> Result<Value, &'static str> {
    let mut circuit: Circuit = if state.is_null() {
        Circuit::new()
    } else {
        serde_json::from_value(state).unwrap_or_else(|_| Circuit::new())
    };
    
    circuit.unregister_view(&id);
    
    let result = json!({
        "msg": "View unregistered",
        "new_state": circuit
    });

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_id_tree_generation() {
        let mut circuit = Circuit::new();
        let plan = QueryPlan {
            id: "q_all".to_string(),
            source_table: "users".to_string(),
            filter_prefix: None,
        };
        circuit.register_view(plan);

        // 1. Ingest IDs
        let mut delta = HashMap::new();
        delta.insert("users:1".to_string(), 1);
        delta.insert("users:2".to_string(), 1);
        delta.insert("users:3".to_string(), 1);

        let updates = circuit.step("users".to_string(), delta);
        
        assert_eq!(updates.len(), 1);
        let update = &updates[0];
        assert_eq!(update.query_id, "q_all");
        assert_eq!(update.result_ids, vec!["users:1", "users:2", "users:3"]);
        
        // 2. Verify Tree
        // With 3 items, threshold allows leaf node.
        assert!(update.tree.ids.is_some());
        assert_eq!(update.tree.ids.as_ref().unwrap().len(), 3);
    }

    #[test]
    fn test_filter_and_delete() {
        let mut circuit = Circuit::new();
        let plan = QueryPlan {
            id: "q_admin".to_string(),
            source_table: "users".to_string(),
            filter_prefix: Some("users:admin".to_string()),
        };
        circuit.register_view(plan);

        // 1. Ingest Mixed IDs
        let mut delta = HashMap::new();
        delta.insert("users:admin:1".to_string(), 1);
        delta.insert("users:guest:1".to_string(), 1);

        let updates = circuit.step("users".to_string(), delta);
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].result_ids, vec!["users:admin:1"]); // Guest should be filtered out

        // 2. Delete Admin
        let mut delta_delete = HashMap::new();
        delta_delete.insert("users:admin:1".to_string(), -1);
        
        let updates_delete = circuit.step("users".to_string(), delta_delete);
        assert_eq!(updates_delete.len(), 1);
        assert_eq!(updates_delete[0].result_ids.len(), 0); // Should be empty
    }
}
