use crate::debug_log;
use super::circuit::Database;
use super::eval::{
    apply_numeric_filter, compare_spooky_values, hash_spooky_value, NumericFilterConfig,
    normalize_record_id, resolve_nested_value,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use smol_str::SmolStr;
use std::cmp::Ordering;

// Re-export types for backward compatibility
pub use super::operators::{JoinCondition, Operator, OrderSpec, Predicate, Projection};
pub use super::types::{FastMap, Path, RowKey, SpookyValue, VersionMap, Weight, ZSet};
pub use super::update::{MaterializedViewUpdate, ViewResultFormat, ViewUpdate};

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
    #[serde(default)]
    pub params: Option<SpookyValue>,
    #[serde(default)]
    pub version_map: VersionMap, // Track versions for each record
    #[serde(default)]
    pub format: ViewResultFormat, // Output format strategy
}

impl View {
    pub fn new(plan: QueryPlan, params: Option<Value>, format: Option<ViewResultFormat>) -> Self {
        Self {
            plan,
            cache: FastMap::default(),
            last_hash: String::new(),
            params: params.map(SpookyValue::from),
            version_map: FastMap::default(),
            format: format.unwrap_or_default(),
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
    ) -> Option<ViewUpdate> {
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
    ) -> Option<ViewUpdate> {
        // FIX: FIRST RUN CHECK
        let is_first_run = self.last_hash.is_empty();

        // Check if any delta contains CREATE or DELETE operations for tables used in subqueries
        let has_subquery_changes = !is_first_run && self.has_changes_for_subqueries(deltas, db);

        debug_log!("DEBUG VIEW: id={} is_first_run={} has_subquery_changes={}", self.plan.id, is_first_run, has_subquery_changes);

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
            let target_set = self.eval_snapshot(&self.plan.root, db, self.params.as_ref()).into_owned();
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

        debug_log!("DEBUG VIEW: id={} view_delta_empty={} has_cached_updates={} is_optimistic={} updated_ids_len={}", self.plan.id, view_delta.is_empty(), has_cached_updates, is_optimistic, updated_record_ids.len());

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


        // CAPTURE DELTA SETS (needed for all formats)
        // Additions: records with positive weight
        let mut additions: Vec<(String, u64)> = Vec::new();
        // Removals: records with negative weight
        let mut removals: Vec<String> = Vec::new();

        for (key, weight) in &view_delta {
            if *weight > 0 {
                // Addition: will get version after version_map update
                additions.push((key.to_string(), 0)); // version TBD
            } else if *weight < 0 {
                // Removal
                removals.push(key.to_string());
            }
        }

        // Updates: records in updated_record_ids that are NOT new additions
        let addition_ids: std::collections::HashSet<&str> =
            additions.iter().map(|(id, _)| id.as_str()).collect();
        let updates: Vec<String> = updated_record_ids
            .iter()
            .filter(|id| !addition_ids.contains(id.as_str()))
            .cloned()
            .collect();

        // OPTIMIZATION: For Streaming mode, skip expensive full snapshot operations
        // We only need to track versions for records in the delta
        if matches!(self.format, ViewResultFormat::Streaming) {
            // Only update versions for records that changed
            for (id, _) in &view_delta {
                if let Some(_current_hash) = self.get_row_hash(id.as_str(), db) {
                    let id_key = SmolStr::new(id.as_str());
                    let version = self.version_map.entry(id_key).or_insert(0);
                    if *version == 0 {
                        *version = 1;
                    } else if is_optimistic && updated_record_ids.contains(&id.to_string()) {
                        let _old_ver = *version;
                        *version += 1;
                        debug_log!("DEBUG VIEW: Incrementing version for id={} old={} new={}", id, _old_ver, *version);
                    }
                }
            }

            // Finalize delta sets with versions
            let additions_with_versions: Vec<(String, u64)> = additions
                .iter()
                .map(|(id, _)| {
                    let version = self.version_map.get(id.as_str()).copied().unwrap_or(1);
                    (id.clone(), version)
                })
                .collect();

            let updates_with_versions: Vec<(String, u64)> = updates
                .iter()
                .map(|id| {
                    let version = self.version_map.get(id.as_str()).copied().unwrap_or(1);
                    (id.clone(), version)
                })
                .collect();

            // Build streaming update directly, no need for result_data or hash
            use super::update::{DeltaEvent, DeltaRecord, StreamingUpdate, ViewUpdate};

            let mut delta_records = Vec::new();

            if is_first_run {
                // First run: treat all cache entries as Created
                for (id, _) in &self.cache {
                    let version = self.version_map.get(id.as_str()).copied().unwrap_or(1);
                    delta_records.push(DeltaRecord {
                        id: id.to_string(),
                        event: DeltaEvent::Created,
                        version,
                    });
                }
            } else {
                // Map additions → Created
                for (id, version) in additions_with_versions {
                    delta_records.push(DeltaRecord {
                        id,
                        event: DeltaEvent::Created,
                        version,
                    });
                }

                // Map removals → Deleted
                for id in removals {
                    delta_records.push(DeltaRecord {
                        id,
                        event: DeltaEvent::Deleted,
                        version: 0,
                    });
                }

                // Map updates → Updated
                for (id, version) in updates_with_versions {
                    delta_records.push(DeltaRecord {
                        id,
                        event: DeltaEvent::Updated,
                        version,
                    });
                }
            }

            // No hash computation needed for streaming—track by version numbers
            self.last_hash = "streaming".to_string();

            return Some(ViewUpdate::Streaming(StreamingUpdate {
                view_id: self.plan.id.clone(),
                records: delta_records,
            }));
        }

        // FALLBACK: For Flat/Tree modes, build full snapshot
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
                            self.eval_snapshot(subquery_op, db, Some(parent_row)).into_owned();
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
                let id_key = SmolStr::new(id);
                let version = self.version_map.entry(id_key).or_insert(0);
                if *version == 0 {
                    *version = 1;
                } else if is_optimistic && updated_record_ids.contains(id) {
                    // Optimistic update: increment version to trigger hash change
                    let _old_ver = *version;
                    *version += 1;
                    debug_log!("DEBUG VIEW: Incrementing version for id={} old={} new={}", id, _old_ver, *version);
                }
            }
        }

        // Finalize delta sets with versions
        let additions_with_versions: Vec<(String, u64)> = additions
            .iter()
            .map(|(id, _)| {
                let version = self.version_map.get(id.as_str()).copied().unwrap_or(1);
                (id.clone(), version)
            })
            .collect();

        let updates_with_versions: Vec<(String, u64)> = updates
            .iter()
            .map(|id| {
                let version = self.version_map.get(id.as_str()).copied().unwrap_or(1);
                (id.clone(), version)
            })
            .collect();

        // Build raw result data (format-agnostic)
        let result_data: Vec<(String, u64)> = all_ids
            .iter()
            .map(|id| {
                let version = self.version_map.get(id.as_str()).copied().unwrap_or(1);
                (id.clone(), version)
            })
            .collect();

        // Delegate formatting to update module (Strategy Pattern)
        use super::update::{build_update, compute_flat_hash, RawViewResult, ViewDelta};

        let view_delta_struct = if is_first_run {
            None // First run = treat as full snapshot
        } else {
            Some(ViewDelta {
                additions: additions_with_versions,
                removals,
                updates: updates_with_versions,
            })
        };

        let raw_result = RawViewResult {
            query_id: self.plan.id.clone(),
            records: result_data.clone(),
            delta: view_delta_struct,
        };

        // Build update using the configured format
        let update = build_update(raw_result, self.format.clone());

        // Extract hash for comparison (depends on format)
        let hash = match &update {
            ViewUpdate::Flat(flat) | ViewUpdate::Tree(flat) => flat.result_hash.clone(),
            ViewUpdate::Streaming(_) => compute_flat_hash(&result_data),
        };

        if hash != self.last_hash {
            self.last_hash = hash;
            return Some(update);
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

        debug_log!("DEBUG has_changes: view={} subquery_tables={:?} delta_tables={:?}", self.plan.id, subquery_tables, deltas.keys().collect::<Vec<_>>());

        if subquery_tables.is_empty() {
            debug_log!("DEBUG has_changes: view={} NO SUBQUERY TABLES", self.plan.id);
            return false;
        }

        // Check if any delta for a subquery table contains changes (weight != 0)
        for table in subquery_tables {
            if let Some(delta) = deltas.get(&table) {
                debug_log!("DEBUG has_changes: view={} table={} delta_keys={:?}", self.plan.id, table, delta.keys().collect::<Vec<_>>());
                // Check if any record in this delta is a CREATE (weight > 0 and not in version_map)
                // or a DELETE (weight < 0 and in version_map)
                for (key, weight) in delta {
                    let in_version_map = self.version_map.contains_key(key.as_str());
                    debug_log!("DEBUG has_changes: view={} key={} weight={} in_version_map={}", self.plan.id, key, weight, in_version_map);
                    // CREATE: positive weight, not in version_map
                    // DELETE: negative weight, in version_map
                    if (*weight > 0 && !in_version_map) || (*weight < 0 && in_version_map) {
                        debug_log!("DEBUG has_changes: view={} FOUND CHANGE key={} weight={}", self.plan.id, key, weight);
                        return true;
                    }
                }
            }
        }

        debug_log!("DEBUG has_changes: view={} NO CHANGES FOUND", self.plan.id);
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
                    debug_log!("DEBUG get_updated_cached_records: view={} table={} found cached record={}", self.plan.id, _table, record_id);
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
                    debug_log!("DEBUG get_updated_cached_records: view={} table={} found versioned record={}", self.plan.id, _table, record_id);
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
    ) -> Option<ViewUpdate> {
        let current_version = self.version_map.get(record_id).copied().unwrap_or(0);

        if current_version != version {
            debug_log!("DEBUG VIEW: set_record_version id={} record={} old={} new={}", self.plan.id, record_id, current_version, version);
            self.version_map.insert(SmolStr::new(record_id), version);

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

                // Try SIMD fast path using NumericFilterConfig
                if let Some(config) = NumericFilterConfig::from_predicate(predicate) {
                    Some(apply_numeric_filter(&upstream_delta, &config, db))
                } else {
                    // Slow Path (non-numeric predicates)
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
    /// Returns Cow to avoid cloning ZSets when possible
    fn eval_snapshot<'a>(&self, op: &Operator, db: &'a Database, context: Option<&SpookyValue>) -> std::borrow::Cow<'a, ZSet> {
        use std::borrow::Cow;
        
        match op {
            Operator::Scan { table } => {
                if let Some(tb) = db.tables.get(table) {
                    // Zero-copy borrow for scan operations
                    Cow::Borrowed(&tb.zset)
                } else {
                    Cow::Owned(FastMap::default())
                }
            }
            Operator::Filter { input, predicate } => {
                let upstream = self.eval_snapshot(input, db, context);

                // Try SIMD fast path using NumericFilterConfig
                if let Some(config) = NumericFilterConfig::from_predicate(predicate) {
                    // apply_numeric_filter takes &ZSet, so we can pass borrowed ref
                    Cow::Owned(apply_numeric_filter(upstream.as_ref(), &config, db))
                } else {
                    // Slow Path (non-numeric predicates)
                    let mut out = FastMap::default();
                    for (key, weight) in upstream.as_ref() {
                        if self.check_predicate(predicate, key, db, context) {
                            out.insert(key.clone(), *weight);
                        }
                    }
                    Cow::Owned(out)
                }
            }
            Operator::Project { input, .. } => self.eval_snapshot(input, db, context),
            Operator::Limit {
                input,
                limit,
                order_by,
            } => {
                let upstream = self.eval_snapshot(input, db, context);
                let mut items: Vec<_> = upstream.iter().map(|(k, v)| (k, v)).collect();

                if let Some(orders) = order_by {
                    items.sort_by(|a, b| {
                        let row_a = self.get_row_value(a.0.as_str(), db);
                        let row_b = self.get_row_value(b.0.as_str(), db);

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
                        a.0.cmp(b.0)
                    });
                } else {
                    items.sort_unstable_by(|a, b| a.0.cmp(b.0));
                }

                let mut out = FastMap::default();
                for (i, (key, weight)) in items.into_iter().enumerate() {
                    if i < *limit {
                        out.insert(key.clone(), *weight);
                    } else {
                        break;
                    }
                }
                Cow::Owned(out)
            }
            Operator::Join { left, right, on } => {
                let s_left = self.eval_snapshot(left, db, context);
                let s_right = self.eval_snapshot(right, db, context);
                let mut out = FastMap::default();

                // 1. BUILD PHASE: Build Index for the RIGHT side
                // Map: Hash of Join-Field -> List of (Key, Weight)
                let mut right_index: FastMap<u64, Vec<(&SmolStr, &i64)>> = FastMap::default();

                for (r_key, r_weight) in s_right.as_ref() {
                    if let Some(r_val) = self.get_row_value(r_key.as_str(), db) {
                        if let Some(r_field) = resolve_nested_value(Some(r_val), &on.right_field) {
                            let hash = hash_spooky_value(r_field);
                            right_index.entry(hash).or_default().push((r_key, r_weight));
                        }
                    }
                }

                // 2. PROBE PHASE: Iterate Left and lookup Right (O(1))
                for (l_key, l_weight) in s_left.as_ref() {
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
                Cow::Owned(out)
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
