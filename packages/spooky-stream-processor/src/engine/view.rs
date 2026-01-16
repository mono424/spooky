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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SpookyValue {
    Null,
    Bool(bool),
    Number(f64),
    Str(SmolStr),
    Array(Vec<SpookyValue>),
    Object(FastMap<SmolStr, SpookyValue>),
}

impl From<serde_json::Value> for SpookyValue {
    fn from(v: serde_json::Value) -> Self {
        match v {
            serde_json::Value::Null => SpookyValue::Null,
            serde_json::Value::Bool(b) => SpookyValue::Bool(b),
            serde_json::Value::Number(n) => SpookyValue::Number(n.as_f64().unwrap_or(0.0)), // Simplified fallback
            serde_json::Value::String(s) => SpookyValue::Str(SmolStr::from(s)),
            serde_json::Value::Array(arr) => {
                SpookyValue::Array(arr.into_iter().map(SpookyValue::from).collect())
            }
            serde_json::Value::Object(obj) => SpookyValue::Object(
                obj.into_iter()
                    .map(|(k, v)| (SmolStr::from(k), SpookyValue::from(v)))
                    .collect(),
            ),
        }
    }
}

// Convert back to serde_json::Value for compatibility (if needed)
impl From<SpookyValue> for serde_json::Value {
    fn from(val: SpookyValue) -> Self {
        match val {
            SpookyValue::Null => serde_json::Value::Null,
            SpookyValue::Bool(b) => serde_json::Value::Bool(b),
            SpookyValue::Number(n) => json!(n),
            SpookyValue::Str(s) => serde_json::Value::String(s.to_string()),
            SpookyValue::Array(arr) => {
                serde_json::Value::Array(arr.into_iter().map(|v| v.into()).collect())
            }
            SpookyValue::Object(obj) => serde_json::Value::Object(
                obj.into_iter()
                    .map(|(k, v)| (k.to_string(), v.into()))
                    .collect(),
            ),
        }
    }
}

// A Z-Set is a mapping from Data -> Weight
// IMPORTANT: This must match the definition in circuit.rs!
pub type ZSet = FastMap<RowKey, Weight>;

// --- Version Tracking Implementation ---

// Version map: record_id -> version number
pub type VersionMap = FastMap<String, u64>;

/// Compute hash from a flat array of [record-id, version] pairs
pub fn compute_flat_hash(data: &[(String, u64)]) -> String {
    let mut hasher = blake3::Hasher::new();
    for (id, version) in data {
        hasher.update(id.as_bytes());
        hasher.update(&version.to_le_bytes());
        hasher.update(&[0]); // Delimiter
    }
    hasher.finalize().to_hex().to_string()
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
    pub result_data: Vec<(String, u64)>, // [[record-id, version], ...]
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct View {
    pub plan: QueryPlan,
    pub cache: ZSet,
    pub last_hash: String,
    #[serde(default)]
    pub params: Option<SpookyValue>,
    #[serde(default)]
    pub version_map: VersionMap, // Track versions for each record
}

impl View {
    pub fn new(plan: QueryPlan, params: Option<Value>) -> Self {
        Self {
            plan,
            cache: FastMap::default(),
            last_hash: String::new(),
            params: params.map(SpookyValue::from),
            version_map: FastMap::default(),
        }
    }

    /// The main function for updates.
    /// Uses delta optimization if possible.
    pub fn process(
        &mut self,
        changed_table: &str,
        input_delta: &ZSet,
        db: &Database,
        is_optimistic: bool,
    ) -> Option<MaterializedViewUpdate> {
        let mut deltas = FastMap::default();
        if !changed_table.is_empty() {
            deltas.insert(changed_table.to_string(), input_delta.clone());
        }
        self.process_ingest(&deltas, db, is_optimistic)
    }

    /// Optimized 2-Phase Processing: Handles multiple table updates at once.
    /// is_optimistic: true = increment versions (local mutations), false = keep versions (remote sync)
    pub fn process_ingest(
        &mut self,
        deltas: &FastMap<String, ZSet>,
        db: &Database,
        is_optimistic: bool,
    ) -> Option<MaterializedViewUpdate> {
        // FIX: FIRST RUN CHECK
        let is_first_run = self.last_hash.is_empty();

        // Check if any delta contains CREATE or DELETE operations for tables used in subqueries
        let has_subquery_changes = !is_first_run && self.has_changes_for_subqueries(deltas, db);

        #[cfg(target_arch = "wasm32")]
        web_sys::console::log_1(
            &format!(
                "DEBUG VIEW: id={} is_first_run={} has_subquery_changes={}",
                self.plan.id, is_first_run, has_subquery_changes
            )
            .into(),
        );

        let maybe_delta = if is_first_run || has_subquery_changes {
            // Force full scan if:
            // 1. First run (no cache yet)
            // 2. Records were created/deleted that might affect subquery results
            None
        } else {
            self.eval_delta_batch(&self.plan.root, deltas, db, self.params.as_ref())
        };

        let view_delta = if let Some(d) = maybe_delta {
            d
        } else {
            // FALLBACK MODE: Full Scan & Diff
            let target_set = self.eval_snapshot(&self.plan.root, db, self.params.as_ref());
            let mut diff = FastMap::default();

            for (key, &new_w) in &target_set {
                let old_w = self.cache.get(key).copied().unwrap_or(0);
                if new_w != old_w {
                    diff.insert(key.clone(), new_w - old_w);
                }
            }
            for (key, &old_w) in &self.cache {
                if !target_set.contains_key(key) {
                    diff.insert(key.clone(), 0 - old_w);
                }
            }
            diff
        };

        // Check if any record in the cache has been updated in the deltas
        // This handles UPDATE operations where the ID doesn't change but content does
        // Returns the list of updated record IDs so we can increment their versions
        let updated_record_ids = self.get_updated_cached_records(deltas);
        let has_cached_updates = !updated_record_ids.is_empty();

        #[cfg(target_arch = "wasm32")]
        web_sys::console::log_1(
            &format!(
                "DEBUG VIEW: id={} view_delta_empty={} has_cached_updates={} is_optimistic={} updated_ids_len={}",
                self.plan.id,
                view_delta.is_empty(),
                has_cached_updates,
                is_optimistic,
                updated_record_ids.len()
            )
            .into(),
        );

        if view_delta.is_empty() && !is_first_run && !has_subquery_changes && !has_cached_updates {
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

        // Build result with version tracking
        let mut result_ids: Vec<String> = self.cache.keys().map(|k| k.to_string()).collect();
        result_ids.sort_unstable();

        // Collect ALL IDs including subquery children
        let mut all_ids: Vec<String> = Vec::new();

        // Find subquery projections in the plan
        let subquery_projections = self.find_subquery_projections(&self.plan.root);

        for id in &result_ids {
            // Add main record ID
            all_ids.push(id.clone());

            // For each subquery projection, evaluate and collect matched IDs
            if !subquery_projections.is_empty() {
                if let Some(parent_row) = self.get_row_value(id, db) {
                    for subquery_op in &subquery_projections {
                        let subquery_results =
                            self.eval_snapshot(subquery_op, db, Some(parent_row));
                        for (sub_id, _weight) in subquery_results {
                            all_ids.push(sub_id.to_string());
                        }
                    }
                }
            }
        }

        // Deduplicate and sort
        all_ids.sort_unstable();
        all_ids.dedup();

        // Update version map for all IDs
        // For updated records, only increment version for optimistic updates (local mutations)
        // Remote syncs should use their own version numbers
        for id in &all_ids {
            if let Some(_current_hash) = self.get_row_hash(id, db) {
                let version = self.version_map.entry(id.clone()).or_insert(0);
                if *version == 0 {
                    *version = 1;
                } else if is_optimistic && updated_record_ids.contains(id) {
                    // Optimistic update: increment version to trigger hash change
                    let old_ver = *version;
                    *version += 1;
                    #[cfg(target_arch = "wasm32")]
                    web_sys::console::log_1(
                        &format!(
                            "DEBUG VIEW: Incrementing version for id={} old={} new={}",
                            id, old_ver, *version
                        )
                        .into(),
                    );
                }
            }
        }

        // Build flat array: [[record-id, version], ...]
        let result_data: Vec<(String, u64)> = all_ids
            .iter()
            .map(|id| {
                let version = self.version_map.get(id).copied().unwrap_or(1);
                (id.clone(), version)
            })
            .collect();

        // Hash the flat array
        let hash = compute_flat_hash(&result_data);

        if hash != self.last_hash {
            self.last_hash = hash.clone();
            return Some(MaterializedViewUpdate {
                query_id: self.plan.id.clone(),
                result_hash: hash,
                result_data,
            });
        }

        None
    }

    /// Find all Subquery projections in the operator tree
    fn find_subquery_projections(&self, op: &Operator) -> Vec<Operator> {
        let mut subqueries = Vec::new();
        self.collect_subquery_projections(op, &mut subqueries);
        subqueries
    }

    fn collect_subquery_projections(&self, op: &Operator, out: &mut Vec<Operator>) {
        match op {
            Operator::Project { input, projections } => {
                for proj in projections {
                    if let Projection::Subquery { plan, .. } = proj {
                        out.push((**plan).clone());
                        // Recursively check nested subqueries
                        self.collect_subquery_projections(plan, out);
                    }
                }
                self.collect_subquery_projections(input, out);
            }
            Operator::Filter { input, .. } => {
                self.collect_subquery_projections(input, out);
            }
            Operator::Limit { input, .. } => {
                self.collect_subquery_projections(input, out);
            }
            Operator::Join { left, right, .. } => {
                self.collect_subquery_projections(left, out);
                self.collect_subquery_projections(right, out);
            }
            Operator::Scan { .. } => {}
        }
    }

    /// Check if deltas contain changes (CREATE or DELETE) for tables used in subqueries
    /// This is needed because new/deleted records need full scan to update subquery results
    fn has_changes_for_subqueries(&self, deltas: &FastMap<String, ZSet>, _db: &Database) -> bool {
        // Get all tables used in subqueries
        let subquery_tables = self.extract_subquery_tables(&self.plan.root);

        #[cfg(target_arch = "wasm32")]
        web_sys::console::log_1(
            &format!(
                "DEBUG has_changes: view={} subquery_tables={:?} delta_tables={:?}",
                self.plan.id,
                subquery_tables,
                deltas.keys().collect::<Vec<_>>()
            )
            .into(),
        );

        if subquery_tables.is_empty() {
            #[cfg(target_arch = "wasm32")]
            web_sys::console::log_1(
                &format!(
                    "DEBUG has_changes: view={} NO SUBQUERY TABLES",
                    self.plan.id
                )
                .into(),
            );
            return false;
        }

        // Check if any delta for a subquery table contains changes (weight != 0)
        for table in subquery_tables {
            if let Some(delta) = deltas.get(&table) {
                #[cfg(target_arch = "wasm32")]
                web_sys::console::log_1(
                    &format!(
                        "DEBUG has_changes: view={} table={} delta_keys={:?}",
                        self.plan.id,
                        table,
                        delta.keys().collect::<Vec<_>>()
                    )
                    .into(),
                );
                // Check if any record in this delta is a CREATE (weight > 0 and not in version_map)
                // or a DELETE (weight < 0 and in version_map)
                for (key, weight) in delta {
                    let in_version_map = self.version_map.contains_key(key.as_str());
                    #[cfg(target_arch = "wasm32")]
                    web_sys::console::log_1(
                        &format!(
                            "DEBUG has_changes: view={} key={} weight={} in_version_map={}",
                            self.plan.id, key, weight, in_version_map
                        )
                        .into(),
                    );
                    // CREATE: positive weight, not in version_map
                    // DELETE: negative weight, in version_map
                    if (*weight > 0 && !in_version_map) || (*weight < 0 && in_version_map) {
                        #[cfg(target_arch = "wasm32")]
                        web_sys::console::log_1(
                            &format!(
                                "DEBUG has_changes: view={} FOUND CHANGE key={} weight={}",
                                self.plan.id, key, weight
                            )
                            .into(),
                        );
                        return true;
                    }
                }
            }
        }

        #[cfg(target_arch = "wasm32")]
        web_sys::console::log_1(
            &format!("DEBUG has_changes: view={} NO CHANGES FOUND", self.plan.id).into(),
        );
        false
    }

    /// Get all record IDs currently in the view's cache/version_map that have been updated in the deltas
    /// This handles UPDATE operations where the ID set doesn't change but content does
    fn get_updated_cached_records(&self, deltas: &FastMap<String, ZSet>) -> Vec<String> {
        let mut updated_ids = Vec::new();

        // For each table in the deltas, check if any updated record is in our cache
        for (_table, delta) in deltas {
            for (record_id, weight) in delta {
                // Only check records with positive weight (existing/updated records)
                // Negative weight means deletion which is handled elsewhere
                if *weight > 0 && self.cache.contains_key(record_id.as_str()) {
                    #[cfg(target_arch = "wasm32")]
                    web_sys::console::log_1(
                        &format!(
                            "DEBUG get_updated_cached_records: view={} table={} found cached record={}",
                            self.plan.id, _table, record_id
                        )
                        .into(),
                    );
                    updated_ids.push(record_id.to_string());
                }
            }
        }

        // Also check version_map for subquery records that might be affected
        for (_table, delta) in deltas {
            for (record_id, weight) in delta {
                if *weight > 0
                    && self.version_map.contains_key(record_id.as_str())
                    && !updated_ids.contains(&record_id.to_string())
                {
                    #[cfg(target_arch = "wasm32")]
                    web_sys::console::log_1(
                        &format!(
                            "DEBUG get_updated_cached_records: view={} table={} found versioned record={}",
                            self.plan.id, _table, record_id
                        )
                        .into(),
                    );
                    updated_ids.push(record_id.to_string());
                }
            }
        }

        updated_ids
    }

    /// Explicitly set the version of a record in the view
    pub fn set_record_version(
        &mut self,
        record_id: &str,
        version: u64,
        db: &Database,
    ) -> Option<MaterializedViewUpdate> {
        let current_version = self.version_map.get(record_id).copied().unwrap_or(0);

        if current_version != version {
            #[cfg(target_arch = "wasm32")]
            web_sys::console::log_1(
                &format!(
                    "DEBUG VIEW: set_record_version id={} record={} old={} new={}",
                    self.plan.id, record_id, current_version, version
                )
                .into(),
            );
            self.version_map.insert(record_id.to_string(), version);

            // Trigger re-hashing by processing empty deltas
            let empty_deltas = FastMap::default();
            // We pass is_optimistic=false because we've already manually manipulated the version map
            // and we just want to recompute the hash and return the update.
            self.process_ingest(&empty_deltas, db, false)
        } else {
            None
        }
    }

    /// Extract all table names used in subquery projections
    fn extract_subquery_tables(&self, op: &Operator) -> Vec<String> {
        let mut tables = Vec::new();
        self.collect_subquery_tables(op, &mut tables);
        tables.sort_unstable();
        tables.dedup();
        tables
    }

    fn collect_subquery_tables(&self, op: &Operator, out: &mut Vec<String>) {
        match op {
            Operator::Project { input, projections } => {
                for proj in projections {
                    if let Projection::Subquery { plan, .. } = proj {
                        // Extract tables from the subquery plan
                        self.collect_tables_from_operator(plan, out);
                    }
                }
                self.collect_subquery_tables(input, out);
            }
            Operator::Filter { input, .. } => {
                self.collect_subquery_tables(input, out);
            }
            Operator::Limit { input, .. } => {
                self.collect_subquery_tables(input, out);
            }
            Operator::Join { left, right, .. } => {
                self.collect_subquery_tables(left, out);
                self.collect_subquery_tables(right, out);
            }
            Operator::Scan { .. } => {}
        }
    }

    fn collect_tables_from_operator(&self, op: &Operator, out: &mut Vec<String>) {
        match op {
            Operator::Scan { table } => {
                out.push(table.clone());
            }
            Operator::Filter { input, .. } => {
                self.collect_tables_from_operator(input, out);
            }
            Operator::Project { input, projections } => {
                self.collect_tables_from_operator(input, out);
                for proj in projections {
                    if let Projection::Subquery { plan, .. } = proj {
                        self.collect_tables_from_operator(plan, out);
                    }
                }
            }
            Operator::Limit { input, .. } => {
                self.collect_tables_from_operator(input, out);
            }
            Operator::Join { left, right, .. } => {
                self.collect_tables_from_operator(left, out);
                self.collect_tables_from_operator(right, out);
            }
        }
    }

    /// Attempts to calculate the delta purely incrementally for a BATCH of changes.
    fn eval_delta_batch(
        &self,
        op: &Operator,
        deltas: &FastMap<String, ZSet>,
        db: &Database,
        context: Option<&SpookyValue>,
    ) -> Option<ZSet> {
        match op {
            Operator::Scan { table } => {
                // Return the delta for this table if it exists, otherwise empty
                if let Some(d) = deltas.get(table) {
                    Some(d.clone())
                } else {
                    Some(FastMap::default())
                }
            }
            Operator::Filter { input, predicate } => {
                let upstream_delta = self.eval_delta_batch(input, deltas, db, context)?;

                // SIMD Optimization Check (Copy of eval_snapshot logic)
                let simd_target_op = match predicate {
                    Predicate::Gt {
                        value: Value::Number(n),
                        ..
                    } => n.as_f64().map(|f| (f, NumericOp::Gt)),
                    Predicate::Gte {
                        value: Value::Number(n),
                        ..
                    } => n.as_f64().map(|f| (f, NumericOp::Gte)),
                    Predicate::Lt {
                        value: Value::Number(n),
                        ..
                    } => n.as_f64().map(|f| (f, NumericOp::Lt)),
                    Predicate::Lte {
                        value: Value::Number(n),
                        ..
                    } => n.as_f64().map(|f| (f, NumericOp::Lte)),
                    Predicate::Eq {
                        value: Value::Number(n),
                        ..
                    } => n.as_f64().map(|f| (f, NumericOp::Eq)),
                    Predicate::Neq {
                        value: Value::Number(n),
                        ..
                    } => n.as_f64().map(|f| (f, NumericOp::Neq)),
                    _ => None,
                };

                let field_path = match predicate {
                    Predicate::Gt { field, .. }
                    | Predicate::Gte { field, .. }
                    | Predicate::Lt { field, .. }
                    | Predicate::Lte { field, .. }
                    | Predicate::Eq { field, .. }
                    | Predicate::Neq { field, .. } => Some(field),
                    _ => None,
                };

                if let (Some((target, op)), Some(path)) = (simd_target_op, field_path) {
                    // SIMD PATH
                    let (keys, weights, numbers) = extract_number_column(&upstream_delta, path, db);
                    let passing_indices = filter_f64_batch(&numbers, target, op);

                    let mut out_delta = FastMap::default();
                    for idx in passing_indices {
                        out_delta.insert(keys[idx].clone(), weights[idx]);
                    }
                    Some(out_delta)
                } else {
                    // Slow Path
                    let mut out_delta = FastMap::default();
                    for (key, weight) in upstream_delta {
                        if self.check_predicate(predicate, &key, db, context) {
                            out_delta.insert(key, weight);
                        }
                    }
                    Some(out_delta)
                }
            }
            Operator::Project { input, .. } => self.eval_delta_batch(input, deltas, db, context),

            // Complex operators (Joins, Limits) fall back to snapshot
            Operator::Join { .. } | Operator::Limit { .. } => None,
        }
    }

    /// Deprecated: Helper wrapper for single-table delta (retained for compatibility if needed internally)
    #[allow(dead_code)]
    fn eval_delta(
        &self,
        op: &Operator,
        changed_table: &str,
        input_delta: &ZSet,
        db: &Database,
        context: Option<&SpookyValue>,
    ) -> Option<ZSet> {
        let mut deltas = FastMap::default();
        deltas.insert(changed_table.to_string(), input_delta.clone());
        self.eval_delta_batch(op, &deltas, db, context)
    }

    /// The classic detailed Full-Scan Evaluator (for fallback and init)
    fn eval_snapshot(&self, op: &Operator, db: &Database, context: Option<&SpookyValue>) -> ZSet {
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

                // SIMD Optimization Check
                let simd_target_op = match predicate {
                    Predicate::Gt {
                        value: Value::Number(n),
                        ..
                    } => n.as_f64().map(|f| (f, NumericOp::Gt)),
                    Predicate::Gte {
                        value: Value::Number(n),
                        ..
                    } => n.as_f64().map(|f| (f, NumericOp::Gte)),
                    Predicate::Lt {
                        value: Value::Number(n),
                        ..
                    } => n.as_f64().map(|f| (f, NumericOp::Lt)),
                    Predicate::Lte {
                        value: Value::Number(n),
                        ..
                    } => n.as_f64().map(|f| (f, NumericOp::Lte)),
                    Predicate::Eq {
                        value: Value::Number(n),
                        ..
                    } => n.as_f64().map(|f| (f, NumericOp::Eq)),
                    Predicate::Neq {
                        value: Value::Number(n),
                        ..
                    } => n.as_f64().map(|f| (f, NumericOp::Neq)),
                    _ => None,
                };

                let field_path = match predicate {
                    Predicate::Gt { field, .. }
                    | Predicate::Gte { field, .. }
                    | Predicate::Lt { field, .. }
                    | Predicate::Lte { field, .. }
                    | Predicate::Eq { field, .. }
                    | Predicate::Neq { field, .. } => Some(field),
                    _ => None,
                };

                if let (Some((target, op)), Some(path)) = (simd_target_op, field_path) {
                    // SIMD PATH
                    let (keys, weights, numbers) = extract_number_column(&upstream, path, db);
                    let passing_indices = filter_f64_batch(&numbers, target, op);

                    let mut out = FastMap::default();
                    for idx in passing_indices {
                        // Safety: indices returned by filter_batch are valid for keys/weights
                        out.insert(keys[idx].clone(), weights[idx]);
                    }
                    out
                } else {
                    // Slow Path (Loop)
                    let mut out = FastMap::default();
                    for (key, weight) in upstream {
                        if self.check_predicate(predicate, &key, db, context) {
                            out.insert(key, weight);
                        }
                    }
                    out
                }
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

                            let cmp = compare_spooky_values(val_a, val_b);
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
                            let hash = hash_spooky_value(r_field);
                            right_index.entry(hash).or_default().push((r_key, r_weight));
                        }
                    }
                }

                // 2. PROBE PHASE: Iterate Left and lookup Right (O(1))
                for (l_key, l_weight) in &s_left {
                    if let Some(l_val) = self.get_row_value(l_key.as_str(), db) {
                        if let Some(l_field) = resolve_nested_value(Some(l_val), &on.left_field) {
                            let hash = hash_spooky_value(l_field);

                            // Hash Lookup instead of Loop!
                            if let Some(matches) = right_index.get(&hash) {
                                for (_r_key, r_weight) in matches {
                                    // We have a match! (Should double check equality with compare_spooky_values for strictness)
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

    fn get_row_value<'a>(&self, key: &str, db: &'a Database) -> Option<&'a SpookyValue> {
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
        context: Option<&SpookyValue>,
    ) -> bool {
        // Helper to get actual SpookyValue for comparison from the Predicate (which stores Value)
        let resolve_val = |_field: &Path, value: &Value| -> Option<SpookyValue> {
            if let Some(obj) = value.as_object() {
                if let Some(param_path) = obj.get("$param") {
                    if let Some(ctx) = context {
                        // $param is usually a path like "parent.author" or just "author"
                        // Strip "parent." prefix since context IS the parent row
                        let path_str = param_path.as_str().unwrap_or("");
                        let effective_path = if path_str.starts_with("parent.") {
                            &path_str[7..] // Strip "parent." (7 chars)
                        } else {
                            path_str
                        };
                        let path = Path::new(effective_path);
                        // resolve nested param path from SpookyValue context!
                        // IMPORTANT: Normalize RecordId-like objects to strings for proper comparison
                        resolve_nested_value(Some(ctx), &path)
                            .cloned()
                            .map(normalize_record_id)
                    } else {
                        None
                    }
                } else {
                    Some(SpookyValue::from(value.clone()))
                }
            } else {
                Some(SpookyValue::from(value.clone()))
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
                        if let SpookyValue::Str(s) = val {
                            return s.starts_with(prefix);
                        }
                    }
                }
                false
            }
            Predicate::Eq { field, value }
            | Predicate::Neq { field, value }
            | Predicate::Gt { field, value }
            | Predicate::Gte { field, value }
            | Predicate::Lt { field, value }
            | Predicate::Lte { field, value } => {
                let target_val = resolve_val(field, value);
                if target_val.is_none() {
                    return false;
                }
                let target_val = target_val.unwrap();

                let actual_val_opt = if field.0.len() == 1 && field.0[0] == "id" {
                    Some(SpookyValue::Str(SmolStr::new(key)))
                } else {
                    self.get_row_value(key, db)
                        .and_then(|r| resolve_nested_value(Some(r), field).cloned())
                };

                if let Some(actual_val) = actual_val_opt {
                    let ord = compare_spooky_values(Some(&actual_val), Some(&target_val));
                    match pred {
                        Predicate::Eq { .. } => ord == Ordering::Equal,
                        Predicate::Neq { .. } => ord != Ordering::Equal,
                        Predicate::Gt { .. } => ord == Ordering::Greater,
                        Predicate::Gte { .. } => ord == Ordering::Greater || ord == Ordering::Equal,
                        Predicate::Lt { .. } => ord == Ordering::Less,
                        Predicate::Lte { .. } => ord == Ordering::Less || ord == Ordering::Equal,
                        _ => false,
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
// Optimized for SpookyValue
fn compare_spooky_values(a: Option<&SpookyValue>, b: Option<&SpookyValue>) -> Ordering {
    use std::cmp::Ordering;
    match (a, b) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (Some(va), Some(vb)) => match (va, vb) {
            (SpookyValue::Null, SpookyValue::Null) => Ordering::Equal,
            (SpookyValue::Bool(ba), SpookyValue::Bool(bb)) => ba.cmp(bb),
            (SpookyValue::Number(na), SpookyValue::Number(nb)) => {
                // Simple f64 comparison
                na.partial_cmp(&nb).unwrap_or(Ordering::Equal)
            }
            (SpookyValue::Str(sa), SpookyValue::Str(sb)) => sa.cmp(sb),
            (SpookyValue::Array(aa), SpookyValue::Array(ab)) => {
                let len_cmp = aa.len().cmp(&ab.len());
                if len_cmp != Ordering::Equal {
                    return len_cmp;
                }
                for (ia, ib) in aa.iter().zip(ab.iter()) {
                    let cmp = compare_spooky_values(Some(ia), Some(ib));
                    if cmp != Ordering::Equal {
                        return cmp;
                    }
                }
                Ordering::Equal
            }
            (SpookyValue::Object(oa), SpookyValue::Object(ob)) => oa.len().cmp(&ob.len()), // Deep compare skipped for perf
            _ => type_rank(va).cmp(&type_rank(vb)),
        },
    }
}

fn type_rank(v: &SpookyValue) -> u8 {
    match v {
        SpookyValue::Null => 0,
        SpookyValue::Bool(_) => 1,
        SpookyValue::Number(_) => 2,
        SpookyValue::Str(_) => 3,
        SpookyValue::Array(_) => 4,
        SpookyValue::Object(_) => 5,
    }
}

/// Normalize RecordId-like objects to string format.
/// Converts { tb: "table", id: "id" } or { table: "table", id: "id" } to "table:id"
fn normalize_record_id(val: SpookyValue) -> SpookyValue {
    if let SpookyValue::Object(ref map) = val {
        // Check for SurrealDB RecordId patterns: { tb, id } or { table, id }
        let table = map
            .get(&SmolStr::new("tb"))
            .or_else(|| map.get(&SmolStr::new("table")));
        let id = map.get(&SmolStr::new("id"));

        if let (Some(SpookyValue::Str(table_str)), Some(id_val)) = (table, id) {
            // Convert id to string - might be string or another type
            let id_str = match id_val {
                SpookyValue::Str(s) => s.to_string(),
                SpookyValue::Number(n) => n.to_string(),
                _ => return val, // Can't normalize, return as-is
            };
            return SpookyValue::Str(SmolStr::new(format!("{}:{}", table_str, id_str)));
        }
    }
    val
}

// Dot notation access: "address.city" -> traverses json
// Optimized specifically for Path and SpookyValue
#[inline(always)]
fn resolve_nested_value<'a>(root: Option<&'a SpookyValue>, path: &Path) -> Option<&'a SpookyValue> {
    let mut current = root;
    for part in &path.0 {
        match current {
            Some(SpookyValue::Object(map)) => {
                current = map.get(part);
            }
            _ => return None,
        }
    }
    current
}

// Fast hashing for Join Keys
#[inline(always)]
fn hash_spooky_value(v: &SpookyValue) -> u64 {
    let mut hasher = FxHasher::default();
    hash_value_recursive(v, &mut hasher);
    hasher.finish()
}

// --- 3. SIMD / COLUMNAR OPTIMIZATIONS ---

// Helper Enum for numeric predicates
enum NumericOp {
    Gt,
    Gte,
    Lt,
    Lte,
    Eq,
    Neq,
}

/*
   Attempts to extract a "Column" of f64 values from the ZSet + Database.
   Returns: (Ids, Weights, Numbers) aligned by index.
   If a value is missing or not a number, it defaults to f64::NAN which fails most comparisons safely.
*/
#[inline(always)]
fn extract_number_column(
    zset: &ZSet,
    path: &Path,
    db: &Database,
    // Optional context if needed for resolving params locally (not used for column extraction usually)
) -> (Vec<SmolStr>, Vec<i64>, Vec<f64>) {
    let mut ids = Vec::with_capacity(zset.len());
    let mut weights = Vec::with_capacity(zset.len());
    let mut numbers = Vec::with_capacity(zset.len());

    for (key, weight) in zset {
        let val_opt = if let Some((table, _)) = key.split_once(':') {
            db.tables.get(table).and_then(|t| t.rows.get(key))
        } else {
            None
        };

        let num_val = if let Some(row_val) = val_opt {
            if let Some(SpookyValue::Number(n)) = resolve_nested_value(Some(row_val), path) {
                *n
            } else {
                f64::NAN
            }
        } else {
            f64::NAN
        };

        ids.push(key.clone());
        weights.push(*weight);
        numbers.push(num_val);
    }

    (ids, weights, numbers)
}

// Auto-vectorizable batch filter
// Returns indices of passing elements
fn filter_f64_batch(values: &[f64], target: f64, op: NumericOp) -> Vec<usize> {
    let mut indices = Vec::with_capacity(values.len());

    // Explicit chunking to encourage SIMD opt by the compiler
    let chunks = values.chunks_exact(8);
    let remainder = chunks.remainder();

    let mut i = 0;
    for chunk in chunks {
        // Inner loop fixed size 8 - easier for LLVM to vectorize
        for val in chunk {
            let pass = match op {
                NumericOp::Gt => *val > target,
                NumericOp::Gte => *val >= target,
                NumericOp::Lt => *val < target,
                NumericOp::Lte => *val <= target,
                NumericOp::Eq => (*val - target).abs() < f64::EPSILON, // Float approx eq
                NumericOp::Neq => (*val - target).abs() > f64::EPSILON,
            };
            if pass {
                indices.push(i);
            }
            i += 1;
        }
    }

    for val in remainder {
        let pass = match op {
            NumericOp::Gt => *val > target,
            NumericOp::Gte => *val >= target,
            NumericOp::Lt => *val < target,
            NumericOp::Lte => *val <= target,
            NumericOp::Eq => (*val - target).abs() < f64::EPSILON,
            NumericOp::Neq => (*val - target).abs() > f64::EPSILON,
        };
        if pass {
            indices.push(i);
        }
        i += 1;
    }

    indices
}

// Portable SIMD Sum (Chunked)
#[allow(dead_code)] // Will be used in future aggregations
#[inline(always)]
pub fn sum_f64_simd(values: &[f64]) -> f64 {
    let mut sums = [0.0; 8];
    let chunks = values.chunks_exact(8);
    let remainder = chunks.remainder();

    for chunk in chunks {
        for i in 0..8 {
            sums[i] += chunk[i];
        }
    }

    let mut total: f64 = sums.iter().sum();
    for v in remainder {
        total += v;
    }
    total
}

fn hash_value_recursive(v: &SpookyValue, hasher: &mut FxHasher) {
    match v {
        SpookyValue::Null => {
            hasher.write_u8(0);
        }
        SpookyValue::Bool(b) => {
            hasher.write_u8(1);
            hasher.write_u8(*b as u8);
        }
        SpookyValue::Number(n) => {
            hasher.write_u8(2);
            hasher.write_u64(n.to_bits());
        }
        SpookyValue::Str(s) => {
            hasher.write_u8(3);
            hasher.write(s.as_bytes());
        }
        SpookyValue::Array(arr) => {
            hasher.write_u8(4);
            for item in arr {
                hash_value_recursive(item, hasher);
            }
        }
        SpookyValue::Object(obj) => {
            hasher.write_u8(5);
            // Simple iteration, no sorting for speed (as discussed in prev steps)
            for (k, v) in obj {
                hasher.write(k.as_bytes());
                hash_value_recursive(v, hasher);
            }
        }
    }
}
