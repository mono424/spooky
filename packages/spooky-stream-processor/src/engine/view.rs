use crate::engine::circuit::Database;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap; // Forward declaration logic

// --- Data Model ---

pub type Weight = i64;
pub type RowKey = String;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Row {
    pub data: Value,
}

// A Z-Set is a mapping from Data -> Weight
pub type ZSet = HashMap<RowKey, Weight>;

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
            let prefix = item
                .id
                .chars()
                .next()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "".to_string());
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
    Scan {
        table: String,
    },
    Filter {
        input: Box<Operator>,
        predicate: Predicate,
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
    },
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
    Subquery { alias: String, plan: Box<Operator> },
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

#[derive(Serialize, Deserialize, Debug)]
pub struct MaterializedViewUpdate {
    pub query_id: String,
    pub result_hash: String,
    pub result_ids: Vec<String>,
    pub tree: IdTree,
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
    pub fn process(
        &mut self,
        _changed_table: &str,
        _input_delta: &ZSet,
        db: &Database,
    ) -> Option<MaterializedViewUpdate> {
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
        let items: Vec<LeafItem> = result_ids
            .iter()
            .map(|id| {
                // Get Hash from DB Cache
                let hash = self
                    .get_row_hash(id, db)
                    .unwrap_or_else(|| "0000".to_string());
                LeafItem {
                    id: id.clone(),
                    hash,
                }
            })
            .collect();

        // Compute root hash from items
        // Note: IdTree::build computes its own hash, but we check change first.
        let tree = IdTree::build(items);
        let hash = tree.hash.clone();

        eprintln!(
            "DEBUG: process: view={}, delta_len={}, hash={}, last_hash={}",
            self.plan.id,
            view_delta.len(),
            hash,
            self.last_hash
        );

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
        // Break huge match into private methods per op if preferred, but existing structure is fine for now
        match op {
            Operator::Scan { table } => {
                if let Some(tb) = db.tables.get(table) {
                    // println!("DEBUG: eval_snapshot SCAN table {} found items: {}", table, tb.zset.len());
                    tb.zset.clone()
                } else {
                    // println!("DEBUG: eval_snapshot SCAN table {} NOT FOUND", table);
                    HashMap::new()
                }
            }
            Operator::Filter { input, predicate } => {
                let upstream = self.eval_snapshot(input, db, context);
                let mut out = HashMap::new();
                for (key, weight) in upstream {
                    if self.check_predicate(predicate, &key, db, context) {
                        out.insert(key, weight);
                    } else {
                        // println!("DEBUG: Filter REJECTED key {} with pred {:?}", key, predicate);
                    }
                }
                out
            }
            Operator::Project {
                input,
                projections: _,
            } => {
                // Identity for ZSet (ID set) unless we implement Map
                self.eval_snapshot(input, db, context)
            }
            Operator::Limit {
                input,
                limit,
                order_by,
            } => {
                let upstream = self.eval_snapshot(input, db, context);
                let mut items: Vec<_> = upstream.into_iter().collect();

                if let Some(orders) = order_by {
                    items.sort_by(|a, b| {
                        let row_a = self.get_row_value(&a.0, db);
                        let row_b = self.get_row_value(&b.0, db);

                        for ord in orders {
                            let val_a = row_a
                                .and_then(|r| r.as_object())
                                .and_then(|o| o.get(&ord.field));
                            let val_b = row_b
                                .and_then(|r| r.as_object())
                                .and_then(|o| o.get(&ord.field));

                            let cmp = compare_json_values(val_a, val_b);
                            if cmp != std::cmp::Ordering::Equal {
                                return if ord.direction.eq_ignore_ascii_case("DESC") {
                                    cmp.reverse()
                                } else {
                                    cmp
                                };
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
            }
            Operator::Join { left, right, on } => {
                // Nested Loop Join on Full Snapshots
                let s_left = self.eval_snapshot(left, db, context);
                let s_right = self.eval_snapshot(right, db, context);
                let mut out = HashMap::new();

                for (l_key, l_weight) in &s_left {
                    let l_val_opt = self.get_row_value(l_key, db);
                    if l_val_opt.is_none() {
                        continue;
                    }
                    let l_val = l_val_opt.unwrap();

                    // Get Join Field Value
                    let l_field_val = l_val.as_object().and_then(|o| o.get(&on.left_field));
                    if l_field_val.is_none() {
                        continue;
                    }

                    for (r_key, r_weight) in &s_right {
                        let r_val_opt = self.get_row_value(r_key, db);
                        if let Some(r_val) = r_val_opt {
                            let r_field_val =
                                r_val.as_object().and_then(|o| o.get(&on.right_field));
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
        if parts.len() < 2 {
            return None;
        }
        db.tables.get(parts[0])?.rows.get(key)
    }

    fn get_row_hash(&self, key: &str, db: &Database) -> Option<String> {
        let parts: Vec<&str> = key.splitn(2, ':').collect();
        if parts.len() < 2 {
            return None;
        }
        db.tables.get(parts[0])?.hashes.get(key).cloned()
    }

    fn check_predicate(
        &self,
        pred: &Predicate,
        key: &str,
        db: &Database,
        context: Option<&Value>,
    ) -> bool {
        match pred {
            Predicate::And { predicates } => {
                for p in predicates {
                    if !self.check_predicate(p, key, db, context) {
                        return false;
                    }
                }
                true
            }
            Predicate::Or { predicates } => {
                for p in predicates {
                    if self.check_predicate(p, key, db, context) {
                        return true;
                    }
                }
                false
            }
            Predicate::Prefix { prefix } => key.starts_with(prefix),
            Predicate::Eq { field, value } => {
                // Check if value is a param
                let target_val = if let Some(obj) = value.as_object() {
                    if let Some(param_path) = obj.get("$param") {
                        // Resolve param from context
                        if let Some(ctx) = context {
                            if let Some(ctx_val) = ctx.get(param_path.as_str().unwrap_or("")) {
                                // println!("DEBUG: Resolved param {}: {:?}", param_path, ctx_val);
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
                if parts.len() < 2 {
                    return false;
                }
                let table_name = parts[0];

                if field == "id" {
                    let key_val = json!(key);
                    return compare_json_values(Some(&key_val), Some(target_val))
                        == std::cmp::Ordering::Equal;
                }

                if let Some(table) = db.tables.get(table_name) {
                    if let Some(row_val) = table.rows.get(key) {
                        if let Some(obj) = row_val.as_object() {
                            // Handle nested fields? e.g. "author.name"
                            // Simple field access for now
                            if let Some(f_val) = obj.get(field) {
                                return compare_json_values(Some(f_val), Some(target_val))
                                    == std::cmp::Ordering::Equal;
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
