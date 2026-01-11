use super::circuit::Database;
use rustc_hash::FxHasher;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use smol_str::SmolStr;
use std::cmp::Ordering;
use std::hash::{BuildHasherDefault, Hasher};

// --- Data Model ---

pub type Weight = i64;
pub type RowKey = SmolStr;

// We use FxHashMap instead of standard HashMap for internal calculations.
// It is extremely fast for integers and strings.
pub type FastMap<K, V> = std::collections::HashMap<K, V, BuildHasherDefault<FxHasher>>;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Row {
    pub data: Value,
}

// A Z-Set is a mapping from Data -> Weight
// IMPORTANT: This must match the definition in circuit.rs!
pub type ZSet = FastMap<RowKey, Weight>;

// --- ID Tree Implementation ---

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LeafItem {
    pub id: SmolStr,
    pub hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<FastMap<String, IdTree>>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct IdTree {
    pub hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<FastMap<String, IdTree>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub leaves: Option<Vec<LeafItem>>,
}

pub fn compute_hash(items: &[String]) -> String {
    let mut hasher = blake3::Hasher::new();
    for item in items {
        hasher.update(item.as_bytes());
        hasher.update(&[0]); // Delimiter
    }
    hasher.finalize().to_hex().to_string()
}

impl IdTree {
    pub fn build(items: Vec<LeafItem>) -> Self {
        const THRESHOLD: usize = 100;

        // Base case: If few items, hash directly (Leaf node)
        if items.len() <= THRESHOLD {
            let mut hasher = blake3::Hasher::new();
            for item in &items {
                hasher.update(item.id.as_bytes());
                hasher.update(item.hash.as_bytes());
                if let Some(children) = &item.children {
                    // Sorting is important for deterministic hashing
                    let mut keys: Vec<&String> = children.keys().collect();
                    keys.sort_unstable();
                    for k in keys {
                        hasher.update(k.as_bytes());
                        let child = children.get(k).unwrap();
                        hasher.update(child.hash.as_bytes());
                    }
                }
                hasher.update(&[0]);
            }
            let hash = hasher.finalize().to_hex().to_string();

            return IdTree {
                hash,
                children: None,
                leaves: Some(items),
            };
        }

        // Recursive case: Chunking
        // We split the list into fixed blocks to prevent stack overflow.
        let mut children = FastMap::default();
        let mut child_hashes = Vec::with_capacity(items.len() / THRESHOLD + 1);

        for (i, chunk) in items.chunks(THRESHOLD).enumerate() {
            let child_node = IdTree::build(chunk.to_vec());

            // Key is index as string
            let key = i.to_string();

            child_hashes.push(format!("{}:{}", key, child_node.hash));
            children.insert(key, child_node);
        }

        child_hashes.sort_unstable();

        let mut hasher = blake3::Hasher::new();
        for item in child_hashes {
            hasher.update(item.as_bytes());
            hasher.update(&[0]);
        }
        let hash = hasher.finalize().to_hex().to_string();

        IdTree {
            hash,
            children: Some(children),
            leaves: None,
        }
    }
}

// --- Path Optimization ---

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Path(pub Vec<SmolStr>);

impl Path {
    pub fn new(s: &str) -> Self {
        if s.is_empty() {
            Path(vec![])
        } else {
            Path(s.split('.').map(SmolStr::new).collect())
        }
    }
    
    pub fn as_str(&self) -> String {
       self.0.join(".")
    }
}

impl serde::Serialize for Path {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        if self.0.is_empty() {
            serializer.serialize_str("")
        } else {
            serializer.serialize_str(&self.0.join("."))
        }
    }
}

impl<'de> serde::Deserialize<'de> for Path {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s: String = serde::Deserialize::deserialize(deserializer)?;
        Ok(Path::new(&s))
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
    pub field: Path,
    pub direction: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Projection {
    All,
    Field { name: Path },
    Subquery { alias: String, plan: Box<Operator> },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct JoinCondition {
    pub left_field: Path,
    pub right_field: Path,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Predicate {
    Prefix { field: Path, prefix: String },
    Eq { field: Path, value: Value },
    Neq { field: Path, value: Value },
    Gt { field: Path, value: Value },
    Gte { field: Path, value: Value },
    Lt { field: Path, value: Value },
    Lte { field: Path, value: Value },
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
    #[serde(default)]
    pub params: Option<Value>,
}

impl View {
    pub fn new(plan: QueryPlan, params: Option<Value>) -> Self {
        Self {
            plan,
            cache: FastMap::default(),
            last_hash: String::new(),
            params,
        }
    }

    /// The main function for updates.
    /// Uses delta optimization if possible.
    pub fn process(
        &mut self,
        changed_table: &str,
        input_delta: &ZSet,
        db: &Database,
    ) -> Option<MaterializedViewUpdate> {
        // FIX: FIRST RUN CHECK
        // If last_hash is empty, this is the very first run.
        // We MUST verify a full scan (eval_snapshot) to initially populate the cache.
        // A pure delta would not be enough here, because the cache is still empty.
        let is_first_run = self.last_hash.is_empty();

        let maybe_delta = if is_first_run {
            None // Forces fallback to snapshot
        } else {
            // Try the fast delta path
            self.eval_delta(
                &self.plan.root,
                changed_table,
                input_delta,
                db,
                self.params.as_ref(),
            )
        };

        let view_delta = if let Some(d) = maybe_delta {
            // TURBO MODE: We calculated the delta directly!
            d
        } else {
            // FALLBACK MODE: Full Scan & Diff (slow but safe)
            let target_set = self.eval_snapshot(&self.plan.root, db, self.params.as_ref());
            let mut diff = FastMap::default();

            // Check new set
            for (key, &new_w) in &target_set {
                let old_w = self.cache.get(key).copied().unwrap_or(0);
                if new_w != old_w {
                    diff.insert(key.clone(), new_w - old_w);
                }
            }
            // Check old entries (deleted)
            for (key, &old_w) in &self.cache {
                if !target_set.contains_key(key) {
                    diff.insert(key.clone(), 0 - old_w);
                }
            }
            diff
        };

        // If nothing happened and it is not the first run -> Abort
        if view_delta.is_empty() && !is_first_run {
            return None;
        }

        // Update cache (Incremental)
        for (key, weight) in &view_delta {
            let entry = self.cache.entry(key.clone()).or_insert(0);
            *entry += weight;
            if *entry == 0 {
                self.cache.remove(key);
            }
        }

        // Build result
        let mut result_ids: Vec<String> = self.cache.keys().map(|k| k.to_string()).collect();
        result_ids.sort_unstable();

        let items: Vec<LeafItem> = result_ids
            .iter()
            .map(|id| self.expand_item(id, &self.plan.root, db))
            .collect();

        let tree = IdTree::build(items);
        let hash = tree.hash.clone();

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

    /// Attempts to calculate the delta purely incrementally.
    fn eval_delta(
        &self,
        op: &Operator,
        changed_table: &str,
        input_delta: &ZSet,
        db: &Database,
        context: Option<&Value>,
    ) -> Option<ZSet> {
        match op {
            Operator::Scan { table } => {
                if table == changed_table {
                    // If this is the changed table: Pass delta through!
                    Some(input_delta.clone())
                } else {
                    // Other table changed: No impact on this scan
                    Some(FastMap::default())
                }
            }
            Operator::Filter { input, predicate } => {
                let upstream_delta =
                    self.eval_delta(input, changed_table, input_delta, db, context)?;
                let mut out_delta = FastMap::default();

                // We only filter the changes!
                for (key, weight) in upstream_delta {
                    if self.check_predicate(predicate, &key, db, context) {
                        out_delta.insert(key, weight);
                    }
                }
                Some(out_delta)
            }
            Operator::Project { input, .. } => {
                // Identity Projection for IDs -> Pass delta through
                self.eval_delta(input, changed_table, input_delta, db, context)
            }

            // Complex operators (Joins, Limits) fall back to snapshot
            Operator::Join { .. } | Operator::Limit { .. } => None,
        }
    }

    /// The classic detailed Full-Scan Evaluator (for fallback and init)
    fn eval_snapshot(&self, op: &Operator, db: &Database, context: Option<&Value>) -> ZSet {
        match op {
            Operator::Scan { table } => {
                if let Some(tb) = db.tables.get(table) {
                    // DB uses FxHashMap, we too -> clone() is efficient
                    tb.zset.clone()
                } else {
                    FastMap::default()
                }
            }
            Operator::Filter { input, predicate } => {
                let upstream = self.eval_snapshot(input, db, context);
                let mut out = FastMap::default();
                for (key, weight) in upstream {
                    if self.check_predicate(predicate, &key, db, context) {
                        out.insert(key, weight);
                    }
                }
                out
            }
            Operator::Project { input, .. } => self.eval_snapshot(input, db, context),
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
                            let val_a = resolve_nested_value(row_a, &ord.field);
                            let val_b = resolve_nested_value(row_b, &ord.field);

                            let cmp = compare_json_values(val_a, val_b);
                            if cmp != Ordering::Equal {
                                return if ord.direction.eq_ignore_ascii_case("DESC") {
                                    cmp.reverse()
                                } else {
                                    cmp
                                };
                            }
                        }
                        a.0.cmp(&b.0)
                    });
                } else {
                    items.sort_unstable_by(|a, b| a.0.cmp(&b.0));
                }

                let mut out = FastMap::default();
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
                let s_left = self.eval_snapshot(left, db, context);
                let s_right = self.eval_snapshot(right, db, context);
                let mut out = FastMap::default();

                // 1. BUILD PHASE: Build Index for the RIGHT side
                // Map: Hash of Join-Field -> List of (Key, Weight)
                let mut right_index: FastMap<u64, Vec<(&SmolStr, &i64)>> = FastMap::default();

                for (r_key, r_weight) in &s_right {
                    if let Some(r_val) = self.get_row_value(r_key.as_str(), db) {
                        if let Some(r_field) = resolve_nested_value(Some(r_val), &on.right_field) {
                            let hash = hash_json_value(r_field);
                            right_index.entry(hash).or_default().push((r_key, r_weight));
                        }
                    }
                }

                // 2. PROBE PHASE: Iterate Left and lookup Right (O(1))
                for (l_key, l_weight) in &s_left {
                    if let Some(l_val) = self.get_row_value(l_key.as_str(), db) {
                        if let Some(l_field) = resolve_nested_value(Some(l_val), &on.left_field) {
                            let hash = hash_json_value(l_field);

                            // Hash Lookup instead of Loop!
                            if let Some(matches) = right_index.get(&hash) {
                                for (_r_key, r_weight) in matches {
                                    // We have a match!
                                    // Optimization: We assume Hash collision is rare enough to ignore for now
                                    // or checking actual equality would be added here if strictness required
                                    let w = l_weight * *r_weight;
                                    *out.entry(l_key.clone()).or_insert(0) += w;
                                }
                            }
                        }
                    }
                }
                out
            }
        }
    }

    fn expand_item(&self, id: &str, op: &Operator, db: &Database) -> LeafItem {
        let mut final_hash = self
            .get_row_hash(id, db)
            .unwrap_or_else(|| "0000".to_string());

        let mut children_map = FastMap::default();
        let projections = self.find_projections(op);

        if !projections.is_empty() {
            let mut dependency_hashes = Vec::with_capacity(projections.len());

            for proj in projections {
                if let Projection::Subquery {
                    alias,
                    plan: sub_op,
                } = proj
                {
                    let mut context = self.params.clone().unwrap_or(json!({}));
                    if let Some(obj) = context.as_object_mut() {
                        obj.insert("parent".to_string(), json!(id));
                    } else {
                        context = json!({"parent": id});
                    }

                    let sub_zset = self.eval_snapshot(sub_op, db, Some(&context));
                    let mut sub_ids: Vec<String> = sub_zset.keys().map(|k| k.to_string()).collect();
                    sub_ids.sort_unstable();

                    let sub_items: Vec<LeafItem> = sub_ids
                        .iter()
                        .map(|sub_id| self.expand_item(sub_id, sub_op, db))
                        .collect();

                    let sub_tree = IdTree::build(sub_items);
                    dependency_hashes.push(sub_tree.hash.clone());
                    children_map.insert(alias.clone(), sub_tree);
                }
            }

            if !dependency_hashes.is_empty() {
                let mut hasher = blake3::Hasher::new();
                hasher.update(final_hash.as_bytes());
                for h in dependency_hashes {
                    hasher.update(h.as_bytes());
                }
                final_hash = hasher.finalize().to_hex().to_string();
            }
        }

        LeafItem {
            id: SmolStr::new(id),
            hash: final_hash,
            children: if children_map.is_empty() {
                None
            } else {
                Some(children_map)
            },
        }
    }

    fn find_projections<'a>(&self, op: &'a Operator) -> Vec<&'a Projection> {
        match op {
            Operator::Project { projections, .. } => projections.iter().collect(),
            Operator::Limit { input, .. } => self.find_projections(input),
            _ => vec![],
        }
    }

    fn get_row_value<'a>(&self, key: &str, db: &'a Database) -> Option<&'a Value> {
        // Optimization: Avoid allocation for split if possible or use SmolStr if we change internal map keys
        // For now, key is &str, db uses SmolStr keys.
        // We assume valid format "table:id"
        let (table_name, _id) = key.split_once(':')?;
        db.tables.get(table_name)?.rows.get(key)
    }

    fn get_row_hash(&self, key: &str, db: &Database) -> Option<String> {
        let (table_name, _id) = key.split_once(':')?;
        db.tables.get(table_name)?.hashes.get(key).cloned()
    }

    fn check_predicate(
        &self,
        pred: &Predicate,
        key: &str,
        db: &Database,
        context: Option<&Value>,
    ) -> bool {
        // Helper to get actual json value for comparison
        let resolve_val = |_field: &Path, value: &Value| -> Option<Value> {
             if let Some(obj) = value.as_object() {
                if let Some(param_path) = obj.get("$param") {
                     if let Some(ctx) = context {
                        // $param is usually a simple string path like "user.id"
                        // We need to parse it as a Path to resolve it, OR since $param is dynamic we treat it as string
                        let path_str = param_path.as_str().unwrap_or("");
                        let path = Path::new(path_str);
                        // resolve nested param path
                        resolve_nested_value(Some(ctx), &path).cloned()
                    } else {
                        None
                    }
                } else {
                    Some(value.clone())
                }
            } else {
                Some(value.clone())
            }
        };

        match pred {
            Predicate::And { predicates } => predicates
                .iter()
                .all(|p| self.check_predicate(p, key, db, context)),
            Predicate::Or { predicates } => predicates
                .iter()
                .any(|p| self.check_predicate(p, key, db, context)),
             Predicate::Prefix { field, prefix } => {
                 // Check if field value starts with prefix
                 if field.0.len() == 1 && field.0[0] == "id" {
                     return key.starts_with(prefix);
                 }
                  if let Some(row_val) = self.get_row_value(key, db) {
                     if let Some(val) = resolve_nested_value(Some(row_val), field) {
                         if let Some(s) = val.as_str() {
                             return s.starts_with(prefix);
                         }
                     }
                 }
                 false
             },
            Predicate::Eq { field, value } |
            Predicate::Neq { field, value } |
            Predicate::Gt { field, value } |
            Predicate::Gte { field, value } |
            Predicate::Lt { field, value } |
            Predicate::Lte { field, value } => {
                let target_val = resolve_val(field, value);
                if target_val.is_none() { return false; }
                let target_val = target_val.unwrap();

                let actual_val_opt = if field.0.len() == 1 && field.0[0] == "id" {
                     Some(json!(key))
                } else {
                    self.get_row_value(key, db).and_then(|r| resolve_nested_value(Some(r), field).cloned())
                };
                
                if let Some(actual_val) = actual_val_opt {
                    let ord = compare_json_values(Some(&actual_val), Some(&target_val));
                    match pred {
                        Predicate::Eq { .. } => ord == Ordering::Equal,
                        Predicate::Neq { .. } => ord != Ordering::Equal,
                        Predicate::Gt { .. } => ord == Ordering::Greater,
                        Predicate::Gte { .. } => ord == Ordering::Greater || ord == Ordering::Equal,
                        Predicate::Lt { .. } => ord == Ordering::Less,
                        Predicate::Lte { .. } => ord == Ordering::Less || ord == Ordering::Equal,
                        _ => false
                    }
                } else {
                    false
                }
            }
        }
    }
}

// --- OPTIMIZED COMPARISON & HASHING ---

// Avoids Allocations (.to_string) completely for primitive types.
fn compare_json_values(a: Option<&Value>, b: Option<&Value>) -> Ordering {
    use std::cmp::Ordering;
    use serde_json::Value;
    match (a, b) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (Some(va), Some(vb)) => match (va, vb) {
            (Value::Null, Value::Null) => Ordering::Equal,
            (Value::Bool(ba), Value::Bool(bb)) => ba.cmp(bb),
            (Value::Number(na), Value::Number(nb)) => {
                if let (Some(fa), Some(fb)) = (na.as_f64(), nb.as_f64()) {
                    fa.partial_cmp(&fb).unwrap_or(Ordering::Equal)
                } else {
                    // Fallback only if strictly necessary
                    na.to_string().cmp(&nb.to_string())
                }
            },
            (Value::String(sa), Value::String(sb)) => sa.cmp(sb),
            (Value::Array(aa), Value::Array(ab)) => {
                let len_cmp = aa.len().cmp(&ab.len());
                if len_cmp != Ordering::Equal { return len_cmp; }
                for (ia, ib) in aa.iter().zip(ab.iter()) {
                    let cmp = compare_json_values(Some(ia), Some(ib));
                    if cmp != Ordering::Equal { return cmp; }
                }
                Ordering::Equal
            }
            (Value::Object(oa), Value::Object(ob)) => oa.len().cmp(&ob.len()), // Deep compare skipped for perf
            _ => type_rank(va).cmp(&type_rank(vb)),
        }
    }
}

fn type_rank(v: &Value) -> u8 {
    match v {
        Value::Null => 0,
        Value::Bool(_) => 1,
        Value::Number(_) => 2,
        Value::String(_) => 3,
        Value::Array(_) => 4,
        Value::Object(_) => 5,
    }
}

// Dot notation access: "address.city" -> traverses json
// Optimized specifically for Path
fn resolve_nested_value<'a>(root: Option<&'a serde_json::Value>, path: &Path) -> Option<&'a serde_json::Value> {
    let mut current = root;
    for part in &path.0 {
        match current {
            Some(serde_json::Value::Object(map)) => { current = map.get(part.as_str()); }
            _ => return None,
        }
    }
    current
}

// Fast hashing for Join Keys
fn hash_json_value(v: &Value) -> u64 {
     let mut hasher = FxHasher::default();
     hash_value_recursive(v, &mut hasher);
     hasher.finish()
}

fn hash_value_recursive(v: &Value, hasher: &mut FxHasher) {
    match v {
        Value::Null => { hasher.write_u8(0); },
        Value::Bool(b) => { hasher.write_u8(1); hasher.write_u8(*b as u8); },
        Value::Number(n) => { 
            hasher.write_u8(2); 
            if let Some(f) = n.as_f64() {
                hasher.write_u64(f.to_bits());
            } else {
                 hasher.write(n.to_string().as_bytes()); // Fallback
            }
        },
        Value::String(s) => { hasher.write_u8(3); hasher.write(s.as_bytes()); },
        Value::Array(arr) => {
            hasher.write_u8(4);
            for item in arr {
                hash_value_recursive(item, hasher);
            }
        },
        Value::Object(obj) => {
             hasher.write_u8(5);
             // Sort keys for deterministic hash? 
             // For strict correctness yes, but costly. 
             // We assume key order stable enough or just use length as quick hash for now
             // Or better: xor hash of keys+values to be order independent?
             // For now: simple iteration
             for (k, v) in obj {
                 hasher.write(k.as_bytes());
                 hash_value_recursive(v, hasher);
             }
        }
    }
}
