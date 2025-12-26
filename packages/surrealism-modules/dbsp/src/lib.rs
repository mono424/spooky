use surrealism::surrealism;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

pub mod converter; // Import converter module

// Note: This module uses a simulated in-memory implementation of the DBSP logic.
// The `dbsp` crate (v0.160+) relies on server-side async runtimes (Tokio/Actix)
// which are currently incompatible with the WASM target environment required here.
//
// Z-Set Architecture (Incremental Engine):
// - Data is represented as Z-Sets: Collection of (Data, Weight).
// - Weight: i64 (+1 for insertion, -1 for deletion).
// - Processing: Views consume Deltas (changes) and update their internal cache incrementally.

// --- Data Model ---

pub type Weight = i64;
pub type RowKey = String; 

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Row {
    pub data: Value,
}

// A Z-Set is a mapping from Data -> Weight
pub type ZSet = HashMap<RowKey, Weight>;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct Table {
    pub name: String,
    // The canonical state of the table is a Z-Set
    pub zset: ZSet,
    // Actual data storage (needed for Joins/Filters)
    pub rows: HashMap<RowKey, Value>,
}

impl Table {
    pub fn new(name: String) -> Self {
        Self {
            name,
            zset: HashMap::new(),
            rows: HashMap::new(),
        }
    }

    pub fn update_row(&mut self, key: String, data: Value) {
        self.rows.insert(key, data);
    }
    
    pub fn delete_row(&mut self, key: &str) {
        self.rows.remove(key);
    }

    /// Apply a delta to this table's state.
    pub fn apply_delta(&mut self, delta: &ZSet) {
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
pub struct Database {
    pub tables: HashMap<String, Table>,
}

impl Database {
    pub fn new() -> Self {
        Self {
            tables: HashMap::new(),
        }
    }

    pub fn ensure_table(&mut self, name: &str) -> &mut Table {
        self.tables
            .entry(name.to_string())
            .or_insert_with(|| Table::new(name.to_string()))
    }
}

// --- ID Tree Implementation ---

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct IdTree {
    pub hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<HashMap<String, IdTree>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ids: Option<Vec<String>>,
}

// Helper to compute hash of a list of strings
pub fn compute_hash(items: &[String]) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    items.join(",").hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

impl IdTree {
    /// Recursively build the Radix Tree from a sorted list of IDs.
    pub fn build(ids: Vec<String>) -> Self {
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
#[serde(tag = "op", rename_all = "lowercase")]
pub enum Operator {
    Scan { table: String },
    Filter { 
        input: Box<Operator>,
        predicate: Predicate 
    },
    Join {
        left: Box<Operator>,
        right: Box<Operator>,
        on: JoinCondition, 
    },
    Project {
        input: Box<Operator>,
        projections: Vec<Projection>,
    },
    Limit {
        input: Box<Operator>,
        limit: usize,
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Projection {
    All,
    Field { name: String },
    Subquery { alias: String, plan: Box<Operator> }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct JoinCondition {
    pub left_field: String,
    pub right_field: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Predicate {
    Prefix { prefix: String },
    Eq { field: String, value: Value },
    // Future support for: GT, LT, etc.
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct QueryPlan {
    pub id: String,
    pub root: Operator,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct View {
    pub plan: QueryPlan,
    pub cache: ZSet, 
    pub last_hash: String,
}

impl View {
    pub fn new(plan: QueryPlan) -> Self {
        Self {
            plan,
            cache: HashMap::new(),
            last_hash: String::new(),
        }
    }

    /// Incrementally process a Delta from the circuit.
    pub fn process(&mut self, _changed_table: &str, _input_delta: &ZSet, db: &Database) -> Option<MaterializedViewUpdate> {
        // Strategy: Re-evaluation (Snapshot Diff)
        // 1. Compute the Target Set based on current DB state.
        let target_set = self.eval_snapshot(&self.plan.root, db, None);
        
        // 2. Compute Delta = Target - Cache
        let mut view_delta: ZSet = HashMap::new();
        
        // Add new/updated items
        for (key, &new_weight) in &target_set {
            let old_weight = self.cache.get(key).cloned().unwrap_or(0);
            if new_weight != old_weight {
                view_delta.insert(key.clone(), new_weight - old_weight);
            }
        }
        
        // Remove deleted items (present in cache but not in target)
        for (key, &old_weight) in &self.cache {
            if !target_set.contains_key(key) {
                view_delta.insert(key.clone(), 0 - old_weight);
            }
        }
        
        if view_delta.is_empty() {
             return None;
        }

        // 3. Update Cache & Emit
        for (key, weight) in &view_delta {
            let entry = self.cache.entry(key.clone()).or_insert(0);
            *entry += weight;
            if *entry == 0 {
                self.cache.remove(key);
            }
        }

        // Compute Result Set
        let mut result_ids: Vec<String> = self.cache.keys().cloned().collect();
        result_ids.sort();
        
        let hash = compute_hash(&result_ids);

        if hash != self.last_hash {
            self.last_hash = hash.clone();
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

    fn eval_snapshot(&self, op: &Operator, db: &Database, context: Option<&Value>) -> ZSet {
        match op {
            Operator::Scan { table } => {
                if let Some(tb) = db.tables.get(table) {
                    tb.zset.clone()
                } else {
                    HashMap::new()
                }
            },
            Operator::Filter { input, predicate } => {
                let upstream = self.eval_snapshot(input, db, context);
                let mut out = HashMap::new();
                for (key, weight) in upstream {
                    if self.check_predicate(predicate, &key, db, context) {
                        out.insert(key, weight);
                    }
                }
                out
            },
            Operator::Project { input, projections: _ } => {
                // Identity for ZSet (ID set) unless we implement Map
                self.eval_snapshot(input, db, context)
            },
            Operator::Limit { input, limit } => {
                let upstream = self.eval_snapshot(input, db, context);
                let mut items: Vec<_> = upstream.into_iter().collect();
                // implicit sort by key (ID)
                items.sort_by(|a, b| a.0.cmp(&b.0)); 
                
                let mut out = HashMap::new();
                for (i, (key, weight)) in items.into_iter().enumerate() {
                    if i < *limit {
                        out.insert(key, weight);
                    } else {
                        break;
                    }
                }
                out
            },
            Operator::Join { left, right, on } => {
                // Nested Loop Join on Full Snapshots
                let s_left = self.eval_snapshot(left, db, context);
                let s_right = self.eval_snapshot(right, db, context);
                let mut out = HashMap::new();

                for (l_key, l_weight) in &s_left {
                     let l_val_opt = self.get_row_value(l_key, db);
                     if l_val_opt.is_none() { continue; }
                     let l_val = l_val_opt.unwrap();
                     
                     // Get Join Field Value
                     let l_field_val = l_val.as_object().and_then(|o| o.get(&on.left_field));
                     if l_field_val.is_none() { continue; }

                     for (r_key, r_weight) in &s_right {
                         let r_val_opt = self.get_row_value(r_key, db);
                         if let Some(r_val) = r_val_opt {
                             let r_field_val = r_val.as_object().and_then(|o| o.get(&on.right_field));
                             if l_field_val == r_field_val {
                                 // Match!
                                 let w = l_weight * r_weight;
                                 *out.entry(l_key.clone()).or_insert(0) += w;
                             }
                        }
                     }
                }
                out
            }
        }
    }

    fn get_row_value<'a>(&self, key: &str, db: &'a Database) -> Option<&'a Value> {
         let parts: Vec<&str> = key.splitn(2, ':').collect();
         if parts.len() < 2 { return None; }
         db.tables.get(parts[0])?.rows.get(key)
    }

    fn check_predicate(&self, pred: &Predicate, key: &str, db: &Database, context: Option<&Value>) -> bool {
        match pred {
            Predicate::Prefix { prefix } => key.starts_with(prefix),
            Predicate::Eq { field, value } => {
                // Check if value is a param
                let target_val = if let Some(obj) = value.as_object() {
                    if let Some(param_path) = obj.get("$param") {
                         // Resolve param from context
                         if let Some(ctx) = context {
                             if let Some(ctx_val) = ctx.get(param_path.as_str().unwrap_or("")) {
                                 ctx_val
                             } else {
                                 return false; // Param not found in context
                             }
                         } else {
                             return false; // No context
                         }
                    } else {
                        value
                    }
                } else {
                    value
                };

                // Find table from key
                let parts: Vec<&str> = key.splitn(2, ':').collect();
                if parts.len() < 2 { return false; }
                let table_name = parts[0];
                
                if let Some(table) = db.tables.get(table_name) {
                    if let Some(row_val) = table.rows.get(key) {
                        if let Some(obj) = row_val.as_object() {
                            if let Some(f_val) = obj.get(field) {
                                return f_val == target_val;
                            }
                        }
                    }
                }
                false
            }
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Circuit {
    pub db: Database,
    pub views: Vec<View>,
}

impl Circuit {
    pub fn new() -> Self {
        Self {
            db: Database::new(),
            views: Vec::new(),
        }
    }

    pub fn register_view(&mut self, plan: QueryPlan) {
        if !self.views.iter().any(|v| v.plan.id == plan.id) {
            self.views.push(View::new(plan));
        }
    }

    pub fn unregister_view(&mut self, id: &str) {
        self.views.retain(|v| v.plan.id != id);
    }

    pub fn step(&mut self, table: String, delta: ZSet) -> Vec<MaterializedViewUpdate> {
        // 1. Update DB State
        let tb = self.db.ensure_table(&table);
        tb.apply_delta(&delta);

        // 2. Propagate Delta to Views
        let mut updates = Vec::new();
        for i in 0..self.views.len() {
             if let Some(update) = self.views[i].process(&table, &delta, &self.db) {
                 updates.push(update);
             }
        }
        updates
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct MaterializedViewUpdate {
    pub query_id: String,
    pub result_hash: String,
    pub result_ids: Vec<String>,
    pub tree: IdTree,
}

#[derive(Serialize, Deserialize)]
pub struct IngestResult {
    pub updates: Vec<MaterializedViewUpdate>,
    pub new_state: Circuit,
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
        "CREATE" => { 
            delta.insert(key.clone(), 1); 
            // Update storage
            let tb = circuit.db.ensure_table(&table);
            tb.update_row(key, record);
        },
        "DELETE" => { 
            delta.insert(key.clone(), -1); 
            // Update storage
            let tb = circuit.db.ensure_table(&table);
            tb.delete_row(&key);
        },
        "UPDATE" => { 
            delta.insert(key.clone(), 1); 
             // Update storage
            let tb = circuit.db.ensure_table(&table);
            tb.update_row(key, record);
        }, 
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
    
    let root_op = if let Ok(parsed_plan) = serde_json::from_str::<QueryPlan>(&plan_json) {
        parsed_plan.root
    } else if let Ok(op) = serde_json::from_str::<Operator>(&plan_json) {
         op
    } else {
        // Try parsing as SQL
        match converter::convert_surql_to_dbsp(&plan_json) {
            Ok(json_val) => {
                match serde_json::from_value::<Operator>(json_val) {
                    Ok(op) => op,
                    Err(_) => Operator::Scan { table: plan_json } // Fallback
                }
            },
            Err(_) => {
                // Fallback for legacy simple format (just table string)
                Operator::Scan { table: plan_json }
            }
        }
    };

    let plan = QueryPlan {
        id: id.clone(),
        root: root_op,
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
