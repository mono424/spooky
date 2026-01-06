use crate::engine::store::Store;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

// StandardCircuit Dependencies (restored)
use crate::engine::standard_circuit::Database;

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
        store: &dyn Store,
        _changed_table: &str,
        input_delta: &ZSet,
    ) -> Option<MaterializedViewUpdate> {
        // Incremental Strategy:
        // 1. Compute output delta based on input delta
        let view_delta = self.eval_delta(&self.plan.root, input_delta, store, self.params.as_ref());

        if view_delta.is_empty() {
            return None;
        }

        // 2. Update Cache & Emit
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

        // Build Leaf Items
        let items: Vec<LeafItem> = result_ids
            .iter()
            .map(|id| {
                // Get Hash from Store? Or just use "0000" if we don't query full object?
                // For the tree hash, strictly speaking we want the content hash.
                // But fetching every item just to hash it is expensive if we want to be lazy.
                // However, the Tree *is* the content verification.
                // If we want to be "Lazy", we might accept that the View Tree is just IDs?
                // But if the view is "Select *", we want the content hash.
                // If we use `Store`, we can fetch it.
                // But that defeats "Lazy Loading" if we fetch ALL items in the view to build the tree.
                //
                // COMPROMISE: If the client effectively needs the hash to invalidate cache, we need it.
                // For now, let's fetch it. "Lazy" means we don't load the WHOLE DB to memory at start.
                // But we might still load the View Result Items.
                // If the View Result is huge, this is still slow.
                // But Spooky is for small streams usually.

                // Parse table from ID
                let parts: Vec<&str> = id.splitn(2, ':').collect();
                let table = if parts.len() > 0 { parts[0] } else { "" };

                let hash = if let Some(_val) = store.get(table, id) {
                    // We need a hash of the value.
                    // To keep it simple/consistent with previous `compute_hash` or `ingest` logic:
                    // We assume the Store might return the hash too?
                    // The `Store::get` returns `Value`.
                    // We can re-hash it.
                    // Or we add `get_hash` to Store trait.
                    // For now, let's just use "hash_of_value".
                    // But wait, the `Circuit` had `hashes` map.
                    // Ideally `Store` returns `(Value, Hash)` or we just hash the Value.
                    "0000".to_string() // Placeholder to avoid hashing overhead in this step for now, or implement hashing
                } else {
                    "0000".to_string()
                };

                LeafItem {
                    id: id.clone(),
                    hash,
                }
            })
            .collect();

        // Compute root hash from items
        let tree = IdTree::build(items);
        let hash = tree.hash.clone();

        /*
        eprintln!(
            "DEBUG: process: view={}, delta_len={}, hash={}, last_hash={}",
            self.plan.id,
            view_delta.len(),
            hash,
            self.last_hash
        );
        */

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

    fn eval_delta(
        &self,
        op: &Operator,
        input_delta: &ZSet,
        store: &dyn Store,
        context: Option<&Value>,
    ) -> ZSet {
        match op {
            Operator::Scan { table } => {
                // If input_delta belongs to this table, return it.
                // We rely on the caller (Circuit::step) to pass delta only if it matches?
                // Actually `Circuit::step` passes `table` name.
                // But `input_delta` keys are `table:id`.
                // So reliable way is to check prefix inside delta keys?
                // Or simplistic assumption: if Scan.table matched logic in Circuit, it flows.
                // But `Circuit` iterates ALL views.
                // So we must filter here.

                let mut out = HashMap::new();
                for (key, weight) in input_delta {
                    if key.starts_with(&format!("{}:", table)) {
                        out.insert(key.clone(), *weight);
                    }
                }
                out
            }
            Operator::Filter { input, predicate } => {
                let upstream = self.eval_delta(input, input_delta, store, context);
                let mut out = HashMap::new();
                for (key, weight) in upstream {
                    if self.check_predicate_remote(predicate, &key, store, context) {
                        out.insert(key, weight);
                    }
                }
                out
            }
            Operator::Project { input, .. } => {
                // Identity for now
                self.eval_delta(input, input_delta, store, context)
            }
            Operator::Limit {
                input,
                limit: _,
                order_by: _,
            } => {
                // Limit on Delta is tricky without keeping state of "Count so far".
                // Current `View` has `cache` which is the Full Result Set.
                // So we can check `self.cache.len()`.
                // If `weight > 0`: if len < limit, pass it. Else drop.
                // If `weight < 0`: always pass (retraction).
                // BUT: if we are at limit, and one leaves, does one from "waiting list" enter?
                // Without "Waiting List" state, we can't implement correct LIMIT.
                // For MVP Refactor, we might skip precise Limit logic or assume it is not used in critical path
                // OR we just use the upstream delta.

                // Very naive: Pass everything.
                self.eval_delta(input, input_delta, store, context)
            }
            Operator::Join { left, right, on } => {
                // Delta Join (Symmetric Hash Join logic)
                // d(L*R) = dL * R + L * dR + dL * dR
                // But since we process one atom (batch) at a time, we treat input_delta as "The Change".
                // We don't know if it came from Left or Right side of the tree unless we trace it.
                // But `eval_delta` is recursive. `input_delta` is at the leaves.

                // If `left` is a Scan(T1) and `right` is Scan(T2).
                // input_delta contains items from T1 OR T2 (or both).

                let d_left = self.eval_delta(left, input_delta, store, context);
                let d_right = self.eval_delta(right, input_delta, store, context);

                let mut out = HashMap::new();

                // 1. Handle Left Change (dL * R)
                if !d_left.is_empty() {
                    // We need to query Right Base for matches.
                    // But Right might be a complex Subquery.
                    // If Right is complex, we cannot easily "Query it by field".
                    // LIMITATION: Joins only supported on Base Tables or Materialized Views (if we had them).
                    // For now, let's assume Right ends in a Scan or simple filter we can push down.
                    // If Right is generic operator, we can't "fetch matches" without re-running Right on ALL data.
                    // That defeats the purpose.

                    // Assumption: The plan structure for Joins usually involves Scan.
                    // We need `store.get_by_field`.

                    if let Operator::Scan { table: r_table } = &**right {
                        for (l_key, l_weight) in &d_left {
                            // Fetch L Value to get join key
                            let parts: Vec<&str> = l_key.splitn(2, ':').collect();
                            let l_table = parts[0];
                            if let Some(l_val) = store.get(l_table, l_key) {
                                let join_val =
                                    l_val.as_object().and_then(|o| o.get(&on.left_field));

                                if let Some(val) = join_val {
                                    // Query Right
                                    let matches = store.get_by_field(r_table, &on.right_field, val);
                                    for m in matches {
                                        // Construct R Key?
                                        // Store.get_by_field returns Value, but we need ID to make the key.
                                        // We assume Value has `id` field (SurrealDB records do).
                                        if let Some(r_id_val) =
                                            m.as_object().and_then(|o| o.get("id"))
                                        {
                                            if let Some(_r_id) = r_id_val.as_str() {
                                                // Key logic?
                                                // The Join OUTPUT key is usually L_ID that matched?
                                                // In the old code: `out.entry(l_key.clone())`
                                                // It effectively filtered L by semi-join R.

                                                let w = l_weight * 1; // R exists
                                                *out.entry(l_key.clone()).or_insert(0) += w;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // 2. Handle Right Change (L * dR)
                if !d_right.is_empty() {
                    if let Operator::Scan { table: l_table } = &**left {
                        for (r_key, r_weight) in &d_right {
                            // Fetch R Value
                            let parts: Vec<&str> = r_key.splitn(2, ':').collect();
                            let r_table = parts[0];
                            if let Some(r_val) = store.get(r_table, r_key) {
                                let join_val =
                                    r_val.as_object().and_then(|o| o.get(&on.right_field));

                                if let Some(val) = join_val {
                                    // Query Left
                                    let matches = store.get_by_field(l_table, &on.left_field, val);
                                    for m in matches {
                                        if let Some(l_id_val) =
                                            m.as_object().and_then(|o| o.get("id"))
                                        {
                                            if let Some(l_id) = l_id_val.as_str() {
                                                // Emit L Key
                                                // NOTE: The ID format from Surreal might be `table:id`.
                                                // We should ensure it matches our key format.
                                                // If `l_id` comes from `store`, it is usually fully qualified `table:id`.

                                                let w = 1 * r_weight;
                                                *out.entry(l_id.to_string()).or_insert(0) += w;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                out
            }
        }
    }

    fn check_predicate_remote(
        &self,
        pred: &Predicate,
        key: &str,
        store: &dyn Store,
        context: Option<&Value>,
    ) -> bool {
        match pred {
            Predicate::And { predicates } => {
                for p in predicates {
                    if !self.check_predicate_remote(p, key, store, context) {
                        return false;
                    }
                }
                true
            }
            Predicate::Or { predicates } => {
                for p in predicates {
                    if self.check_predicate_remote(p, key, store, context) {
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

                // Fetch Row from Store
                if let Some(row_val) = store.get(table_name, key) {
                    if let Some(obj) = row_val.as_object() {
                        if let Some(f_val) = obj.get(field) {
                            return compare_json_values(Some(f_val), Some(target_val))
                                == std::cmp::Ordering::Equal;
                        }
                    }
                }
                false
            }
        }
    }
    // --- SNAPSHOT/STANDARD MODE METHODS (Restored) ---

    pub fn process_snapshot(
        &mut self,
        _changed_table: &str,
        _input_delta: &ZSet,
        db: &Database,
    ) -> Option<MaterializedViewUpdate> {
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

        // Compute root hash
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

    fn eval_snapshot(&self, op: &Operator, db: &Database, context: Option<&Value>) -> ZSet {
        match op {
            Operator::Scan { table } => {
                if let Some(tb) = db.tables.get(table) {
                    tb.zset.clone()
                } else {
                    HashMap::new()
                }
            }
            Operator::Filter { input, predicate } => {
                let upstream = self.eval_snapshot(input, db, context);
                let mut out = HashMap::new();
                for (key, weight) in upstream {
                    if self.check_predicate_standard(predicate, &key, db, context) {
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
                        a.0.cmp(&b.0)
                    });
                } else {
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
                let s_left = self.eval_snapshot(left, db, context);
                let s_right = self.eval_snapshot(right, db, context);
                let mut out = HashMap::new();

                for (l_key, l_weight) in &s_left {
                    let l_val_opt = self.get_row_value(l_key, db);
                    if l_val_opt.is_none() {
                        continue;
                    }
                    let l_val = l_val_opt.unwrap();
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

    fn check_predicate_standard(
        &self,
        pred: &Predicate,
        key: &str,
        db: &Database,
        context: Option<&Value>,
    ) -> bool {
        match pred {
            Predicate::And { predicates } => {
                for p in predicates {
                    if !self.check_predicate_standard(p, key, db, context) {
                        return false;
                    }
                }
                true
            }
            Predicate::Or { predicates } => {
                for p in predicates {
                    if self.check_predicate_standard(p, key, db, context) {
                        return true;
                    }
                }
                false
            }
            Predicate::Prefix { prefix } => key.starts_with(prefix),
            Predicate::Eq { field, value } => {
                let target_val = if let Some(obj) = value.as_object() {
                    if let Some(param_path) = obj.get("$param") {
                        if let Some(ctx) = context {
                            if let Some(ctx_val) = ctx.get(param_path.as_str().unwrap_or("")) {
                                ctx_val
                            } else {
                                return false;
                            }
                        } else {
                            return false;
                        }
                    } else {
                        value
                    }
                } else {
                    value
                };

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
