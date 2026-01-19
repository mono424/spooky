use super::circuit::Database;
use super::eval::{
    apply_numeric_filter, compare_spooky_values, hash_spooky_value, normalize_record_id,
    resolve_nested_value, NumericFilterConfig,
};
use crate::debug_log;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use smol_str::SmolStr;
use std::cmp::Ordering;

// Re-export types for backward compatibility
pub use super::operators::{JoinCondition, Operator, OrderSpec, Predicate, Projection};
pub use super::types::{FastMap, Path, RowKey, SpookyValue, VersionMap, Weight, ZSet};
pub use super::update::{
    DeltaEvent, DeltaRecord, MaterializedViewUpdate, StreamingUpdate, ViewResultFormat, ViewUpdate,
};

/// Context for view processing - computed once, used throughout
struct ProcessContext {
    is_first_run: bool,
    is_streaming: bool,
    has_subquery_changes: bool,
}

impl ProcessContext {
    fn new(view: &View, deltas: &FastMap<String, ZSet>, db: &Database) -> Self {
        let is_first_run = view.last_hash.is_empty();
        Self {
            is_first_run,
            is_streaming: matches!(view.format, ViewResultFormat::Streaming),
            has_subquery_changes: !is_first_run && view.has_changes_for_subqueries(deltas, db),
        }
    }

    fn should_full_scan(&self) -> bool {
        self.is_first_run || self.has_subquery_changes
    }
}

/// Categorized changes from view delta
struct DeltaCategories {
    additions: Vec<String>,
    removals: Vec<String>,
    updates: Vec<String>,
}

impl DeltaCategories {
    fn with_capacity(cap: usize) -> Self {
        Self {
            additions: Vec::with_capacity(cap),
            removals: Vec::with_capacity(cap),
            updates: Vec::with_capacity(cap),
        }
    }

    fn is_empty(&self) -> bool {
        self.additions.is_empty() && self.removals.is_empty() && self.updates.is_empty()
    }
}

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
    /// Cache is only used for Flat/Tree modes. For Streaming mode, version_map is the source of truth.
    /// Skip serializing if empty (streaming mode keeps it empty).
    #[serde(default, skip_serializing_if = "is_cache_empty_or_streaming")]
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
        let ctx = ProcessContext::new(self, deltas, db);

        debug_log!(
            "DEBUG VIEW: id={} is_first_run={} has_subquery_changes={} is_streaming={}",
            self.plan.id,
            ctx.is_first_run,
            ctx.has_subquery_changes,
            ctx.is_streaming
        );

        // Step 1: Compute view delta
        let view_delta = self.compute_view_delta(&ctx, deltas, db)?;

        // Step 2: Identify updated records (records in view that changed)
        let updated_record_ids = self.identify_updated_records(&ctx, deltas);

        // Step 3: Categorize changes
        let categories = self.categorize_changes(&view_delta, &updated_record_ids);

        // Step 4: Early exit if no meaningful changes
        if categories.is_empty() && !ctx.is_first_run && !ctx.has_subquery_changes {
            return None;
        }

        // Step 5: Emit update based on format
        // Step 5: Emit update based on format
        // Step 5: Emit update based on format
        match ctx.is_streaming {
            true => self.emit_streaming_update(&ctx, &categories, db, is_optimistic),
            false => self.emit_materialized_update(&ctx, &view_delta, &categories, db, is_optimistic),
        }
    }

    fn compute_view_delta(
        &self,
        ctx: &ProcessContext,
        deltas: &FastMap<String, ZSet>,
        db: &Database,
    ) -> Option<ZSet> {
        if ctx.should_full_scan() {
            // Full scan strategy
            return Some(self.compute_snapshot_delta(ctx, db));
        }

        // Try incremental strategy
        if let Some(delta) = self.eval_delta_batch(&self.plan.root, deltas, db, self.params.as_ref()) {
            return Some(delta);
        }

        // Fallback to full scan strategy if incremental not supported for operator
        Some(self.compute_snapshot_delta(ctx, db))
    }

    fn compute_snapshot_delta(&self, ctx: &ProcessContext, db: &Database) -> ZSet {
        let target_set = self
            .eval_snapshot(&self.plan.root, db, self.params.as_ref())
            .into_owned();
        self.diff_against_cache(&target_set, ctx.is_streaming)
    }

    fn diff_against_cache(&self, target_set: &ZSet, is_streaming: bool) -> ZSet {
        let mut diff = FastMap::default();

        if is_streaming {
            // Streaming mode: compare against version_map
            for (key, &new_w) in target_set {
                if new_w > 0 && !self.version_map.contains_key(key.as_str()) {
                    diff.insert(key.clone(), 1);
                }
            }
            // Check for removals (only non-subquery records)
            let subquery_tables: std::collections::HashSet<String> = 
                self.extract_subquery_tables(&self.plan.root).into_iter().collect();
            
            for key in self.version_map.keys() {
                if !target_set.contains_key(key.as_str()) {
                    if let Some((table_name, _)) = key.split_once(':') {
                        if !subquery_tables.contains(table_name) {
                            diff.insert(SmolStr::new(key), -1);
                        }
                    }
                }
            }
        } else {
            // Flat/Tree mode: use cache
            for (key, &new_w) in target_set {
                let old_w = self.cache.get(key).copied().unwrap_or(0);
                if new_w != old_w {
                    diff.insert(key.clone(), new_w - old_w);
                }
            }
            for (key, &old_w) in &self.cache {
                if !target_set.contains_key(key) {
                    diff.insert(key.clone(), -old_w);
                }
            }
        }

        diff
    }

    fn identify_updated_records(
        &self,
        ctx: &ProcessContext,
        deltas: &FastMap<String, ZSet>,
    ) -> Vec<String> {
        if ctx.is_streaming {
            self.get_updated_records_streaming(deltas)
        } else {
            self.get_updated_cached_records(deltas)
        }
    }

    fn categorize_changes(
        &self,
        view_delta: &ZSet,
        updated_record_ids: &[String],
    ) -> DeltaCategories {
        let mut categories = DeltaCategories::with_capacity(view_delta.len());

        // Build lookup set for O(1) update checks
        let updated_set: std::collections::HashSet<&str> = 
            updated_record_ids.iter().map(|s| s.as_str()).collect();

        for (key, weight) in view_delta {
            if *weight > 0 {
                // Positive weight: addition (unless it's an update)
                if !updated_set.contains(key.as_str()) {
                    categories.additions.push(key.to_string());
                }
            } else if *weight < 0 {
                // Negative weight: removal
                categories.removals.push(key.to_string());
            }
        }

        // Updates: records that changed but weren't removed
        let removal_set: std::collections::HashSet<&str> = 
            categories.removals.iter().map(|s| s.as_str()).collect();
        
        for id in updated_record_ids {
            if !removal_set.contains(id.as_str()) {
                categories.updates.push(id.clone());
            }
        }

        categories
    }

    fn emit_streaming_update(
        &mut self,
        ctx: &ProcessContext,
        categories: &DeltaCategories,
        db: &Database,
        is_optimistic: bool,
    ) -> Option<ViewUpdate> {
        let mut delta_records: Vec<DeltaRecord> = Vec::new();

        if ctx.is_first_run {
            delta_records = self.handle_first_run_streaming(db);
        } else if ctx.has_subquery_changes {
            delta_records = self.handle_subquery_changes_streaming(db);
        } else {
            delta_records = self.handle_normal_streaming(categories, db, is_optimistic);
        }

        // Update sentinel hash
        self.last_hash = "streaming".to_string();

        if delta_records.is_empty() && !ctx.is_first_run {
            return None;
        }

        Some(ViewUpdate::Streaming(StreamingUpdate {
            view_id: self.plan.id.clone(),
            records: delta_records,
        }))
    }

    fn handle_first_run_streaming(&mut self, db: &Database) -> Vec<DeltaRecord> {
        let mut records = Vec::new();
        let target_set = self
            .eval_snapshot(&self.plan.root, db, self.params.as_ref())
            .into_owned();

        // Collect all IDs including subquery children
        let mut all_ids: Vec<String> = Vec::new();
        for (id, weight) in &target_set {
            if *weight > 0 {
                all_ids.push(id.to_string());
                if let Some(parent_row) = self.get_row_value(id.as_str(), db) {
                    self.collect_subquery_ids_recursive(&self.plan.root, parent_row, db, &mut all_ids);
                }
            }
        }

        // Deduplicate
        all_ids.sort_unstable();
        all_ids.dedup();

        // Create events
        for id in all_ids {
            let id_key = SmolStr::new(id.as_str());
            self.version_map.insert(id_key, 1);
            records.push(DeltaRecord {
                id,
                event: DeltaEvent::Created,
                version: 1,
            });
        }

        records
    }

    fn handle_subquery_changes_streaming(&mut self, db: &Database) -> Vec<DeltaRecord> {
        let mut records = Vec::new();
        let target_set = self
            .eval_snapshot(&self.plan.root, db, self.params.as_ref())
            .into_owned();

        // Collect all current IDs
        let mut current_ids: Vec<String> = Vec::new();
        for (main_id, _) in &target_set {
            current_ids.push(main_id.to_string());
            if let Some(parent_row) = self.get_row_value(main_id.as_str(), db) {
                self.collect_subquery_ids_recursive(&self.plan.root, parent_row, db, &mut current_ids);
            }
        }
        current_ids.sort_unstable();
        current_ids.dedup();

        let current_set: std::collections::HashSet<&str> = 
            current_ids.iter().map(|s| s.as_str()).collect();

        // Find NEW IDs
        for id in &current_ids {
            if !self.version_map.contains_key(id.as_str()) {
                let id_key = SmolStr::new(id.as_str());
                self.version_map.insert(id_key, 1);
                records.push(DeltaRecord {
                    id: id.clone(),
                    event: DeltaEvent::Created,
                    version: 1,
                });
            }
        }

        // Find REMOVED IDs
        let version_keys: Vec<SmolStr> = self.version_map.keys().cloned().collect();
        for id in version_keys {
            if !current_set.contains(id.as_str()) {
                self.version_map.remove(&id);
                records.push(DeltaRecord {
                    id: id.to_string(),
                    event: DeltaEvent::Deleted,
                    version: 0,
                });
            }
        }

        records
    }

    fn handle_normal_streaming(
        &mut self,
        categories: &DeltaCategories,
        db: &Database,
        is_optimistic: bool,
    ) -> Vec<DeltaRecord> {
        let mut records = Vec::new();

        // Handle removals first
        for id in &categories.removals {
            let id_key = SmolStr::new(id.as_str());
            self.version_map.remove(&id_key);
            records.push(DeltaRecord {
                id: id.clone(),
                event: DeltaEvent::Deleted,
                version: 0,
            });
        }

        // Handle additions
        for id in &categories.additions {
            let id_key = SmolStr::new(id.as_str());
            let version = self.version_map.entry(id_key).or_insert(0);
            if *version == 0 {
                *version = 1;
            }
            records.push(DeltaRecord {
                id: id.clone(),
                event: DeltaEvent::Created,
                version: *version,
            });

            // Collect subquery IDs for new additions
            if let Some(parent_row) = self.get_row_value(id.as_str(), db) {
                let mut subquery_ids = Vec::new();
                self.collect_subquery_ids_recursive(&self.plan.root, parent_row, db, &mut subquery_ids);
                for sub_id in subquery_ids {
                    if !self.version_map.contains_key(sub_id.as_str()) {
                        let sub_key = SmolStr::new(sub_id.as_str());
                        self.version_map.insert(sub_key, 1);
                        records.push(DeltaRecord {
                            id: sub_id,
                            event: DeltaEvent::Created,
                            version: 1,
                        });
                    }
                }
            }
        }

        // Handle updates
        if is_optimistic {
            for id in &categories.updates {
                let id_key = SmolStr::new(id.as_str());
                let version = self.version_map.entry(id_key).or_insert(0);
                *version += 1;
                records.push(DeltaRecord {
                    id: id.clone(),
                    event: DeltaEvent::Updated,
                    version: *version,
                });
            }
        } else {
            // Non-optimistic: still emit updates but don't increment version
            for id in &categories.updates {
                let version = self.version_map.get(id.as_str()).copied().unwrap_or(1);
                records.push(DeltaRecord {
                    id: id.clone(),
                    event: DeltaEvent::Updated,
                    version,
                });
            }
        }

        records
    }

    fn emit_materialized_update(
        &mut self,
        ctx: &ProcessContext,
        view_delta: &ZSet,
        categories: &DeltaCategories,
        db: &Database,
        is_optimistic: bool,
    ) -> Option<ViewUpdate> {
        // Apply cache updates
        if !ctx.is_streaming {
            for (key, weight) in view_delta {
                let entry = self.cache.entry(key.clone()).or_insert(0);
                *entry += weight;
                if *entry == 0 {
                    self.cache.remove(key);
                }
            }
        }

        // Build result set
        let mut result_ids: Vec<String> = self.cache.keys().map(|k| k.to_string()).collect();
        result_ids.sort_unstable();

        // Collect all IDs including subqueries
        let mut all_ids: Vec<String> = Vec::new();
        for id in &result_ids {
            all_ids.push(id.clone());
            if let Some(parent_row) = self.get_row_value(id, db) {
                self.collect_subquery_ids_recursive(&self.plan.root, parent_row, db, &mut all_ids);
            }
        }
        all_ids.sort_unstable();
        all_ids.dedup();

        // Update version map
        for id in &all_ids {
            if let Some(_hash) = self.get_row_hash(id, db) {
                let id_key = SmolStr::new(id);
                let version = self.version_map.entry(id_key).or_insert(0);
                if *version == 0 {
                    *version = 1;
                } else if is_optimistic && categories.updates.contains(id) {
                    *version += 1;
                }
            }
        }

        // Build result data
        let result_data: Vec<(String, u64)> = all_ids
            .iter()
            .map(|id| {
                let version = self.version_map.get(id.as_str()).copied().unwrap_or(1);
                (id.clone(), version)
            })
            .collect();

        // Compute hash
        let hash = super::update::compute_flat_hash(&result_data);
        
        if hash == self.last_hash && !ctx.is_first_run {
            return None;
        }
        self.last_hash = hash.clone();

        let update = MaterializedViewUpdate {
            query_id: self.plan.id.clone(),
            result_hash: hash,
            result_data,
        };

        Some(match self.format {
            ViewResultFormat::Tree => ViewUpdate::Tree(update),
            _ => ViewUpdate::Flat(update),
        })
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

    /// Recursively collect all record IDs from subqueries, using the correct parent context for each level.
    /// This ensures nested subqueries (like comment.author inside comments) get the right parent row.
    fn collect_subquery_ids_recursive(
        &self,
        op: &Operator,
        parent_row: &SpookyValue,
        db: &Database,
        out: &mut Vec<String>,
    ) {
        match op {
            Operator::Project { input, projections } => {
                // First, recurse into the input operator (to handle any projections further down)
                self.collect_subquery_ids_recursive(input, parent_row, db, out);
                
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
                                // Only recurse into the subquery's plan to find ITS nested subqueries
                                // This handles cases like: comments -> comment.author, comments -> comment.replies -> reply.author
                                self.collect_nested_subquery_ids(plan, sub_row, db, out);
                            }
                        }
                    }
                }
            }
            Operator::Filter { input, .. } => {
                self.collect_subquery_ids_recursive(input, parent_row, db, out);
            }
            Operator::Limit { input, .. } => {
                self.collect_subquery_ids_recursive(input, parent_row, db, out);
            }
            Operator::Join { left, right, .. } => {
                self.collect_subquery_ids_recursive(left, parent_row, db, out);
                self.collect_subquery_ids_recursive(right, parent_row, db, out);
            }
            Operator::Scan { .. } => {
                // Base case: no subqueries in a scan
            }
        }
    }

    /// Helper: Collect IDs from subqueries nested within a subquery's plan.
    /// This is separate from collect_subquery_ids_recursive because we only want to find
    /// the Project nodes that contain subquery projections, not re-evaluate the entire query.
    fn collect_nested_subquery_ids(
        &self,
        op: &Operator,
        parent_row: &SpookyValue,
        db: &Database,
        out: &mut Vec<String>,
    ) {
        match op {
            Operator::Project { input, projections } => {
                // First check the input for any nested Projects with subqueries
                self.collect_nested_subquery_ids(input, parent_row, db, out);
                
                // Process subquery projections at this level
                for proj in projections {
                    if let Projection::Subquery { plan, .. } = proj {
                        // Evaluate this nested subquery with the current parent context
                        let subquery_results = self
                            .eval_snapshot(plan, db, Some(parent_row))
                            .into_owned();
                        
                        for (sub_id, _weight) in &subquery_results {
                            out.push(sub_id.to_string());
                            
                            // Recursively handle even deeper nesting
                            if let Some(sub_row) = self.get_row_value(sub_id.as_str(), db) {
                                self.collect_nested_subquery_ids(plan, sub_row, db, out);
                            }
                        }
                    }
                }
            }
            Operator::Filter { input, .. } => {
                self.collect_nested_subquery_ids(input, parent_row, db, out);
            }
            Operator::Limit { input, .. } => {
                self.collect_nested_subquery_ids(input, parent_row, db, out);
            }
            Operator::Join { left, right, .. } => {
                self.collect_nested_subquery_ids(left, parent_row, db, out);
                self.collect_nested_subquery_ids(right, parent_row, db, out);
            }
            Operator::Scan { .. } => {
                // Base case
            }
        }
    }

    /// Check if deltas contain changes (CREATE or DELETE) for tables used in subqueries
    /// This is needed because new/deleted records need full scan to update subquery results
    fn has_changes_for_subqueries(&self, deltas: &FastMap<String, ZSet>, _db: &Database) -> bool {
        // Get all tables used in subqueries
        let subquery_tables = self.extract_subquery_tables(&self.plan.root);

        debug_log!(
            "DEBUG has_changes: view={} subquery_tables={:?} delta_tables={:?}",
            self.plan.id,
            subquery_tables,
            deltas.keys().collect::<Vec<_>>()
        );

        if subquery_tables.is_empty() {
            debug_log!(
                "DEBUG has_changes: view={} NO SUBQUERY TABLES",
                self.plan.id
            );
            return false;
        }

        // Check if any delta for a subquery table contains changes (weight != 0)
        for table in subquery_tables {
            if let Some(delta) = deltas.get(&table) {
                debug_log!(
                    "DEBUG has_changes: view={} table={} delta_keys={:?}",
                    self.plan.id,
                    table,
                    delta.keys().collect::<Vec<_>>()
                );
                // Check if any record in this delta is a CREATE (weight > 0 and not in version_map)
                // or a DELETE (weight < 0 and in version_map)
                for (key, weight) in delta {
                    // Use SmolStr for lookup to ensure hash compatibility with FxHasher
                    let key_smol = SmolStr::new(key.as_str());
                    let in_version_map = self.version_map.contains_key(&key_smol);
                    debug_log!(
                        "DEBUG has_changes: view={} key={} weight={} in_version_map={}",
                        self.plan.id,
                        key,
                        weight,
                        in_version_map
                    );
                    // CREATE: positive weight, not in version_map
                    // DELETE: negative weight, in version_map
                    if (*weight > 0 && !in_version_map) || (*weight < 0 && in_version_map) {
                        debug_log!(
                            "DEBUG has_changes: view={} FOUND CHANGE key={} weight={}",
                            self.plan.id,
                            key,
                            weight
                        );
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
                    debug_log!(
                        "DEBUG get_updated_cached_records: view={} table={} found cached record={}",
                        self.plan.id,
                        _table,
                        record_id
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
                    debug_log!("DEBUG get_updated_cached_records: view={} table={} found versioned record={}", self.plan.id, _table, record_id);
                    updated_ids.push(record_id.to_string());
                }
            }
        }

        updated_ids
    }

    /// Get all record IDs in the view (via version_map) that have been updated in the deltas.
    /// This is the streaming-mode variant that uses version_map instead of cache.
    fn get_updated_records_streaming(&self, deltas: &FastMap<String, ZSet>) -> Vec<String> {
        let mut updated_ids = Vec::new();

        for (_table, delta) in deltas {
            for (record_id, weight) in delta {
                // Only check records with positive weight (existing/updated records)
                // and that are already in the view (tracked in version_map)
                if *weight > 0 && self.version_map.contains_key(record_id.as_str()) {
                    debug_log!(
                        "DEBUG get_updated_records_streaming: view={} table={} found versioned record={}",
                        self.plan.id,
                        _table,
                        record_id
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
    ) -> Option<ViewUpdate> {
        let current_version = self.version_map.get(record_id).copied().unwrap_or(0);

        if current_version != version {
            debug_log!(
                "DEBUG VIEW: set_record_version id={} record={} old={} new={}",
                self.plan.id,
                record_id,
                current_version,
                version
            );
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