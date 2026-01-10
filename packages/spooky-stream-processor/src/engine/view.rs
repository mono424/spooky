use super::circuit::Database;
use rustc_hash::FxHashMap; // High-Performance HashMap
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::cmp::Ordering;
use std::hash::Hasher; // Für Blake3 Hasher Trait Nutzung

// --- Data Model ---

pub type Weight = i64;
pub type RowKey = String;

// Wir nutzen FxHashMap statt der Standard-HashMap für interne Berechnungen.
// Sie ist extrem schnell für Integer und Strings.
type FastMap<K, V> = FxHashMap<K, V>;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Row {
    pub data: Value,
}

// A Z-Set is a mapping from Data -> Weight
// WICHTIG: Das muss mit der Definition in circuit.rs übereinstimmen!
pub type ZSet = FastMap<RowKey, Weight>;

// --- ID Tree Implementation ---

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LeafItem {
    pub id: String,
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

        // Basisfall: Wenn wenig Items, direkt hashen (Blattknoten)
        if items.len() <= THRESHOLD {
            let mut hasher = blake3::Hasher::new();
            for item in &items {
                hasher.update(item.id.as_bytes());
                hasher.update(item.hash.as_bytes());
                if let Some(children) = &item.children {
                    // Sorting ist wichtig für deterministisches Hashing
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

        // Rekursiver Fall: Chunking
        // Wir teilen die Liste in feste Blöcke, um Stack Overflow zu verhindern.
        let mut children = FastMap::default();
        let mut child_hashes = Vec::with_capacity(items.len() / THRESHOLD + 1);

        for (i, chunk) in items.chunks(THRESHOLD).enumerate() {
            let child_node = IdTree::build(chunk.to_vec());

            // Als Key nehmen wir den Index als String
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
    pub direction: String,
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
    Gt { field: String, value: Value },
    Lt { field: String, value: Value },
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

    /// Die Hauptfunktion für Updates.
    /// Nutzt Delta-Optimierung, wenn möglich.
    pub fn process(
        &mut self,
        changed_table: &str,
        input_delta: &ZSet,
        db: &Database,
    ) -> Option<MaterializedViewUpdate> {
        // FIX: FIRST RUN CHECK
        // Wenn last_hash leer ist, ist das der allererste Lauf.
        // Wir müssen zwingend einen Full-Scan (eval_snapshot) machen, um den Cache initial zu füllen.
        // Ein reines Delta würde hier nicht reichen, weil der Cache noch leer ist.
        let is_first_run = self.last_hash.is_empty();

        let maybe_delta = if is_first_run {
            None // Erzwingt Fallback auf Snapshot
        } else {
            // Versuche den schnellen Delta-Pfad
            self.eval_delta(
                &self.plan.root,
                changed_table,
                input_delta,
                db,
                self.params.as_ref(),
            )
        };

        let view_delta = if let Some(d) = maybe_delta {
            // TURBO-MODUS: Wir haben das Delta direkt berechnet!
            d
        } else {
            // FALLBACK-MODUS: Full Scan & Diff (langsam, aber sicher)
            let target_set = self.eval_snapshot(&self.plan.root, db, self.params.as_ref());
            let mut diff = FastMap::default();

            // Neues Set prüfen
            for (key, &new_w) in &target_set {
                let old_w = self.cache.get(key).copied().unwrap_or(0);
                if new_w != old_w {
                    diff.insert(key.clone(), new_w - old_w);
                }
            }
            // Alte Einträge prüfen (Gelöschte)
            for (key, &old_w) in &self.cache {
                if !target_set.contains_key(key) {
                    diff.insert(key.clone(), 0 - old_w);
                }
            }
            diff
        };

        // Wenn nichts passiert ist und es nicht der erste Lauf ist -> Abbruch
        if view_delta.is_empty() && !is_first_run {
            return None;
        }

        // Cache aktualisieren (Inkrementell)
        for (key, weight) in &view_delta {
            let entry = self.cache.entry(key.clone()).or_insert(0);
            *entry += weight;
            if *entry == 0 {
                self.cache.remove(key);
            }
        }

        // Cache aktualisieren (Inkrementell)
        for (key, weight) in &view_delta {
            let entry = self.cache.entry(key.clone()).or_insert(0);
            *entry += weight;
            if *entry == 0 {
                self.cache.remove(key);
            }
        }

        // Ergebnis bauen
        let mut result_ids: Vec<String> = self.cache.keys().cloned().collect();
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

    /// Versucht, das Delta rein inkrementell zu berechnen.
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
                    // Wenn dies die geänderte Tabelle ist: Delta durchreichen!
                    Some(input_delta.clone())
                } else {
                    // Andere Tabelle geändert: Kein Einfluss auf diesen Scan
                    Some(FastMap::default())
                }
            }
            Operator::Filter { input, predicate } => {
                let upstream_delta =
                    self.eval_delta(input, changed_table, input_delta, db, context)?;
                let mut out_delta = FastMap::default();

                // Wir filtern nur die Änderungen!
                for (key, weight) in upstream_delta {
                    if self.check_predicate(predicate, &key, db, context) {
                        out_delta.insert(key, weight);
                    }
                }
                Some(out_delta)
            }
            Operator::Project { input, .. } => {
                // Identity Projection für IDs -> Delta durchreichen
                self.eval_delta(input, changed_table, input_delta, db, context)
            }

            // Komplexe Operatoren (Joins, Limits) fallen auf Snapshot zurück
            Operator::Join { .. } | Operator::Limit { .. } => None,
        }
    }

    /// Der klassische Full-Scan Evaluator (für Fallback und Init)
    fn eval_snapshot(&self, op: &Operator, db: &Database, context: Option<&Value>) -> ZSet {
        match op {
            Operator::Scan { table } => {
                if let Some(tb) = db.tables.get(table) {
                    // DB nutzt FxHashMap, wir auch -> clone() ist effizient
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
                            let val_a = row_a
                                .and_then(|r| r.as_object())
                                .and_then(|o| o.get(&ord.field));
                            let val_b = row_b
                                .and_then(|r| r.as_object())
                                .and_then(|o| o.get(&ord.field));

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

                // 1. BUILD PHASE: Baue Index für die RECHTE Seite
                // Map: Wert des Join-Feldes -> Liste von (Key, Weight)
                let mut right_index: FastMap<String, Vec<(&String, &i64)>> = FastMap::default();

                for (r_key, r_weight) in &s_right {
                    if let Some(r_val) = self.get_row_value(r_key, db) {
                        // Wir nutzen eine String-Repräsentation des Wertes als Key für den Join
                        // (Für echte DBs wäre hier ein Hash des Values besser, aber String geht)
                        if let Some(r_field) =
                            r_val.as_object().and_then(|o| o.get(&on.right_field))
                        {
                            let lookup_key = r_field.to_string(); // Alloc, aber notwendig für Map Key
                            right_index
                                .entry(lookup_key)
                                .or_default()
                                .push((r_key, r_weight));
                        }
                    }
                }

                // 2. PROBE PHASE: Iteriere Links und schlage Rechts nach (O(1))
                for (l_key, l_weight) in &s_left {
                    if let Some(l_val) = self.get_row_value(l_key, db) {
                        if let Some(l_field) = l_val.as_object().and_then(|o| o.get(&on.left_field))
                        {
                            let lookup_key = l_field.to_string();

                            // Hash Lookup statt Loop!
                            if let Some(matches) = right_index.get(&lookup_key) {
                                for (r_key, r_weight) in matches {
                                    // Wir haben einen Treffer!
                                    // Strenger Vergleich (falls to_string kollidiert, selten)
                                    // Hier sparen wir uns den compare_json_values meistens
                                    let w = l_weight * *r_weight;
                                    *out.entry(l_key.clone()).or_insert(0) += w;

                                    // HINWEIS: Ein echter Join würde l_key + r_key kombinieren.
                                    // Deine aktuelle Logik behält nur l_key (Semi-Join Verhalten).
                                    // Wenn das Absicht ist, ist es okay.
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
                    let mut sub_ids: Vec<String> = sub_zset.keys().cloned().collect();
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
            id: id.to_string(),
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
            Predicate::And { predicates } => predicates
                .iter()
                .all(|p| self.check_predicate(p, key, db, context)),
            Predicate::Or { predicates } => predicates
                .iter()
                .any(|p| self.check_predicate(p, key, db, context)),
            Predicate::Prefix { prefix } => key.starts_with(prefix),
            Predicate::Eq { field, value } => {
                let target_val = if let Some(obj) = value.as_object() {
                    if let Some(param_path) = obj.get("$param") {
                        if let Some(ctx) = context {
                            let path = param_path.as_str().unwrap_or("");
                            ctx.get(path)
                        } else {
                            None
                        }
                    } else {
                        Some(value)
                    }
                } else {
                    Some(value)
                };

                if target_val.is_none() {
                    return false;
                }
                let target_val = target_val.unwrap();

                let parts: Vec<&str> = key.splitn(2, ':').collect();
                if parts.len() < 2 {
                    return false;
                }
                let table_name = parts[0];

                if field == "id" {
                    let key_val = json!(key);
                    return compare_json_values(Some(&key_val), Some(target_val))
                        == Ordering::Equal;
                }

                if let Some(table) = db.tables.get(table_name) {
                    if let Some(row_val) = table.rows.get(key) {
                        if let Some(obj) = row_val.as_object() {
                            if let Some(f_val) = obj.get(field) {
                                return compare_json_values(Some(f_val), Some(target_val))
                                    == Ordering::Equal;
                            }
                        }
                    }
                }
                false
            }
            Predicate::Gt { field, value } => {
                // GT Logic
                let parts: Vec<&str> = key.splitn(2, ':').collect();
                if parts.len() < 2 {
                    return false;
                }
                let table_name = parts[0];

                if let Some(table) = db.tables.get(table_name) {
                    if let Some(row_val) = table.rows.get(key) {
                        if let Some(obj) = row_val.as_object() {
                            if let Some(f_val) = obj.get(field) {
                                return compare_json_values(Some(f_val), Some(value))
                                    == Ordering::Greater;
                            }
                        }
                    }
                }
                false
            }
            Predicate::Lt { field, value } => {
                // LT Logic
                let parts: Vec<&str> = key.splitn(2, ':').collect();
                if parts.len() < 2 {
                    return false;
                }
                let table_name = parts[0];

                if let Some(table) = db.tables.get(table_name) {
                    if let Some(row_val) = table.rows.get(key) {
                        if let Some(obj) = row_val.as_object() {
                            if let Some(f_val) = obj.get(field) {
                                return compare_json_values(Some(f_val), Some(value))
                                    == Ordering::Less;
                            }
                        }
                    }
                }
                false
            }
        }
    }
}

// --- OPTIMIERTE COMPARISON ---
// Vermeidet Allocations (.to_string) komplett für primitive Typen.
fn compare_json_values(a: Option<&Value>, b: Option<&Value>) -> Ordering {
    match (a, b) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (Some(va), Some(vb)) => {
            match (va, vb) {
                (Value::Null, Value::Null) => Ordering::Equal,
                (Value::Bool(ba), Value::Bool(bb)) => ba.cmp(bb),
                (Value::Number(na), Value::Number(nb)) => {
                    if let (Some(fa), Some(fb)) = (na.as_f64(), nb.as_f64()) {
                        fa.partial_cmp(&fb).unwrap_or(Ordering::Equal)
                    } else {
                        // Fallback für extrem komplexe Numbers (selten)
                        na.to_string().cmp(&nb.to_string())
                    }
                }
                (Value::String(sa), Value::String(sb)) => sa.cmp(sb),
                (Value::Array(aa), Value::Array(ab)) => {
                    let len_cmp = aa.len().cmp(&ab.len());
                    if len_cmp != Ordering::Equal {
                        return len_cmp;
                    }
                    for (ia, ib) in aa.iter().zip(ab.iter()) {
                        let cmp = compare_json_values(Some(ia), Some(ib));
                        if cmp != Ordering::Equal {
                            return cmp;
                        }
                    }
                    Ordering::Equal
                }
                (Value::Object(oa), Value::Object(ob)) => {
                    let len_cmp = oa.len().cmp(&ob.len());
                    if len_cmp != Ordering::Equal {
                        return len_cmp;
                    }
                    // Performance Note: Deep Object compare is expensive.
                    // We assume ordered keys for determinism if needed, but here simple length check first.
                    Ordering::Equal
                }
                (ta, tb) => type_rank(ta).cmp(&type_rank(tb)),
            }
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
