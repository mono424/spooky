use super::circuit::Database;
use super::eval::{
    apply_numeric_filter, compare_spooky_values, hash_spooky_value, normalize_record_id,
    resolve_nested_value, NumericFilterConfig,
};
use super::operators::{Operator, Predicate, Projection};
use super::types::{Delta, FastMap, Path, SpookyValue, ZSet};
use super::update::{ViewResultFormat, ViewUpdate};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use smol_str::SmolStr;
use std::cmp::Ordering;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct QueryPlan {
    pub id: String,
    pub root: Operator,
}

/// Helper function for serde to skip serializing empty caches
fn is_cache_empty_or_streaming(cache: &ZSet) -> bool {
    cache.is_empty()
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct View {
    pub plan: QueryPlan,
    /// Unified cache: Source of truth for all modes (Flat, Tree, Streaming).
    /// Tracks ALL records that are part of the view (including subquery results).
    #[serde(default, skip_serializing_if = "is_cache_empty_or_streaming")]
    pub cache: ZSet,
    pub last_hash: String,
    #[serde(default)]
    pub params: Option<SpookyValue>,
    #[serde(default)]
    pub format: ViewResultFormat, // Output format strategy

    // Cached characteristics
    #[serde(skip)]
    has_subqueries_cached: bool,
    #[serde(skip)]
    referenced_tables_cached: Vec<String>,
}

impl View {
    pub fn new(plan: QueryPlan, params: Option<Value>, format: Option<ViewResultFormat>) -> Self {
        let has_subqueries_cached = plan.root.has_subquery_projections();
        let referenced_tables_cached = plan.root.referenced_tables();

        Self {
            plan,
            cache: FastMap::default(),
            last_hash: String::new(),
            params: params.map(SpookyValue::from),
            format: format.unwrap_or_default(),
            has_subqueries_cached,
            referenced_tables_cached,
        }
    }

    /// Process a delta for this view - optimized fast path for simple views
    pub fn process_delta(&mut self, delta: &Delta, db: &Database) -> Option<ViewUpdate> {
        // Fast check: Does this view even care about this table?
        // Note: circuit.rs usually handles this via dependency list, but this
        // specific check avoids work if called directly or if dependency logic is loose.
        if !self.referenced_tables_cached.iter().any(|t| t == delta.table.as_str()) {
            return None;
        }

        // Fast path: Try optimized processing for simple Scan/Filter views
        if let Some(result) = self.try_fast_single(delta, db) {
            return result;
        }
        
        // Fallback: Use batch processing for complex views
        let mut deltas = FastMap::default();
        let mut zset = ZSet::default();
        zset.insert(delta.key.clone(), delta.weight);
        deltas.insert(delta.table.to_string(), zset);
        
        self.process_batch(&deltas, db)
    }

    /// Try fast single-record processing for simple views (Scan, Filter)
    /// Returns Some(result) if fast path was taken, None if fallback needed
    fn try_fast_single(&mut self, delta: &Delta, db: &Database) -> Option<Option<ViewUpdate>> {
        match &self.plan.root {
            Operator::Scan { table } => {
                if table.as_str() != delta.table.as_str() {
                    return Some(None); // Different table, no effect
                }
                // Handle both creates and deletes
                if delta.weight > 0 {
                    Some(self.apply_single_create(&delta.key))
                } else {
                    Some(self.apply_single_delete(&delta.key))
                }
            }
            Operator::Filter { input, predicate } => {
                // Only optimize Scan+Filter
                if let Operator::Scan { table } = input.as_ref() {
                    if table.as_str() != delta.table.as_str() {
                        return Some(None); // Different table
                    }
                    // Check if record passes filter
                    if self.check_predicate(predicate, &delta.key, db, self.params.as_ref()) {
                        if delta.weight > 0 {
                            return Some(self.apply_single_create(&delta.key));
                        } else {
                            return Some(self.apply_single_delete(&delta.key));
                        }
                    } else {
                        return Some(None); // Filtered out
                    }
                }
                None // Complex filter, use batch path
            }
            _ => None // Complex query (Join, Project, Limit), use batch path
        }
    }

    /// Apply a single record creation to the view (fast path)
    fn apply_single_create(&mut self, key: &SmolStr) -> Option<ViewUpdate> {
        let is_first_run = self.last_hash.is_empty();
        let was_cached = self.cache.contains_key(key);
        
        // Update cache
        *self.cache.entry(key.clone()).or_insert(0) += 1;
        
        // Build result data
        let result_data = self.build_result_data();
        
        // Determine change type
        let (additions, updates) = if was_cached {
            (vec![], vec![Self::strip_table_prefix_smol(key)])
        } else {
            (vec![Self::strip_table_prefix_smol(key)], vec![])
        };
        
        // Build update
        use super::update::{build_update, compute_flat_hash, RawViewResult, ViewDelta};
        
        let view_delta_struct = if is_first_run {
            None
        } else {
            Some(ViewDelta {
                additions,
                removals: vec![],
                updates,
            })
        };
        
        // Compute hash if needed (for Streaming) before moving result_data
        let pre_hash = if matches!(self.format, ViewResultFormat::Streaming) {
            Some(compute_flat_hash(&result_data))
        } else {
            None
        };
        
        let raw_result = RawViewResult {
            query_id: self.plan.id.clone(),
            records: result_data,
            delta: view_delta_struct,
        };
        
        let update = build_update(raw_result, self.format.clone());
        
        // Hash check
        let hash = match &update {
            ViewUpdate::Flat(flat) | ViewUpdate::Tree(flat) => flat.result_hash.clone(),
            ViewUpdate::Streaming(_) => pre_hash.unwrap_or_default(),
        };
        
        let has_changes = match &update {
            ViewUpdate::Streaming(s) => !s.records.is_empty(),
            _ => hash != self.last_hash,
        };
        
        if has_changes {
            self.last_hash = hash;
            Some(update)
        } else {
            None
        }
    }

    /// Apply a single record deletion to the view (fast path)
    fn apply_single_delete(&mut self, key: &SmolStr) -> Option<ViewUpdate> {
        let is_first_run = self.last_hash.is_empty();
        
        // Check if key exists in cache
        if !self.cache.contains_key(key) {
            return None; // Not in view, no change
        }
        
        // Remove from cache
        self.cache.remove(key);
        
        // Build result data
        let result_data = self.build_result_data();
        
        // Build update
        use super::update::{build_update, compute_flat_hash, RawViewResult, ViewDelta};
        
        let view_delta_struct = if is_first_run {
            None
        } else {
            Some(ViewDelta {
                additions: vec![],
                removals: vec![Self::strip_table_prefix_smol(key)],
                updates: vec![],
            })
        };
        
        // Compute hash if needed (for Streaming) before moving result_data
        let pre_hash = if matches!(self.format, ViewResultFormat::Streaming) {
            Some(compute_flat_hash(&result_data))
        } else {
            None
        };
        
        let raw_result = RawViewResult {
            query_id: self.plan.id.clone(),
            records: result_data,
            delta: view_delta_struct,
        };
        
        let update = build_update(raw_result, self.format.clone());
        
        // Hash check
        let hash = match &update {
            ViewUpdate::Flat(flat) | ViewUpdate::Tree(flat) => flat.result_hash.clone(),
            ViewUpdate::Streaming(_) => pre_hash.unwrap_or_default(),
        };
        
        let has_changes = match &update {
            ViewUpdate::Streaming(s) => !s.records.is_empty(),
            _ => hash != self.last_hash,
        };
        
        if has_changes {
            self.last_hash = hash;
            Some(update)
        } else {
            None
        }
    }

    /// Check if this view has any subquery projections
    fn has_subqueries(&self) -> bool {
        self.has_subqueries_cached
    }



    /// Optimized 2-Phase Processing: Handles multiple table updates at once.
    pub fn process_batch(
        &mut self,
        deltas: &FastMap<String, ZSet>,
        db: &Database,
    ) -> Option<ViewUpdate> {
        // FIX: FIRST RUN CHECK
        let is_first_run = self.last_hash.is_empty();

        // Compute view delta
        let view_delta = self.compute_view_delta(deltas, db, is_first_run);
        let updated_record_ids = self.get_updated_cached_records(deltas);
        
        // Early return if no changes
        if view_delta.is_empty() && !is_first_run && updated_record_ids.is_empty() {
            return None;
        }

        // Apply delta to cache
        self.apply_cache_delta(&view_delta);

        // Categorize changes
        let (additions, removals, updates) = self.categorize_changes(&view_delta, &updated_record_ids);

        // Build result data
        let result_data = self.build_result_data();

        // Delegate formatting to update module (Strategy Pattern)
        use super::update::{build_update, compute_flat_hash, RawViewResult, ViewDelta};

        let view_delta_struct = if is_first_run {
            None // First run = treat as full snapshot
        } else {
            Some(ViewDelta {
                additions,
                removals,
                updates,
            })
        };

        // Compute hash if needed (for Streaming) before moving result_data
        let pre_hash = if matches!(self.format, ViewResultFormat::Streaming) {
            Some(compute_flat_hash(&result_data))
        } else {
            None
        };

        let raw_result = RawViewResult {
            query_id: self.plan.id.clone(),
            records: result_data,
            delta: view_delta_struct,
        };

        // Build update using the configured format
        let update = build_update(raw_result, self.format.clone());

        // Extract hash for comparison (depends on format)
        let hash = match &update {
            ViewUpdate::Flat(flat) | ViewUpdate::Tree(flat) => flat.result_hash.clone(),
            ViewUpdate::Streaming(_) => pre_hash.unwrap_or_default(),
        };

        let has_changes = match &update {
            ViewUpdate::Streaming(s) => !s.records.is_empty(),
            _ => hash != self.last_hash,
        };

        if has_changes {
            self.last_hash = hash;
            return Some(update);
        }

        None
    }

    /// Helper to expand a ZSet of root records to include all their subquery dependencies
    fn expand_with_subqueries(&self, zset: &mut ZSet, db: &Database) {
        // Early exit if query has no subqueries
        if !self.has_subqueries() {
            return;
        }
        
        // We must iterate a copy of keys to safely mutate zset
        let keys: Vec<(SmolStr, i64)> = zset.iter().map(|(k, v)| (k.clone(), *v)).collect();

        for (key, weight) in keys {
            // If record is present (weight > 0), find its children
            if weight > 0 {
                if let Some(row) = self.get_row_value(&key, db) {
                    let mut sub_ids = Vec::new();
                    // recursively collect all sub-ids for this parent row
                    self.collect_subquery_ids(&self.plan.root, row, db, &mut sub_ids);
                    
                    for sub_id in sub_ids {
                        // Add sub-id with same weight (ref counting)
                        *zset.entry(SmolStr::new(sub_id)).or_insert(0) += weight;
                    }
                }
            }
        }
    }



    /// Recursively collect all record IDs from subqueries.
    /// Handles nested subqueries by using the correct parent context at each level.
    fn collect_subquery_ids(
        &self,
        op: &Operator,
        parent_row: &SpookyValue,
        db: &Database,
        out: &mut Vec<String>,
    ) {
        match op {
            Operator::Project { input, projections } => {
                // First, recurse into the input operator
                self.collect_subquery_ids(input, parent_row, db, out);
                
                // Then process each subquery projection at this level
                for proj in projections {
                    if let Projection::Subquery { plan, .. } = proj {
                        // Evaluate this subquery with the current parent context
                        let subquery_results = self
                            .eval_snapshot(plan, db, Some(parent_row))
                            .into_owned();
                        
                        // For each result, add the ID and recursively process nested subqueries
                        for (sub_id, _weight) in &subquery_results {
                            out.push(sub_id.to_string());
                            
                            // Get the actual row data for this subquery result
                            // to use as parent context for any nested subqueries WITHIN this subquery
                            if let Some(sub_row) = self.get_row_value(sub_id.as_str(), db) {
                                // Recurse into the subquery's plan to find nested subqueries
                                // This handles cases like: comments -> comment.author, comments -> comment.replies -> reply.author
                                self.collect_subquery_ids(plan, sub_row, db, out);
                            }
                        }
                    }
                }
            }
            Operator::Filter { input, .. } => {
                self.collect_subquery_ids(input, parent_row, db, out);
            }
            Operator::Limit { input, .. } => {
                self.collect_subquery_ids(input, parent_row, db, out);
            }
            Operator::Join { left, right, .. } => {
                self.collect_subquery_ids(left, parent_row, db, out);
                self.collect_subquery_ids(right, parent_row, db, out);
            }
            Operator::Scan { .. } => {
                // Base case: no subqueries in a scan
            }
        }
    }



    /// Compute the view delta using incremental or full-scan approach
    fn compute_view_delta(
        &mut self,
        deltas: &FastMap<String, ZSet>,
        db: &Database,
        is_first_run: bool,
    ) -> ZSet {
        if is_first_run {
            // First run: full scan and diff
            self.compute_full_diff(db)
        } else {
            // Try incremental evaluation first
            if let Some(delta) = self.eval_delta_batch(&self.plan.root, deltas, db, self.params.as_ref()) {
                delta
            } else {
                // Fallback to full scan
                self.compute_full_diff(db)
            }
        }
    }

    /// Compute full diff between current cache and target state
    fn compute_full_diff(&mut self, db: &Database) -> ZSet {
        let mut target_set = self
            .eval_snapshot(&self.plan.root, db, self.params.as_ref())
            .into_owned();
        
        // Expand target set to include implicitly included subquery records
        self.expand_with_subqueries(&mut target_set, db);

        let mut diff = FastMap::default();

        // Compute diff: new weights - old weights
        for (key, &new_w) in &target_set {
            let old_w = self.cache.get(key).copied().unwrap_or(0);
            if new_w != old_w {
                diff.insert(key.clone(), new_w - old_w);
            }
        }
        // Records that were removed
        for (key, &old_w) in &self.cache {
            if !target_set.contains_key(key) {
                diff.insert(key.clone(), 0 - old_w);
            }
        }
        diff
    }

    /// Apply delta to cache, updating weights
    fn apply_cache_delta(&mut self, delta: &ZSet) {
        for (key, weight) in delta {
            let entry = self.cache.entry(key.clone()).or_insert(0);
            *entry += weight;
            if *entry == 0 {
                self.cache.remove(key);
            }
        }
    }

    /// Categorize changes into additions, removals, and updates
    fn categorize_changes(
        &self,
        view_delta: &ZSet,
        updated_record_ids: &[SmolStr],
    ) -> (Vec<SmolStr>, Vec<SmolStr>, Vec<SmolStr>) {
        let delta_size = view_delta.len();
        let mut additions: Vec<SmolStr> = Vec::with_capacity(delta_size);
        let mut removals: Vec<SmolStr> = Vec::with_capacity(delta_size);

        // Build set of updated IDs for O(1) lookup
        // Optimization: Use linear search for small sets
        let use_hashset = updated_record_ids.len() > 8;
        let updated_ids_set: Option<std::collections::HashSet<&str>> = if use_hashset {
            Some(updated_record_ids.iter().map(|s| s.as_str()).collect())
        } else {
            None
        };

        // Categorize additions and removals
        for (key, weight) in view_delta {
            if *weight > 0 {
                // Check if this is genuinely new or an update
                let is_update = if let Some(set) = &updated_ids_set {
                    set.contains(key.as_str())
                } else {
                    updated_record_ids.iter().any(|id| id == key.as_str())
                };

                if !is_update {
                    additions.push(Self::strip_table_prefix_smol(key));
                }
            } else if *weight < 0 {
                removals.push(Self::strip_table_prefix_smol(key));
            }
        }

        // Build removal set for filtering updates
        let removal_set_unstripped: std::collections::HashSet<&str> = 
            view_delta.iter()
                .filter(|(_, w)| **w < 0)
                .map(|(k, _)| k.as_str())
                .collect();

        // Updates: records in updated_record_ids that are NOT being removed
        let updates: Vec<SmolStr> = updated_record_ids
            .iter()
            .filter(|id| !removal_set_unstripped.contains(id.as_str()))
            .map(|id| Self::strip_table_prefix_smol(id))
            .collect();

        (additions, removals, updates)
    }

    /// Build sorted result data from current cache
    fn build_result_data(&self) -> Vec<SmolStr> {
        let mut result_data: Vec<SmolStr> = self.cache.keys()
            .map(|k| {
                k.split_once(':')
                 .map(|(_, id)| SmolStr::new(id))
                 .unwrap_or_else(|| k.clone())
            })
            .collect();
        result_data.sort_unstable();
        result_data
    }

    /// Get all record IDs currently in the view's cache/version_map that have been updated in the deltas
    /// This handles UPDATE operations where the ID set doesn't change but content does
    fn get_updated_cached_records(&self, deltas: &FastMap<String, ZSet>) -> Vec<SmolStr> {
        let mut updated_ids = Vec::new();

        // For each table in the deltas, check if any updated record is in our cache
        for (_table, delta) in deltas {
            for (record_id, weight) in delta {
                // Only check records with positive weight (existing/updated records)
                // Negative weight means deletion which is handled elsewhere
                if *weight > 0 && self.cache.contains_key(record_id.as_str()) {
                    updated_ids.push(record_id.clone());
                }
            }
        }

        updated_ids
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
            Operator::Project { input, projections } => {
                // If any projection is a subquery, we cannot safely compute delta incrementally
                // without knowing dependencies. Fallback to full snapshot.
                for proj in projections {
                    if let Projection::Subquery { .. } = proj {
                        return None;
                    }
                }
                
                self.eval_delta_batch(input, deltas, db, context)
            },

            // Complex operators (Joins, Limits) fall back to snapshot
            Operator::Join { .. } | Operator::Limit { .. } => None,
        }
    }



    /// The classic detailed Full-Scan Evaluator (for fallback and init)
    /// Returns Cow to avoid cloning ZSets when possible
    fn eval_snapshot<'a>(
        &self,
        op: &Operator,
        db: &'a Database,
        context: Option<&SpookyValue>,
    ) -> std::borrow::Cow<'a, ZSet> {
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
        // We assume valid format "table:id" (ZSet Key) -> "id" (Row Key)
        let (table_name, id) = key.split_once(':')?;
        db.tables.get(table_name)?.rows.get(id)
    }



    /// Strip "table:" prefix from ZSet key to get row ID (SmolStr version)
    #[inline]
    fn strip_table_prefix_smol(key: &str) -> SmolStr {
        key.split_once(':').map(|(_, id)| SmolStr::new(id)).unwrap_or_else(|| SmolStr::new(key))
    }

    /// Resolve predicate value, handling $param references to context
    fn resolve_predicate_value(
        value: &Value,
        context: Option<&SpookyValue>,
    ) -> Option<SpookyValue> {
        if let Some(obj) = value.as_object() {
            if let Some(param_path) = obj.get("$param") {
                let ctx = context?;
                let path_str = param_path.as_str().unwrap_or("");
                let effective_path = if path_str.starts_with("parent.") {
                    &path_str[7..] // Strip "parent." prefix
                } else {
                    path_str
                };
                let path = Path::new(effective_path);
                resolve_nested_value(Some(ctx), &path)
                    .cloned()
                    .map(normalize_record_id)
            } else {
                Some(SpookyValue::from(value.clone()))
            }
        } else {
            Some(SpookyValue::from(value.clone()))
        }
    }




    fn check_predicate(
        &self,
        pred: &Predicate,
        key: &str,
        db: &Database,
        context: Option<&SpookyValue>,
    ) -> bool {
        // Helper to get actual SpookyValue for comparison from the Predicate (which stores Value)


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
                let target_val = Self::resolve_predicate_value(value, context);
                if target_val.is_none() {
                    return false;
                }
                let target_val = target_val.unwrap();

                let actual_val_opt = if field.0.len() == 1 && field.0[0] == "id" {
                    // Match against RowKey (stripped), not ZSetKey
                    let row_key = key.split_once(':').map(|(_, id)| id).unwrap_or(key);
                    Some(SpookyValue::Str(SmolStr::new(row_key)))
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