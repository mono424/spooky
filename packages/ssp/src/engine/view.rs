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

use tracing::{instrument, info, debug};

use super::metadata::{MetadataProcessor, ViewMetadataState, VersionStrategy, BatchMeta};
use super::update::{
    build_update, RawViewResult, ViewDelta,
};

// Re-export types for backward compatibility
pub use super::operators::{JoinCondition, Operator, OrderSpec, Predicate, Projection};
pub use super::types::{FastMap, Path, RowKey, SpookyValue, Weight, ZSet};
pub use super::update::{MaterializedViewUpdate, ViewResultFormat, ViewUpdate};
// VersionMap is now in metadata
pub use super::metadata::VersionMap; 

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct QueryPlan {
    pub id: String,
    pub root: Operator,
}

/// Context for view processing - computed once, used throughout
struct ProcessContext<'a> {
    is_first_run: bool,
    is_streaming: bool,
    has_subquery_changes: bool,
    batch_meta: Option<&'a BatchMeta>,
}

impl<'a> ProcessContext<'a> {
    #[inline]
    fn new(view: &mut View, deltas: &FastMap<String, ZSet>, db: &Database, batch_meta: Option<&'a BatchMeta>) -> Self {
        let is_first_run = view.metadata.is_first_run();
        // We need to pass &mut View to check for subquery changes IF we want to cache checking?
        // Actually has_changes_for_subqueries doesn't mutate, but get_subquery_tables (which I will add) might mutate cache.
        // Let's assume view is mut.
        let has_subquery_changes = !is_first_run && view.has_changes_for_subqueries(deltas, db);
        
        Self {
            is_first_run,
            is_streaming: matches!(view.format, ViewResultFormat::Streaming),
            has_subquery_changes,
            batch_meta,
        }
    }

    #[inline]
    fn should_full_scan(&self) -> bool {
        self.is_first_run || self.has_subquery_changes
    }
}

/// Result of change categorization
struct CategorizedChanges {
    delta: ZSet,
    additions: Vec<SmolStr>,
    removals: Vec<SmolStr>,
    updates: Vec<SmolStr>,
}

impl CategorizedChanges {
    #[inline]
    fn with_capacity(cap: usize) -> Self {
        Self {
            delta: FastMap::default(),
            additions: Vec::with_capacity(cap),
            removals: Vec::with_capacity(cap / 4),
            updates: Vec::with_capacity(cap / 2),
        }
    }

    #[inline]
    fn is_empty(&self) -> bool {
        self.additions.is_empty() && self.removals.is_empty() && self.updates.is_empty()
    }
}

/// Helper function for serde to skip serializing empty caches
fn is_cache_empty_or_streaming(cache: &ZSet) -> bool {
    cache.is_empty()
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct View {
    pub plan: QueryPlan,
    /// Cache is only used for Flat/Tree modes. For Streaming mode, metadata is the source of truth.
    /// Skip serializing if empty (streaming mode keeps it empty).
    #[serde(default, skip_serializing_if = "is_cache_empty_or_streaming")]
    pub cache: ZSet,
    #[serde(default)]
    pub params: Option<SpookyValue>,
    #[serde(default)]
    pub format: ViewResultFormat, // Output format strategy
    
    // NEW: Metadata state (replaces version_map + last_hash)
    #[serde(default)]
    pub metadata: ViewMetadataState,

    // NEW: Cache subquery tables (computed once per view)
    #[serde(skip)]
    subquery_tables_cache: Option<std::collections::HashSet<SmolStr>>,
}

impl View {
    pub fn new(plan: QueryPlan, params: Option<Value>, format: Option<ViewResultFormat>) -> Self {
        let fmt = format.unwrap_or_default();
        
        // Determine version strategy based on format
        let strategy = match fmt {
             // For Tree/Flat, we often rely on hash changes, but for simplicity default to Optimistic
             // or HashBased if we want purely content-based versioning.
             // The plan suggests HashBased for Tree.
            ViewResultFormat::Tree => VersionStrategy::HashBased,
            _ => VersionStrategy::Optimistic,
        };

        Self {
            plan,
            cache: FastMap::default(),
            params: params.map(SpookyValue::from),
            format: fmt,
            metadata: ViewMetadataState::new(strategy),
            subquery_tables_cache: None,
        }
    }
    
    // NEW: Constructor with explicit metadata config
    pub fn new_with_strategy(
        plan: QueryPlan,
        params: Option<Value>,
        format: Option<ViewResultFormat>,
        strategy: VersionStrategy,
    ) -> Self {
        Self {
            plan,
            cache: FastMap::default(),
            params: params.map(SpookyValue::from),
            format: format.unwrap_or_default(),
            metadata: ViewMetadataState::new(strategy),
            subquery_tables_cache: None,
        }
    }

    /// The main function for updates.
    /// Uses delta optimization if possible.
    #[inline]
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
        self.process_ingest_with_meta(deltas, db, is_optimistic, None)
    }

    /// NEW: Process with optional explicit metadata
    #[instrument(target = "ssp_module", level = "debug", ret(level = "debug"))]
    pub fn process_ingest_with_meta(
        &mut self,
        deltas: &FastMap<String, ZSet>,
        db: &Database,
        is_optimistic: bool,
        batch_meta: Option<&BatchMeta>,
    ) -> Option<ViewUpdate> {
        let ctx = ProcessContext::new(self, deltas, db, batch_meta);

        debug_log!(
            "DEBUG VIEW: id={} is_first_run={} has_subquery_changes={} is_streaming={}",
            self.plan.id,
            ctx.is_first_run,
            ctx.has_subquery_changes,
            ctx.is_streaming
        );

        // Step 1: Compute Delta
        let input_delta = if ctx.should_full_scan() {
             None
        } else {
             self.eval_delta_batch(&self.plan.root, deltas, db, self.params.as_ref())
        };

        // Step 2: Compute Changes using Context
        let changes = self.compute_changes(input_delta, &ctx, deltas, db);

        // Step 3: Early Exit
        if changes.is_empty() && !ctx.should_full_scan() {
             return None;
        }

        // Step 4: Update Cache (Non-streaming only)
        if !ctx.is_streaming {
            for (key, weight) in &changes.delta {
                let entry = self.cache.entry(key.clone()).or_insert(0);
                *entry += weight;
                if *entry == 0 {
                    self.cache.remove(key);
                }
            }
        }

        // Step 5: Build Raw Result
        let raw_result = self.build_raw_result(&changes, &ctx, is_optimistic, db);

        // Step 6: Format Output
        let update = build_update(raw_result, self.format.clone());

        // Step 7: Check if update should be emitted
        if self.should_emit_update(&update) {
            Some(update)
        } else {
            None
        }
    }

    /// Helper to compute ZSet delta and categorize changes
    fn compute_changes(
        &self,
        input_delta: Option<ZSet>,
        ctx: &ProcessContext,
        deltas: &FastMap<String, ZSet>,
        db: &Database,
    ) -> CategorizedChanges {
        if let Some(d) = input_delta {
            // We have a computed delta, calculate changes
            // Identify updated records (content changed but still in view)
            let updated_ids = if ctx.is_streaming {
                self.get_updated_records_streaming(deltas)
            } else {
                 self.get_updated_cached_records(deltas)
            };
            
            self.categorize_delta_changes(d, &updated_ids)
        } else {
            // Fallback: Full Scan & Diff
             let target_set = self
                .eval_snapshot(&self.plan.root, db, self.params.as_ref())
                .into_owned();
            
            let mut diff = FastMap::default();
            
            if ctx.is_streaming {
                // Streaming diff against metadata
                for (key, &new_w) in &target_set {
                    if new_w > 0 && !self.metadata.contains(key.as_str()) {
                         diff.insert(key.clone(), 1);
                    }
                }
                
                if !ctx.has_subquery_changes {
                     // We can use the cached subquery tables here if we had access to mutable self, 
                     // but compute_changes is &self.
                     // However, extract_subquery_tables is fast enough for the fallback path usually.
                     let subquery_tables: std::collections::HashSet<String> = 
                        self.extract_subquery_tables(&self.plan.root).into_iter().collect();

                     for key in self.metadata.versions.keys() {
                         if !target_set.contains_key(key.as_str()) {
                             if let Some((table_name, _)) = key.split_once(':') {
                                 if !subquery_tables.contains(table_name) {
                                      diff.insert(key.clone(), -1);
                                 }
                             }
                         }
                     }
                }
            } else {
                // Cache diff
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
            }

            let updated_ids = if ctx.is_streaming {
                self.get_updated_records_streaming(deltas)
            } else {
                 self.get_updated_cached_records(deltas)
            };

            self.categorize_delta_changes(diff, &updated_ids)
        }
    }

    fn categorize_delta_changes(
        &self,
        delta: ZSet,
        updated_record_ids: &[String],
    ) -> CategorizedChanges {
         let delta_size = delta.len();
         let mut changes = CategorizedChanges::with_capacity(delta_size);
         changes.delta = delta;

         // Use sorted vec for faster lookups if updated_record_ids is large, 
         // but for now Hash set is O(1). 
         let updated_ids_set: std::collections::HashSet<&str> = 
            updated_record_ids.iter().map(|s| s.as_str()).collect();

         for (key, weight) in &changes.delta {
             if *weight > 0 {
                 if !updated_ids_set.contains(&key.as_str()) {
                     changes.additions.push(SmolStr::from(key.as_str()));
                 }
             } else if *weight < 0 {
                 changes.removals.push(SmolStr::from(key.as_str()));
             }
         }

         let removal_ids_set: std::collections::HashSet<&str> = 
            changes.removals.iter().map(|s| s.as_str()).collect();
            
         changes.updates = updated_record_ids.iter()
            .filter(|id| !removal_ids_set.contains(id.as_str()))
            .map(SmolStr::from)
            .collect();

         changes
    }


    /// Build RawViewResult by coordinating with MetadataProcessor
    fn build_raw_result(
        &mut self,
        changes: &CategorizedChanges,
        ctx: &ProcessContext,
        is_optimistic: bool,
        db: &Database,
    ) -> RawViewResult {
        let processor = MetadataProcessor::new(self.metadata.strategy.clone());

        let mut raw = RawViewResult {
            query_id: self.plan.id.clone(),
            records: Vec::new(),
            delta: None,
        };

        if ctx.is_streaming {
            self.build_streaming_raw_result(
                &mut raw, changes, ctx, is_optimistic,
                &processor, db
            );
        } else {
            self.build_materialized_raw_result(
                &mut raw, changes,
                is_optimistic, &processor, ctx, db
            );
        }
        
        raw
    }

    fn build_streaming_raw_result(
        &mut self,
        raw: &mut RawViewResult,
        changes: &CategorizedChanges,
        ctx: &ProcessContext,
        is_optimistic: bool,
        processor: &MetadataProcessor,
        db: &Database,
    ) {
         let mut delta_out = ViewDelta::default();

         if ctx.is_first_run {
             // First run logic
             let mut all_first_run_ids = Vec::new();
             for (id, weight) in &changes.delta {
                 if *weight > 0 {
                     all_first_run_ids.push(id.to_string());
                     if let Some(parent_row) = self.get_row_value(id.as_str(), db) {
                         self.collect_subquery_ids_recursive(&self.plan.root, parent_row, db, &mut all_first_run_ids);
                     }
                 }
             }
             all_first_run_ids.sort_unstable();
             all_first_run_ids.dedup();

             for id in all_first_run_ids {
                 let version = self.compute_and_store_version(&id, processor, ctx, true, false);
                 delta_out.additions.push((id, version));
             }
         } else if ctx.has_subquery_changes {
             // Subquery changes logic (simplified from original)
             let target_set = self.eval_snapshot(&self.plan.root, db, self.params.as_ref()).into_owned();
             let mut all_current_ids = Vec::new();
             for (main_id, _) in &target_set {
                 all_current_ids.push(main_id.to_string());
                 if let Some(parent_row) = self.get_row_value(main_id.as_str(), db) {
                     self.collect_subquery_ids_recursive(&self.plan.root, parent_row, db, &mut all_current_ids);
                 }
             }
             all_current_ids.sort_unstable();
             all_current_ids.dedup();

             // Additions (New subquery results)
             for id in &all_current_ids {
                 if !self.metadata.contains(id.as_str()) {
                     let version = self.compute_and_store_version(id, processor, ctx, true, false);
                     delta_out.additions.push((id.clone(), version));
                 }
             }
             
             // Removals (Removed subquery results)
             let current_set: std::collections::HashSet<&str> = all_current_ids.iter().map(|s| s.as_str()).collect();
             let current_keys: Vec<SmolStr> = self.metadata.versions.keys().cloned().collect();
             for id in current_keys {
                 if !current_set.contains(id.as_str()) {
                     self.metadata.remove(&id);
                     delta_out.removals.push(id.to_string());
                 }
             }
         } else {
             // Normal streaming
            for id in &changes.additions {
                let version = self.compute_and_store_version(id, processor, ctx, true, false);
                delta_out.additions.push((id.to_string(), version));
                
                // Recursively check for subquery records associated with this new record
                if let Some(parent_row) = self.get_row_value(id, db) {
                    let mut sub_ids = Vec::new();
                    self.collect_subquery_ids_recursive(&self.plan.root, parent_row, db, &mut sub_ids);
                    
                    for sub_id in sub_ids {
                        // Only add if not already tracked (and not the main id we just added)
                        if sub_id != id.as_str() && !self.metadata.contains(&sub_id) {
                            let v = self.compute_and_store_version(&sub_id, processor, ctx, true, false);
                            delta_out.additions.push((sub_id, v));
                        }
                    }
                }
            }
             for id in &changes.removals {
                 self.metadata.remove(id);
                 delta_out.removals.push(id.to_string());
             }
             for id in &changes.updates {
                 let version = self.compute_and_store_version(id, processor, ctx, false, is_optimistic);
                 delta_out.updates.push((id.to_string(), version));
             }
         }
         
         raw.delta = Some(delta_out);
    }

    fn build_materialized_raw_result(
        &mut self,
        raw: &mut RawViewResult,
        changes: &CategorizedChanges,
        is_optimistic: bool,
        processor: &MetadataProcessor,
        ctx: &ProcessContext,
        db: &Database,
    ) {
         // Build full snapshot
         let result_ids: Vec<String> = self.cache.keys().map(|k| k.to_string()).collect();
         // ... need to collect subqueries ...
         let mut all_ids = Vec::new();
         for id in &result_ids {
             all_ids.push(id.clone());
             if let Some(parent_row) = self.get_row_value(id, db) {
                 self.collect_subquery_ids_recursive(&self.plan.root, parent_row, db, &mut all_ids);
             }
         }
         all_ids.sort_unstable();
         all_ids.dedup();
         
         let additions_set: std::collections::HashSet<&str> = 
             changes.additions.iter().map(|id| id.as_str()).collect();
         let updates_set: std::collections::HashSet<&str> = 
             changes.updates.iter().map(|id| id.as_str()).collect();

         for id in all_ids {
             let is_update = updates_set.contains(id.as_str());
             let is_new = additions_set.contains(id.as_str());
             
             // Logic to determine if version should change
             // If it's an update, we might increment. If it's existing, we keep.
             // If it's new (addition), we set.
             
             let version = self.compute_and_store_version(&id, processor, ctx, is_new, is_optimistic && is_update);
             raw.records.push((id, version));
         }
    }

    #[inline]
    fn compute_and_store_version(
        &mut self,
        id: &str,
        processor: &MetadataProcessor,
        ctx: &ProcessContext,
        is_new: bool,
        is_optimistic: bool,
    ) -> u64 {
        let current = self.metadata.get_version(id);
        
        // Check for explicit version in batch metadata
        if let Some(batch_meta) = ctx.batch_meta {
            if let Some(record_meta) = batch_meta.get(id) {
                if let Some(explicit_version) = record_meta.version {
                    self.metadata.set_version(id, explicit_version);
                    return explicit_version;
                }
            }
        }
        
        let result = if is_new {
            processor.compute_new_version(id, current, None)
        } else {
            processor.compute_update_version(id, current, None, is_optimistic)
        };
        
        if result.changed || is_new {
            self.metadata.set_version(id, result.version);
        }
        
        result.version
    }

    fn should_emit_update(&mut self, update: &ViewUpdate) -> bool {
        match update {
            ViewUpdate::Streaming(_) => {
                self.metadata.last_result_hash = "streaming".to_string();
                true
            }
            ViewUpdate::Flat(m) | ViewUpdate::Tree(m) => {
                if m.result_hash != self.metadata.last_result_hash {
                    self.metadata.last_result_hash = m.result_hash.clone();
                    true
                } else {
                    false
                }
            }
        }
    }

    /// Find all Subquery projections in the operator tree
    #[allow(dead_code)]
    fn find_subquery_projections(&self, op: &Operator) -> Vec<Operator> {
        let mut subqueries = Vec::new();
        self.collect_subquery_projections(op, &mut subqueries);
        subqueries
    }

    #[allow(dead_code)]
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
    fn has_changes_for_subqueries(&mut self, deltas: &FastMap<String, ZSet>, _db: &Database) -> bool {
        // Clone ID to allow borrowing self below
        let plan_id = self.plan.id.clone();
        // Get all tables used in subqueries
        let subquery_tables = self.get_subquery_tables();

        debug_log!(
            "DEBUG has_changes: view={} subquery_tables={:?} delta_tables={:?}",
            plan_id,
            subquery_tables,
            deltas.keys().collect::<Vec<_>>()
        );

        if subquery_tables.is_empty() {
            return false;
        }

        // Check if any delta for a subquery table contains changes (weight != 0)
        // Copy keys to avoid referencing self (via subquery_tables which refers to cache)
        let tables: Vec<String> = subquery_tables.iter().map(|s| s.to_string()).collect();
        for table in tables {
            if let Some(delta) = deltas.get(table.as_str()) {
                // Check if any record in this delta is a CREATE (weight > 0 and not in version_map)
                // or a DELETE (weight < 0 and in version_map)
                for (key, weight) in delta {
                    let in_version_map = self.metadata.contains(key.as_str());
                    if (*weight > 0 && !in_version_map) || (*weight < 0 && in_version_map) {
                        return true;
                    }
                }
            }
        }

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
                    && self.metadata.contains(record_id.as_str())
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
                if *weight > 0 && self.metadata.contains(record_id.as_str()) {
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
        let current_version = self.metadata.get_version(record_id);

        if current_version != version {
            debug_log!(
                "DEBUG VIEW: set_record_version id={} record={} old={} new={}",
                self.plan.id,
                record_id,
                current_version,
                version
            );
            self.metadata.set_version(record_id, version);

            // Trigger re-hashing by processing empty deltas
            let empty_deltas = FastMap::default();
            // We pass is_optimistic=false because we've already manually manipulated the version map
            // and we just want to recompute the hash and return the update.
            self.process_ingest(&empty_deltas, db, false)
        } else {
            None
        }
    }

    fn get_subquery_tables(&mut self) -> &std::collections::HashSet<SmolStr> {
        if self.subquery_tables_cache.is_none() {
            self.subquery_tables_cache = Some(
                self.extract_subquery_tables(&self.plan.root)
                    .into_iter()
                    .map(SmolStr::from)
                    .collect()
            );
        }
        self.subquery_tables_cache.as_ref().unwrap()
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

    #[allow(dead_code)]
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