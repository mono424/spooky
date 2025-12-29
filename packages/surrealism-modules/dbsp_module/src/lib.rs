use surrealism::surrealism;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;

lazy_static::lazy_static! {
    static ref CIRCUIT: Mutex<Option<Circuit>> = Mutex::new(None);
}


use surrealism::imports::sql;
use regex::{Regex, Captures};

// --- Helper: Fix SurrealQL object string to valid JSON ---
fn fix_surql_json(s: &str) -> String {
    println!("DEBUG: fix_surql_json input: {}", s);
    
    // Regex to match:
    // 1. Single quoted strings: '...' (Group 1)
    // 2. Double quoted strings: "..." (Group 2)
    // 3. Keys: identifier followed by colon AND space (e.g. "id: "). (Group 3)
    // 4. Record IDs (Simple): ident:ident (e.g. user:123). (Group 4)
    // 5. Record IDs (Backticked): ident:`...` (e.g. thread:`thread:123`). (Group 5)
    let re = Regex::new(r#"('[^']*')|("[^"]*")|(\w+)\s*:\s|(\w+:\w+)|(\w+:`[^`]+`)"#).unwrap();
    
    let result = re.replace_all(s, |caps: &Captures| {
        if let Some(key) = caps.get(3) {
             // Key found. Quote it.
            format!("\"{}\": ", key.as_str())
        } else if let Some(rec_id) = caps.get(4) {
            // Unquoted Record ID (simple). Quote it.
             format!("\"{}\"", rec_id.as_str())
        } else if let Some(rec_id_complex) = caps.get(5) {
             // Complex Record ID with backticks.
             // e.g. thread:`thread:123`
             // transform to "thread:thread:123" OR just strip backticks?
             // Best to just quote the whole raw string to match exact record ID string representation?
             // But JSON parser expects "table:id".
             // If Surreal output is `table:`id``, does `id` contain the table prefix too?
             // Log: id: thread:`thread:b6a...`
             // If we quote it: "thread:`thread:b6a...`" - this is a valid string.
             // But does it match the ROW key?
             // Row Key is String.
             // If the row key in DB is `thread:b6a...`.
             // Does `thread:` prefix equal the table name?
             // `thread:`thread:b6a...`` implies table `thread`, id `thread:b6a...`.
             // This is redundant/weird SurrealQL output.
             //
             // Strategy: Quote the entire matched string, but escape/strip backticks?
             // If I quote it as is: `"thread:`thread:123`"`.
             // `compare_json` will compare this string with stored ID.
             // stored ID is usually `table:id`.
             //
             // Let's try to CLEAN it.
             // Remove backticks?
             let raw = rec_id_complex.as_str();
             // raw: thread:`thread:123`
             let clean = raw.replace("`", ""); 
             // clean: thread:thread:123
             // This looks like double prefix?
             // Maybe just quote it as is essentially stringifying the ID.
             // But backticks in JSON string?
             // "thread:`thread:123`".
             //
             // Wait. If I just quote it, serde_json parses it as a String.
             // Then equality check compares `String("thread:`thread:123`")` vs `String("thread:b6a...")`.
             // They WON'T match.
             //
             // I need to extract the INNER content of the backticks if it repeats?
             // Or maybe just `table:id` format.
             //
             // Log: `thread:b6a...` is the row key.
             // Input param: `thread:`thread:b6a...``.
             //
             // If I extract the part inside backticks: `thread:b6a...`.
             // That matches!
             //
             // So I should extract the content inside backticks.
             // Regex: `\w+:(`[^`]+`)` -> Capture Group 6 inside 5?
             // Let's rely on string manipulation.
             
             let raw = rec_id_complex.as_str();
             if let Some(start) = raw.find('`') {
                 if let Some(end) = raw.rfind('`') {
                     let inner = &raw[start+1..end];
                     return format!("\"{}\"", inner);
                 }
             }
             format!("\"{}\"", raw)
        } else if let Some(sq) = caps.get(1) {
            let content = &sq.as_str()[1..sq.as_str().len()-1];
            let escaped = content.replace("\"", "\\\"");
            format!("\"{}\"", escaped)
        } else {
             caps.get(0).unwrap().as_str().to_string()
        }
    });

    println!("DEBUG: fix_surql_json output: {}", result);
    result.to_string()
}


fn load_state() -> Circuit {
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

fn persist_circuit(circuit: &Circuit) {
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
        // println!("DEBUG: Table {} update_row key={}", self.name, key);
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
pub struct LeafItem {
    pub id: String,
    pub hash: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct IdTree {
    pub hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<HashMap<String, IdTree>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub leaves: Option<Vec<LeafItem>>,
}

// Helper to compute hash of a list of strings
pub fn compute_hash(items: &[String]) -> String {
    let mut hasher = blake3::Hasher::new();
    for item in items {
        hasher.update(item.as_bytes());
        hasher.update(&[0]); // Delimiter
    }
    hasher.finalize().to_hex().to_string()
}

impl IdTree {
    /// Recursively build the Radix Tree from a sorted list of IDs.
    pub fn build(items: Vec<LeafItem>) -> Self {
        const THRESHOLD: usize = 100; // Max items per leaf node

        if items.len() <= THRESHOLD {
            // Hash the leaf items (id + hash)
            let mut hasher = blake3::Hasher::new();
            for item in &items {
                hasher.update(item.id.as_bytes());
                hasher.update(item.hash.as_bytes());
                hasher.update(&[0]);
            }
            let hash = hasher.finalize().to_hex().to_string();

            return IdTree {
                hash,
                children: None,
                leaves: Some(items),
            };
        }

        // Split by first character of ID (Simple Radix)
        let mut groups: HashMap<String, Vec<LeafItem>> = HashMap::new();
        for item in items {
            // Use first char as key
            let prefix = item.id.chars().next().map(|c| c.to_string()).unwrap_or_else(|| "".to_string());
            groups.entry(prefix).or_default().push(item);
        }

        let mut children = HashMap::new();
        let mut child_hashes = Vec::new();

        for (prefix, group_items) in groups {
            let child_node = IdTree::build(group_items);
            child_hashes.push(format!("{}:{}", prefix, child_node.hash));
            children.insert(prefix, child_node);
        }

        // Sort hashes to ensure deterministic parent hash
        child_hashes.sort();
        let hash = compute_hash(&child_hashes);

        IdTree {
            hash,
            children: Some(children),
            leaves: None,
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
        #[serde(default)]
        order_by: Option<Vec<OrderSpec>>,
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct OrderSpec {
    pub field: String,
    pub direction: String, // "ASC" | "DESC"
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
    And { predicates: Vec<Predicate> },
    Or { predicates: Vec<Predicate> },
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
    // Store parameters for this view's query
    #[serde(default)]
    pub params: Option<Value>,
}

impl View {
    pub fn new(plan: QueryPlan, params: Option<Value>) -> Self {
        Self {
            plan,
            cache: HashMap::new(),
            last_hash: String::new(),
            params,
        }
    }

    /// Incrementally process a Delta from the circuit.
    pub fn process(&mut self, _changed_table: &str, _input_delta: &ZSet, db: &Database) -> Option<MaterializedViewUpdate> {
        // Strategy: Re-evaluation (Snapshot Diff)
        // 1. Compute the Target Set based on current DB state.
        // Use self.params as context for predicate evaluation
        let target_set = self.eval_snapshot(&self.plan.root, db, self.params.as_ref());
        
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
        // TODO: Apply top-level ORDER BY if we add it to QueryPlan later.
        // For now, default ID sort is stable.
        result_ids.sort();
        
        // Build Leaf Items
        let items: Vec<LeafItem> = result_ids.iter().map(|id| {
            // Lookup row to get content
            let val = self.get_row_value(id, db);
            let hash = if let Some(v) = val {
                // Try to get precomputed hash from record
                if let Some(h) = v.get("IntrinsicHash").or_else(|| v.get("hash")).or_else(|| v.get("_hash")) {
                    h.as_str().unwrap_or("0000").to_string()
                } else {
                    // Precomputed hash is mandatory
                    // panic!("Missing IntrinsicHash/hash/_hash on record {}", id);
                     "MISSING_HASH".to_string()
                }
            } else {
                 "0000".to_string() // Should not happen for valid views
            };
            LeafItem {
                id: id.clone(),
                hash,
            }
        }).collect();

        // Compute root hash from items
        // Note: IdTree::build computes its own hash, but we check change first.
        // We can optimize by not building tree if hash is same... 
        // But IdTree::build logic starts with full list.
        // Let's build tree first? Or compute simple hash list same as IdTree?
        // IdTree::build is efficient enough.
        
        let tree = IdTree::build(items);
        let hash = tree.hash.clone();
        
        eprintln!("DEBUG: process: view={}, delta_len={}, hash={}, last_hash={}", self.plan.id, view_delta.len(), hash, self.last_hash);

        if hash != self.last_hash {
            self.last_hash = hash.clone();
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
                    println!("DEBUG: eval_snapshot SCAN table {} found items: {}", table, tb.zset.len());
                    tb.zset.clone()
                } else {
                    println!("DEBUG: eval_snapshot SCAN table {} NOT FOUND", table);
                    HashMap::new()
                }
            },
            Operator::Filter { input, predicate } => {
                let upstream = self.eval_snapshot(input, db, context);
                let mut out = HashMap::new();
                for (key, weight) in upstream {
                    if self.check_predicate(predicate, &key, db, context) {
                        out.insert(key, weight);
                    } else {
                         println!("DEBUG: Filter REJECTED key {} with pred {:?}", key, predicate);
                    }
                }
                out
            },
            Operator::Project { input, projections: _ } => {
                // Identity for ZSet (ID set) unless we implement Map
                self.eval_snapshot(input, db, context)
            },
            Operator::Limit { input, limit, order_by } => {
                let upstream = self.eval_snapshot(input, db, context);
                let mut items: Vec<_> = upstream.into_iter().collect();
                
                if let Some(orders) = order_by {
                     items.sort_by(|a, b| {
                         let row_a = self.get_row_value(&a.0, db);
                         let row_b = self.get_row_value(&b.0, db);
                         
                         for ord in orders {
                             let val_a = row_a.and_then(|r| r.as_object()).and_then(|o| o.get(&ord.field));
                             let val_b = row_b.and_then(|r| r.as_object()).and_then(|o| o.get(&ord.field));
                             
                             let cmp = compare_json_values(val_a, val_b);
                             if cmp != std::cmp::Ordering::Equal {
                                 return if ord.direction.eq_ignore_ascii_case("DESC") { cmp.reverse() } else { cmp };
                             }
                         }
                         // Fallback to ID
                         a.0.cmp(&b.0)
                     });
                } else {
                    // implicit sort by key (ID)
                    items.sort_by(|a, b| a.0.cmp(&b.0)); 
                }
                
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
            Predicate::And { predicates } => {
                for p in predicates {
                    if !self.check_predicate(p, key, db, context) {
                        return false;
                    }
                }
                true
            },
            Predicate::Or { predicates } => {
                for p in predicates {
                    if self.check_predicate(p, key, db, context) {
                        return true;
                    }
                }
                false
            },
            Predicate::Prefix { prefix } => key.starts_with(prefix),
            Predicate::Eq { field, value } => {
                // Check if value is a param
                let target_val = if let Some(obj) = value.as_object() {
                    if let Some(param_path) = obj.get("$param") {
                         // Resolve param from context
                         if let Some(ctx) = context {
                             if let Some(ctx_val) = ctx.get(param_path.as_str().unwrap_or("")) {
                                 println!("DEBUG: Resolved param {}: {:?}", param_path, ctx_val);
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
                
                if field == "id" {
                    let key_val = json!(key);
                    return compare_json_values(Some(&key_val), Some(target_val)) == std::cmp::Ordering::Equal;
                }

                if let Some(table) = db.tables.get(table_name) {
                    if let Some(row_val) = table.rows.get(key) {
                        if let Some(obj) = row_val.as_object() {
                             // Handle nested fields? e.g. "author.name"
                             // Simple field access for now
                            if let Some(f_val) = obj.get(field) {
                                return compare_json_values(Some(f_val), Some(target_val)) == std::cmp::Ordering::Equal;
                            }
                        }
                    }
                }
                false
            }
        }
    }
}

// Helper for comparing JSON values (Partial implementation)
fn compare_json_values(a: Option<&Value>, b: Option<&Value>) -> std::cmp::Ordering {
    match (a, b) {
        (None, None) => std::cmp::Ordering::Equal,
        (None, Some(_)) => std::cmp::Ordering::Less,
        (Some(_), None) => std::cmp::Ordering::Greater,
        (Some(xa), Some(xb)) => {
            // Try numeric comparison first
            if let (Some(na), Some(nb)) = (xa.as_f64(), xb.as_f64()) {
                // Use epsilon for float equality? Or simple partial cmp
                 na.partial_cmp(&nb).unwrap_or(std::cmp::Ordering::Equal)
            } else if let (Some(sa), Some(sb)) = (xa.as_str(), xb.as_str()) {
                sa.cmp(sb)
            } else if let (Some(ba), Some(bb)) = (xa.as_bool(), xb.as_bool()) {
                ba.cmp(&bb)
            } else {
                // Fallback: compare string representation
                xa.to_string().cmp(&xb.to_string())
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

    pub fn register_view(&mut self, plan: QueryPlan, params: Option<Value>) {
        // If view exists, remove it first (to support updates/param changes)
        if let Some(pos) = self.views.iter().position(|v| v.plan.id == plan.id) {
            self.views.remove(pos);
        }
        self.views.push(View::new(plan, params));
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
    // Removed new_state
}

// --- Interface ---

fn row_to_key(id: &str, _record: &Value) -> String {
    id.to_string()
}

#[surrealism]
fn ingest(table: String, operation: String, id: String, record: Value) -> Result<Value, &'static str> {
    let mut circuit_guard = CIRCUIT.lock().map_err(|_| "Failed to lock circuit")?;
    
    if circuit_guard.is_none() {
        *circuit_guard = Some(load_state());
    }
    let circuit = circuit_guard.as_mut().unwrap();
    
    // Handle stringified record to avoid FFI type issues with RecordId/Datetime
    let record_obj = match record {
        Value::String(s) => {
            let parsed = serde_json::from_str::<Value>(&s).unwrap_or(Value::Null);
            println!("DEBUG: ingest parsed string record: {:?}", parsed);
            parsed
        },
        _ => {
            println!("DEBUG: ingest received direct record: {:?}", record);
            record
        }
    };
    
    let key = row_to_key(&id, &record_obj);
    let mut delta: ZSet = HashMap::new();

    match operation.as_str() {
        "CREATE" => { 
            delta.insert(key.clone(), 1); 
            // Update storage
            let tb = circuit.db.ensure_table(&table);
            tb.update_row(key, record_obj);
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
            tb.update_row(key, record_obj);
        }, 
        _ => {}
    }

    eprintln!("DEBUG: ingest: table={}, views={}, db_tables={}", table, circuit.views.len(), circuit.db.tables.len());
    if let Some(tb) = circuit.db.tables.get(&table) {
        eprintln!("DEBUG: ingest: table {} size={}", table, tb.zset.len()); // ZSet size tracks logical size
    }

    let updates = circuit.step(table, delta);
    eprintln!("DEBUG: ingest: generated {} updates", updates.len());

    persist_circuit(&circuit);
    
    let result = IngestResult {
        updates,
    };

    serde_json::to_value(result).map_err(|_| "Failed to serialize result")
}

#[surrealism]
fn register_view(id: String, plan_val: Value, params: Value) -> Result<Value, &'static str> {
    println!("DEBUG: register_view called with id: {}, params: {:?}", id, params);
    let mut circuit_guard = CIRCUIT.lock().map_err(|_| "Failed to lock circuit")?;

    if circuit_guard.is_none() {
        *circuit_guard = Some(load_state());
    }
    let circuit = circuit_guard.as_mut().unwrap();

    let plan_json = match plan_val {
        Value::String(s) => s,
        _ => plan_val.to_string()
    };
    
    let root_op = if let Ok(parsed_plan) = serde_json::from_str::<QueryPlan>(&plan_json) {
        parsed_plan.root
    } else if let Ok(op) = serde_json::from_str::<Operator>(&plan_json) {
         op
    } else {
        // Try parsing as SQL
        match converter::convert_surql_to_dbsp(&plan_json) {
            Ok(json_val) => {
                // println!("DBSP DEBUG: Converted SQL: {}", json_val);
                match serde_json::from_value::<Operator>(json_val) {
                    Ok(op) => op,
                    Err(e) => {
                        println!("DBSP DEBUG: JSON Deserialization Error: {}", e);
                        Operator::Scan { table: plan_json } // Fallback
                    }
                }
            },
            Err(e) => {
                println!("DBSP DEBUG: SQL Parse Error: {}", e);
                // Fallback for legacy simple format (just table string)
                Operator::Scan { table: plan_json }
            }
        }
    };

    let plan = QueryPlan {
        id: id.clone(),
        root: root_op,
    };
    
    // Parse params if passed as string
    // Fix SurrealQL object string to JSON (quote keys)
    let params_str = match params {
        Value::String(s) => fix_surql_json(&s),
        _ => params.to_string()
    };

    let params_parsed = match serde_json::from_str::<Value>(&params_str) {
        Ok(v) => {
             println!("DEBUG: register_view parsed params success: {:?}", v);
             v
        },
        Err(e) => {
             println!("DEBUG: register_view parsed params error: {} input: {}", e, params_str);
             Value::Null
        }
    };

    // Pass params (convert Value to Option<Value> - usually it's an Object)
    let params_opt = if params_parsed.is_null() { None } else { Some(params_parsed) };
    circuit.register_view(plan, params_opt);
    
    // Trigger initial hydration to compute hash
    let mut update = None;
    if let Some(view) = circuit.views.last_mut() {
        let empty_delta = HashMap::new();
        update = view.process("", &empty_delta, &circuit.db);
    }

    persist_circuit(&circuit);
    
    if let Some(u) = update {
        Ok(serde_json::to_value(u).unwrap_or(json!({"status": "ERR", "msg": "Serialization failed"})))
    } else {
        // Should not really happen if empty hash is distinct from "" string
         Ok(json!({
            "msg": format!("Registered view '{}' (No initial update)", id),
            "result_hash": "", // Should probably be empty tree hash
            "tree": Value::Null
        }))
    }
}

#[surrealism]
fn reset(_val: Value) -> Result<Value, &'static str> {
    let mut circuit_guard = CIRCUIT.lock().map_err(|_| "Failed to lock circuit")?;
    *circuit_guard = Some(Circuit::new());
    
    // Also clear the persistent state in DB
    // UPDATE _spooky_module_state:dbsp SET content = ""
    let _ = sql::<&str, Vec<Value>>("DELETE _spooky_module_state:dbsp");

    Ok(Value::Null)
}

#[surrealism]
fn unregister_view(id: String) -> Result<Value, &'static str> {
    let mut circuit_guard = CIRCUIT.lock().map_err(|_| "Failed to lock circuit")?;
    
    if circuit_guard.is_none() {
        *circuit_guard = Some(load_state());
    }
    let circuit = circuit_guard.as_mut().unwrap();
    
    circuit.unregister_view(&id);
    persist_circuit(&circuit);
    
    let result = json!({
        "msg": "View unregistered",
    });

    Ok(result)
}

#[surrealism]
fn save_state(_dummy: Option<Value>) -> Result<Vec<Value>, &'static str> {
    let circuit_guard = CIRCUIT.lock().map_err(|_| "Failed to lock circuit")?;
    if let Some(circuit) = circuit_guard.as_ref() {
        persist_circuit(circuit);
    }
    Ok(vec![])
}
