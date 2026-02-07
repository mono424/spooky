use super::circuit::Database;
use super::eval::{
    apply_numeric_filter, compare_spooky_values, hash_spooky_value, resolve_nested_value,
    NumericFilterConfig,
};
use super::operators::{Operator, Projection};
use super::types::{
    parse_zset_key, BatchDeltas, Delta, FastMap, SpookyValue, ZSet, ZSetMembershipOps,
};

use super::operators::check_predicate;
use super::update::{ViewResultFormat, ViewUpdate};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use smol_str::SmolStr;
use std::cmp::Ordering;

mod sort_direction {
    pub const DESC: &str = "DESC";
    // pub const ASC: &str = "ASC";
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct QueryPlan {
    pub id: String,
    pub root: Operator,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct View {
    pub plan: QueryPlan,
    /// Unified cache: Source of truth for all modes (Flat, Tree, Streaming).
    /// Tracks ALL records that are part of the view (including subquery results).
    /// IMPORTANT: Always serialize - needed for correct delta computation!
    #[serde(default)]
    pub cache: ZSet,
    pub last_hash: String,
    #[serde(default)]
    pub has_run: bool,
    #[serde(default)]
    pub params: Option<SpookyValue>,
    #[serde(default)]
    pub format: ViewResultFormat, // Output format strategy

    // Cached characteristics
    #[serde(skip)]
    pub has_subqueries_cached: bool,
    #[serde(skip)]
    pub referenced_tables_cached: Vec<String>,
    #[serde(skip)]
    pub is_simple_scan: bool,
    #[serde(skip)]
    pub is_simple_filter: bool,
}

impl View {
    pub fn new(plan: QueryPlan, params: Option<Value>, format: Option<ViewResultFormat>) -> Self {
        let has_subqueries_cached = plan.root.has_subquery_projections();
        let referenced_tables_cached = plan.root.referenced_tables();

        let is_simple_scan = matches!(plan.root, Operator::Scan { .. });
        let is_simple_filter = if let Operator::Filter { input, .. } = &plan.root {
            matches!(input.as_ref(), Operator::Scan { .. })
        } else {
            false
        };

        Self {
            plan,
            cache: FastMap::default(),
            last_hash: String::new(),
            has_run: false,
            params: params.map(SpookyValue::from),
            format: format.unwrap_or_default(),
            has_subqueries_cached,
            referenced_tables_cached,
            is_simple_scan,
            is_simple_filter,
        }
    }

    /// Initialize cached flags after deserialization
    ///
    /// IMPORTANT: Call this after deserializing a View from storage!
    /// The cached flags are not serialized to save space, so they must
    /// be recomputed when loading state.
    pub fn initialize_after_deserialize(&mut self) {
        self.has_subqueries_cached = self.plan.root.has_subquery_projections();
        self.referenced_tables_cached = self.plan.root.referenced_tables();
        self.is_simple_scan = matches!(self.plan.root, Operator::Scan { .. });
        self.is_simple_filter = if let Operator::Filter { input, .. } = &self.plan.root {
            matches!(input.as_ref(), Operator::Scan { .. })
        } else {
            false
        };

        tracing::debug!(
            target: "ssp::view::init",
            view_id = %self.plan.id,
            has_subqueries = self.has_subqueries_cached,
            is_simple_scan = self.is_simple_scan,
            is_simple_filter = self.is_simple_filter,
            referenced_tables = ?self.referenced_tables_cached,
            "Initialized cached flags after deserialize"
        );
    }

    /// Check if cached flags are initialized
    /// Check if cached flags are initialized
    pub fn is_initialized(&self) -> bool {
        // FIX: Check if ANY cached flag is set (they're all computed together)
        !self.referenced_tables_cached.is_empty()
    }

    /// Process a delta for this view - optimized fast path for simple views
    pub fn process_delta(&mut self, delta: &Delta, db: &Database) -> Option<ViewUpdate> {
        // Fast check: Does this view even care about this table?
        if !self
            .referenced_tables_cached
            .iter()
            .any(|t| t == delta.table.as_str())
        {
            return None;
        }

        // Case 1: Membership change (Create or Delete)
        if delta.weight != 0 {
            // Fast path: Try optimized processing for simple Scan/Filter views
            if let Some(result) = self.try_fast_single(delta, db) {
                return result;
            }

            // Fallback: Use batch processing for complex views
            let mut batch_deltas = BatchDeltas::new();
            let mut zset = ZSet::default();
            zset.insert(delta.key.clone(), delta.weight);
            batch_deltas
                .membership
                .insert(delta.table.to_string(), zset);

            if delta.content_changed {
                batch_deltas
                    .content_updates
                    .entry(delta.table.to_string())
                    .or_default()
                    .insert(delta.key.clone()); // HashSet uses insert, not push
            }

            return self.process_batch(&batch_deltas, db);
        }

        // Case 2: Content-only update (weight=0, content_changed=true)
        if delta.content_changed {
            return self.process_content_update(delta, db);
        }

        // Case 3: No change
        None
    }

    /// Handle content-only update (no membership change)
    fn process_content_update(&mut self, delta: &Delta, db: &Database) -> Option<ViewUpdate> {
        let is_in_cache = self.cache.contains_key(&delta.key);
        let matches_filter = self.record_matches_view(&delta.key, db);

        match (is_in_cache, matches_filter) {
            (true, true) => {
                // Was in view, still in view - content update
                self.build_content_update_notification(&delta.key)
            }
            (true, false) => {
                // Was in view, no longer matches - directly remove from cache
                self.cache.remove(&delta.key);

                // Build removal notification
                use super::update::{build_update, RawViewResult, ViewDelta};

                // For streaming mode, only include the removed record
                // For Flat/Tree modes, need all remaining records for hash computation
                let result_data = match self.format {
                    ViewResultFormat::Streaming => {
                        vec![delta.key.clone()]
                    }
                    ViewResultFormat::Flat | ViewResultFormat::Tree => self.build_result_data(),
                };

                let view_delta = ViewDelta::removals_only(vec![delta.key.clone()]);

                let raw_result = RawViewResult {
                    query_id: self.plan.id.clone(),
                    records: result_data.clone(),
                    delta: Some(view_delta),
                };

                let update = build_update(raw_result, self.format);

                // Streaming: early exit, zero allocations
                if let ViewUpdate::Streaming(_s) = &update {
                    self.has_run = true;
                    return Some(update);
                }

                // Flat/Tree: hash-based change detection
                let hash = match &update {
                    ViewUpdate::Flat(flat) | ViewUpdate::Tree(flat) => flat.result_hash.clone(),
                    ViewUpdate::Streaming(_) => unreachable!(),
                };
                self.has_run = true;
                self.last_hash = hash;

                Some(update)
            }
            (false, true) => {
                // Was not in view, now matches - treat as addition
                let addition_delta = Delta {
                    table: delta.table.clone(),
                    key: delta.key.clone(),
                    weight: 1,
                    content_changed: false,
                };
                self.process_delta(&addition_delta, db)
            }
            (false, false) => {
                // Was not in view, still doesn't match - no update
                None
            }
        }
    }

    /// Check if a record matches this view's filters
    /// OPTIMIZATION: Avoid format! allocations using parse_zset_key
    /// Check if a record matches this view's filters
    /// OPTIMIZATION: Avoid format! allocations using parse_zset_key
    #[inline]
    fn record_matches_view(&self, key: &SmolStr, db: &Database) -> bool {
        match &self.plan.root {
            Operator::Scan { table } => parse_zset_key(key)
                .map(|(t, _)| t == table)
                .unwrap_or(false),
            Operator::Filter { input, predicate } => {
                if let Operator::Scan { table } = input.as_ref() {
                    let matches_table = parse_zset_key(key)
                        .map(|(t, _)| t == table)
                        .unwrap_or(false);

                    if !matches_table {
                        return false;
                    }
                    return check_predicate(&self, predicate, key, db, self.params.as_ref());
                }
                true
            }
            _ => true,
        }
    }

    /// Build notification for content-only update
    fn build_content_update_notification(&mut self, key: &SmolStr) -> Option<ViewUpdate> {
        use super::update::{build_update, RawViewResult, ViewDelta};

        // For streaming mode, only include the updated record
        // For Flat/Tree modes, need all records for hash computation
        let result_data = match self.format {
            ViewResultFormat::Streaming => {
                vec![key.clone()]
            }
            ViewResultFormat::Flat | ViewResultFormat::Tree => self.build_result_data(),
        };

        let view_delta = ViewDelta::updates_only(vec![key.clone()]);

        let raw_result = RawViewResult {
            query_id: self.plan.id.clone(),
            records: result_data,
            delta: Some(view_delta),
        };

        let update = build_update(raw_result, self.format);

        // For streaming, always emit. For flat/tree, content changed but set didn't - still notify
        match &update {
            ViewUpdate::Streaming(s) if !s.records.is_empty() => Some(update),
            ViewUpdate::Flat(_) | ViewUpdate::Tree(_) => Some(update),
            _ => None,
        }
    }

    /// Try fast single-record processing for simple views (Scan, Filter)
    /// Returns Some(result) if fast path was taken, None if fallback needed
    fn try_fast_single(&mut self, delta: &Delta, db: &Database) -> Option<Option<ViewUpdate>> {
        // Optimization: Early check using pre-computed flags
        if !self.is_simple_scan && !self.is_simple_filter {
            return None;
        }

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
                    let matches =
                        check_predicate(&self, predicate, &delta.key, db, self.params.as_ref());
                    if matches {
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
            _ => None, // Complex query (Join, Project, Limit), use batch path
        }
    }

    /// Apply a single record creation to the view (fast path)
    /// Apply a single record creation to the view (fast path)
    fn apply_single_create(&mut self, key: &SmolStr) -> Option<ViewUpdate> {
        let was_member = self.cache.is_member(key);

        // FIX: Use membership-aware add
        self.cache.add_member(key.clone());

        // Determine change type
        let (additions, updates) = if was_member {
            (vec![], vec![key.clone()])
        } else {
            (vec![key.clone()], vec![])
        };

        self.build_single_update(additions, vec![], updates)
    }

    /// Apply a single record deletion to the view (fast path)
    fn apply_single_delete(&mut self, key: &SmolStr) -> Option<ViewUpdate> {
        if !self.cache.is_member(key) {
            return None;
        }

        self.cache.remove_member(key);
        self.build_single_update(vec![], vec![key.clone()], vec![])
    }

    /// Helper to build update for single record changes
    fn build_single_update(
        &mut self,
        additions: Vec<SmolStr>,
        removals: Vec<SmolStr>,
        updates: Vec<SmolStr>,
    ) -> Option<ViewUpdate> {
        let is_first_run = !self.has_run;

        // For streaming mode, only include changed records
        // For Flat/Tree modes, need all records for hash
        let result_data = match self.format {
            ViewResultFormat::Streaming => {
                let mut changed_keys =
                    Vec::with_capacity(additions.len() + removals.len() + updates.len());
                changed_keys.extend(additions.iter().cloned());
                changed_keys.extend(removals.iter().cloned());
                changed_keys.extend(updates.iter().cloned());
                changed_keys
            }
            ViewResultFormat::Flat | ViewResultFormat::Tree => self.build_result_data(),
        };

        use super::update::{build_update, RawViewResult, ViewDelta};

        let view_delta_struct = if is_first_run {
            None
        } else {
            Some(ViewDelta {
                additions,
                removals,
                updates,
            })
        };

        let raw_result = RawViewResult {
            query_id: self.plan.id.clone(),
            records: result_data,
            delta: view_delta_struct,
        };

        let update = build_update(raw_result, self.format);

        // Streaming: early exit, zero allocations
        if let ViewUpdate::Streaming(s) = &update {
            if !s.records.is_empty() {
                self.has_run = true;
                return Some(update);
            }
            return None;
        }

        // Flat/Tree: hash-based change detection
        let hash = match &update {
            ViewUpdate::Flat(flat) | ViewUpdate::Tree(flat) => flat.result_hash.clone(),
            ViewUpdate::Streaming(_) => unreachable!(),
        };

        if hash != self.last_hash {
            self.has_run = true;
            self.last_hash = hash;
            Some(update)
        } else {
            None
        }
    }

    /// Check if this view has any subquery projections
    #[inline]
    fn has_subqueries(&self) -> bool {
        self.has_subqueries_cached
    }

    /// Process a batch of deltas and produce a `ViewUpdate` if the view state changed.
    ///
    /// # Optimization: Streaming vs Flat
    /// - **Streaming Mode**: Result data is filtered to ONLY include records that changed (Added/Removed/Updated).
    ///   This ensures O(1) payload size for O(1) changes, preventing full-view re-transmission.
    /// - **Flat/Tree Mode**: Result data includes ALL records in the view. This is required to compute
    ///   a consistent hash of the view state, which clients use to detect desynchronization.
    ///
    /// # Logic
    /// 1. Computes `view_delta` (Membership changes: Added/Removed).
    /// 2. Identifies `updated_record_ids` (Content changes for existing members).
    /// 3. Categorizes changes into `additions`, `removals`, `updates`.
    /// 4. Updates strict cache state.
    /// 5. Builds result data based on format (Filtered vs Full).
    pub fn process_batch(
        &mut self,
        batch_deltas: &BatchDeltas,
        db: &Database,
    ) -> Option<ViewUpdate> {
        // FIX: FIRST RUN CHECK
        let is_first_run = !self.has_run;

        tracing::debug!(
            target: "ssp::view::process_batch",
            view_id = %self.plan.id,
            is_first_run = is_first_run,
            cache_size_before = self.cache.len(),
            last_hash = %self.last_hash,
            "Starting process_batch"
        );

        // Compute view delta from membership changes
        let view_delta = self.compute_view_delta(&batch_deltas.membership, db, is_first_run);
        let updated_record_ids = self.get_content_updates_in_view(batch_deltas);

        let delta_additions: Vec<_> = view_delta
            .iter()
            .filter(|(_, w)| **w > 0)
            .map(|(k, _)| k.as_str())
            .collect();
        let delta_removals: Vec<_> = view_delta
            .iter()
            .filter(|(_, w)| **w < 0)
            .map(|(k, _)| k.as_str())
            .collect();

        tracing::debug!(
            target: "ssp::view::process_batch",
            view_id = %self.plan.id,
            delta_total = view_delta.len(),
            additions_count = delta_additions.len(),
            removals_count = delta_removals.len(),
            additions_sample = ?delta_additions.iter().take(5).collect::<Vec<_>>(),
            removals_sample = ?delta_removals.iter().take(5).collect::<Vec<_>>(),
            content_updates = updated_record_ids.len(),
            "Computed view delta (ZSet keys include table prefix)"
        );

        // Early return if no changes
        if view_delta.is_empty() && !is_first_run && updated_record_ids.is_empty() {
            tracing::debug!(
                target: "ssp::view::process_batch",
                view_id = %self.plan.id,
                "No changes detected, returning None"
            );
            return None;
        }

        // Categorize changes BEFORE applying delta to detect membership transitions (0 <-> 1)
        // This functionality uses the OLD cache state.
        let (additions, removals, updates) =
            self.categorize_changes(&view_delta, &updated_record_ids);

        // Apply delta to cache (Update state)
        self.apply_cache_delta(&view_delta);

        tracing::debug!(
            target: "ssp::view::process_batch",
            view_id = %self.plan.id,
            cache_size_after = self.cache.len(),
            categorized_additions = additions.len(),
            categorized_removals = removals.len(),
            categorized_updates = updates.len(),
            additions_sample = ?additions.iter().take(5).collect::<Vec<_>>(),
            removals_sample = ?removals.iter().take(5).collect::<Vec<_>>(),
            "Categorized changes (IDs are STRIPPED of table prefix)"
        );

        // Build result data
        // For Streaming mode: only include changed records to avoid sending entire view
        // For Flat/Tree modes: include all records for hash computation
        let result_data = match self.format {
            ViewResultFormat::Streaming => {
                // Collect only records that changed
                let mut changed_keys =
                    Vec::with_capacity(additions.len() + removals.len() + updates.len());
                changed_keys.extend(additions.iter().cloned());
                changed_keys.extend(removals.iter().cloned());
                changed_keys.extend(updates.iter().cloned());

                tracing::debug!(
                    target: "ssp::view::process_batch",
                    view_id = %self.plan.id,
                    changed_count = changed_keys.len(),
                    "Streaming mode: filtered to changed records only"
                );

                changed_keys
            }
            ViewResultFormat::Flat | ViewResultFormat::Tree => {
                // Need all records for hash computation
                self.build_result_data()
            }
        };

        tracing::debug!(
            target: "ssp::view::process_batch",
            view_id = %self.plan.id,
            result_data_count = result_data.len(),
            result_sample = ?result_data.iter().take(5).collect::<Vec<_>>(),
            "Built result_data (these IDs go to StreamingUpdate)"
        );

        // Delegate formatting to update module (Strategy Pattern)
        use super::update::{build_update, RawViewResult, ViewDelta};

        // FIX: On first run, we should still provide a delta for edge creation!
        // Previously this was None, causing build_update to use raw.records as Created
        // Now we explicitly create a delta with all records as additions
        let view_delta_struct = if is_first_run {
            tracing::info!(
                target: "ssp::view::process_batch",
                view_id = %self.plan.id,
                initial_records = additions.len(),
                "First run - creating delta with all records as additions"
            );
            // On first run, all records in result are additions
            Some(ViewDelta {
                additions: additions.clone(),
                removals: vec![],
                updates: vec![],
            })
        } else {
            Some(ViewDelta {
                additions,
                removals,
                updates,
            })
        };

        let raw_result = RawViewResult {
            query_id: self.plan.id.clone(),
            records: result_data,
            delta: view_delta_struct,
        };

        // Build update using the configured format
        let update = build_update(raw_result, self.format);

        // Streaming: early exit, zero allocations
        if let ViewUpdate::Streaming(s) = &update {
            if !s.records.is_empty() {
                self.has_run = true;
                return Some(update);
            }
            return None;
        }

        // Flat/Tree: hash-based change detection
        let hash = match &update {
            ViewUpdate::Flat(flat) | ViewUpdate::Tree(flat) => flat.result_hash.clone(),
            ViewUpdate::Streaming(_) => unreachable!(),
        };

        if hash != self.last_hash {
            self.has_run = true;
            self.last_hash = hash;
            return Some(update);
        }

        None
    }

    /// Expand target set with subquery results
    ///
    /// MEMBERSHIP MODEL: After expansion, all weights are normalized to 1.
    /// OPTIMIZATION: Uses shared accumulator to avoid allocations per parent.
    fn expand_with_subqueries(&self, target_set: &mut ZSet, db: &Database) {
        if !self.has_subqueries() {
            return;
        }

        // Collect subquery results (Accumulator reused for all parents)
        let mut subquery_additions: ZSet = FastMap::default();

        let parent_records: Vec<(SmolStr, i64)> = target_set
            .iter()
            .filter(|(_, &w)| w > 0) // Only process present records
            .map(|(k, &w)| (k.clone(), w))
            .collect();

        for (parent_key, _parent_weight) in parent_records {
            // Note: We ignore parent_weight for membership model

            let parent_data = match self.get_row_value(&parent_key, db) {
                Some(data) => data,
                None => continue,
            };

            // Recursively evaluate into accumulator
            self.evaluate_subqueries_for_parent_into(
                &self.plan.root,
                parent_data,
                db,
                &mut subquery_additions,
            );
        }

        // Merge subquery results (membership: weight = 1)
        // First normalize the accumulator to ensure we only add valid members
        subquery_additions.normalize_to_membership();

        for (key, _) in subquery_additions {
            target_set.add_member(key);
        }

        tracing::debug!(
            target: "ssp::view::subquery",
            view_id = %self.plan.id,
            target_set_size = target_set.len(),
            "Expanded and normalized subqueries"
        );
    }

    /// Evaluate subqueries for a parent record into accumulator
    /// OPTIMIZATION: Pass mutable results map to avoid allocation recursion
    fn evaluate_subqueries_for_parent_into(
        &self,
        op: &Operator,
        parent_context: &SpookyValue,
        db: &Database,
        results: &mut ZSet,
    ) {
        match op {
            Operator::Project { input, projections } => {
                // First, recurse into input
                self.evaluate_subqueries_for_parent_into(input, parent_context, db, results);

                // Then evaluate projections
                for proj in projections {
                    if let Projection::Subquery {
                        alias: _,
                        plan: operator,
                    } = proj
                    {
                        // Evaluate subquery with parent context
                        let subquery_result =
                            self.eval_snapshot(operator, db, Some(parent_context));

                        for (key, weight) in subquery_result.iter() {
                            *results.entry(key.clone()).or_insert(0) += *weight;

                            // Recursively expand nested subqueries
                            if let Some(record) = self.get_row_value(key, db) {
                                self.evaluate_subqueries_for_parent_into(
                                    operator, record, db, results,
                                );
                            }
                        }
                    }
                }
            }
            Operator::Filter { input, .. } => {
                self.evaluate_subqueries_for_parent_into(input, parent_context, db, results);
            }
            Operator::Limit { input, .. } => {
                self.evaluate_subqueries_for_parent_into(input, parent_context, db, results);
            }
            Operator::Join { left, right, .. } => {
                self.evaluate_subqueries_for_parent_into(left, parent_context, db, results);
                self.evaluate_subqueries_for_parent_into(right, parent_context, db, results);
            }
            Operator::Scan { .. } => {
                // Leaf node, no subqueries here
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
            tracing::debug!(
                target: "ssp::view::delta",
                view_id = %self.plan.id,
                "First run - using full scan"
            );
            // First run: full scan and diff
            self.compute_full_diff(db)
        } else {
            // Try incremental evaluation first
            if let Some(delta) =
                self.eval_delta_batch(&self.plan.root, deltas, db, self.params.as_ref())
            {
                tracing::debug!(
                    target: "ssp::view::delta",
                    view_id = %self.plan.id,
                    delta_size = delta.len(),
                    "Incremental eval succeeded"
                );
                delta
            } else {
                tracing::warn!(
                    target: "ssp::view::delta",
                    view_id = %self.plan.id,
                    cache_size = self.cache.len(),
                    "Incremental eval failed (subquery/join/limit?) - falling back to FULL SCAN"
                );
                // Fallback to full scan
                self.compute_full_diff(db)
            }
        }
    }

    /// Compute full diff using membership semantics
    fn compute_full_diff(&self, db: &Database) -> ZSet {
        use crate::engine::types::ZSetMembershipOps;

        // Compute target state
        let mut target_set = self
            .eval_snapshot(&self.plan.root, db, self.params.as_ref())
            .into_owned();

        // Expand with subqueries (will normalize weights)
        self.expand_with_subqueries(&mut target_set, db);

        tracing::debug!(
            target: "ssp::view::delta",
            view_id = %self.plan.id,
            target_set_size = target_set.len(),
            cache_size = self.cache.len(),
            target_sample = ?target_set.keys().take(3).collect::<Vec<_>>(),
            cache_sample = ?self.cache.keys().take(3).collect::<Vec<_>>(),
            "compute_full_diff: membership comparison"
        );

        // Compute diff using new optimized method
        let mut diff_set = FastMap::default();
        self.cache.membership_diff_into(&target_set, &mut diff_set);

        // Convert the diff set into delta for processing
        // Since diff_set keys are owned SmolStr, we move them into result map
        // This avoids one allocation compared to the Vec logic
        // But compute_view_delta expects &FastMap<String, ZSet>, we are in full diff mode so we fake it?
        // Wait, compute_full_diff returns ZSet.
        // The original logic returned ZSet from additions/removals.
        // We can just return the populated diff_set!

        tracing::debug!(
            target: "ssp::view::delta",
            view_id = %self.plan.id,
            diff_additions = diff_set.values().filter(|&&w| w > 0).count(),
            diff_removals = diff_set.values().filter(|&&w| w < 0).count(),
            "compute_full_diff: result"
        );

        diff_set
    }

    /// Apply delta to cache using MEMBERSHIP semantics
    ///
    /// Key difference from DBSP:
    /// - Weights are normalized to 1 (present) or removed (absent)
    /// - This ensures one edge per (view, record) pair
    fn apply_cache_delta(&mut self, delta: &ZSet) {
        let cache_before = self.cache.len();

        // Use membership-aware delta application
        self.cache.apply_membership_delta(delta);

        tracing::debug!(
            target: "ssp::view::cache",
            view_id = %self.plan.id,
            cache_before = cache_before,
            cache_after = self.cache.len(),
            delta_size = delta.len(),
            "Cache updated with membership delta"
        );
    }

    /// Categorize changes into additions, removals, and updates based on MEMBERSHIP transitions
    fn categorize_changes(
        &self,
        view_delta: &ZSet,
        updated_record_ids: &[SmolStr],
    ) -> (Vec<SmolStr>, Vec<SmolStr>, Vec<SmolStr>) {
        let mut additions = Vec::new();
        let mut removals = Vec::new();

        for (key, &weight_delta) in view_delta {
            let is_currently_member = self.cache.is_member(key);
            let will_be_member = {
                let old_weight = self.cache.get(key).copied().unwrap_or(0);
                old_weight + weight_delta > 0
            };

            match (is_currently_member, will_be_member) {
                (false, true) => {
                    // Entering view
                    additions.push(key.clone());
                }
                (true, false) => {
                    // Leaving view
                    removals.push(key.clone());
                }
                _ => {
                    // No membership change (staying in or staying out)
                }
            }
        }

        // Batch logging for entering/leaving records
        if !additions.is_empty() {
            tracing::trace!(
                target: "ssp::view::membership",
                view_id = %self.plan.id,
                count = additions.len(),
                sample = ?additions.iter().take(3).collect::<Vec<_>>(),
                "Records ENTERING view"
            );
        }
        if !removals.is_empty() {
            tracing::trace!(
                target: "ssp::view::membership",
                view_id = %self.plan.id,
                count = removals.len(),
                sample = ?removals.iter().take(3).collect::<Vec<_>>(),
                "Records LEAVING view"
            );
        }

        // Content updates: keys that are members AND have content changes AND not leaving
        let removal_set: std::collections::HashSet<&str> =
            removals.iter().map(|s| s.as_str()).collect();

        let updates: Vec<SmolStr> = updated_record_ids
            .iter()
            .filter(|key| self.cache.is_member(key) && !removal_set.contains(key.as_str()))
            .cloned()
            .collect();

        tracing::debug!(
            target: "ssp::view::categorize",
            view_id = %self.plan.id,
            additions = additions.len(),
            removals = removals.len(),
            updates = updates.len(),
            "Categorized membership changes"
        );

        (additions, removals, updates)
    }

    /// Build sorted result data from current cache
    ///
    /// For Streaming mode, sorting is optional since we only emit deltas.
    /// For Flat/Tree modes, sorting is required for consistent hashing.
    #[inline]
    fn build_result_data(&self) -> Vec<SmolStr> {
        let mut result_data: Vec<SmolStr> = self.cache.keys().cloned().collect();
        // Only sort for Flat/Tree (needed for hash consistency)
        // Streaming emits deltas, order doesn't matter
        if !matches!(self.format, ViewResultFormat::Streaming) {
            result_data.sort_unstable();
        }
        result_data
    }

    /// Get content updates that affect records in this view
    fn get_content_updates_in_view(&self, batch_deltas: &BatchDeltas) -> Vec<SmolStr> {
        let mut updates = Vec::new();

        for (_table, keys) in &batch_deltas.content_updates {
            for key in keys {
                if self.cache.contains_key(key.as_str()) {
                    updates.push(key.clone());
                }
            }
        }

        updates
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
                        if check_predicate(&self, predicate, &key, db, context) {
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
            }

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
                        if check_predicate(&self, predicate, key, db, context) {
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
                                return if ord.direction.eq_ignore_ascii_case(sort_direction::DESC) {
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
                // Map: Hash of Join-Field -> List of (Key, Weight, FieldValue)
                let mut right_index: FastMap<u64, Vec<(&SmolStr, &i64, &SpookyValue)>> =
                    FastMap::default();

                for (r_key, r_weight) in s_right.as_ref() {
                    if let Some(r_val) = self.get_row_value(r_key.as_str(), db) {
                        if let Some(r_field) = resolve_nested_value(Some(r_val), &on.right_field) {
                            let hash = hash_spooky_value(r_field);
                            right_index
                                .entry(hash)
                                .or_default()
                                .push((r_key, r_weight, r_field));
                        }
                    }
                }

                // 2. PROBE PHASE: Iterate Left and lookup Right (O(1))
                for (l_key, l_weight) in s_left.as_ref() {
                    if let Some(l_val) = self.get_row_value(l_key.as_str(), db) {
                        if let Some(l_field) = resolve_nested_value(Some(l_val), &on.left_field) {
                            let hash = hash_spooky_value(l_field);

                            // Hash Lookup + Verification
                            if let Some(matches) = right_index.get(&hash) {
                                for (_r_key, r_weight, r_field) in matches {
                                    // Verify actual equality!
                                    if compare_spooky_values(Some(l_field), Some(*r_field))
                                        == Ordering::Equal
                                    {
                                        let w = l_weight * *r_weight;
                                        *out.entry(l_key.clone()).or_insert(0) += w;
                                    }
                                }
                            }
                        }
                    }
                }
                Cow::Owned(out)
            }
        }
    }

    /// Get row value from database by ZSet key
    ///
    /// OPTIMIZATION: Avoid allocation by trying raw ID first.
    /// If your DB consistently uses one format, simplify this.
    #[inline]
    pub fn get_row_value<'a>(&self, key: &str, db: &'a Database) -> Option<&'a SpookyValue> {
        let (table_name, id) = parse_zset_key(key)?;
        let table = db.tables.get(table_name)?;

        // Fast path: Try raw ID (most common case)
        if let Some(row) = table.rows.get(id) {
            return Some(row);
        }

        // Slow path: Try with table prefix
        // TODO: Normalize row key format at ingestion to eliminate this branch
        // For now, use a static buffer pattern to reduce allocations

        // Check if the key format matches "table:id" where id doesn't have prefix
        // If so, the row might be stored with the full key
        if !id.contains(':') {
            // ID doesn't have prefix, try reconstructing
            // This allocation is unavoidable without changing ingestion
            let prefixed = format!("{}:{}", table_name, id);
            return table.rows.get(prefixed.as_str());
        }

        None
    }
}

//unity test in weight_correction_test.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_first_run_emits_additions() {
        // Setup: Create view with empty cache
        let plan = QueryPlan {
            id: "test".to_string(),
            root: Operator::Scan {
                table: "users".to_string(),
            },
        };
        let mut view = View::new(plan, None, Some(ViewResultFormat::Streaming));

        // Setup: Create database with one record
        let mut db = Database::new();
        let table = db.ensure_table("users");
        table.rows.insert(SmolStr::new("1"), SpookyValue::Null);
        table.zset.insert(SmolStr::new("users:1"), 1);

        // Act: Process empty batch (simulates first run on registration)
        let result = view.process_batch(&BatchDeltas::new(), &db);

        // Assert: Should return StreamingUpdate with Created event
        assert!(result.is_some());
        if let Some(ViewUpdate::Streaming(update)) = result {
            assert_eq!(update.records.len(), 1);
            if let Some(record) = update.records.first() {
                use crate::engine::update::DeltaEvent;
                assert!(matches!(record.event, DeltaEvent::Created));
                // Verify: Record ID is the full ZSet Key (Global ID).
                assert_eq!(record.id.as_str(), "users:1");
            }
        } else {
            panic!("Expected Streaming update");
        }
    }

    #[test]
    fn test_view_serialization_roundtrip() {
        use crate::engine::operators::Predicate;
        use crate::engine::types::Path;

        // Create a view with computed flags
        let plan = QueryPlan {
            id: "test".to_string(),
            root: Operator::Filter {
                input: Box::new(Operator::Scan {
                    table: "users".to_string(),
                }),
                predicate: Predicate::Eq {
                    field: Path::new("id"),
                    value: serde_json::json!("user:1"),
                },
            },
        };
        let view = View::new(plan, None, Some(ViewResultFormat::Streaming));

        // Verify flags are set
        assert!(view.is_simple_filter);
        assert!(!view.is_simple_scan);
        assert_eq!(view.referenced_tables_cached, vec!["users"]);

        // Serialize
        let json = serde_json::to_string(&view).unwrap();

        // Deserialize
        let mut loaded: View = serde_json::from_str(&json).unwrap();

        // Flags should be default (false/empty) before initialization because they are skipped
        assert!(!loaded.is_simple_filter);
        assert!(loaded.referenced_tables_cached.is_empty());

        // Initialize
        loaded.initialize_after_deserialize();

        // Now flags should match original
        assert!(loaded.is_simple_filter);
        assert!(!loaded.is_simple_scan);
        assert_eq!(loaded.referenced_tables_cached, vec!["users"]);
    }
    #[test]
    fn test_fast_path_readd_stays_at_weight_one() {
        // 1. Setup a simple view (Scan table "user")
        let plan = QueryPlan {
            id: "view_1".to_string(),
            root: Operator::Scan {
                table: "user".into(),
            },
        };

        let mut view = View::new(plan, None, None);

        // 2. Add a record (Fast Path)
        let key = "user:123";
        view.apply_single_create(&key.into());

        assert!(view.cache.is_member(key));
        assert_eq!(view.cache.get(key), Some(&1));

        // 3. Add same record again (Fast Path Idempotency Check)
        // AFTER FIX: This should keep weight at 1
        view.apply_single_create(&key.into());

        assert!(view.cache.is_member(key));
        assert_eq!(
            view.cache.get(key),
            Some(&1),
            "Weight should remain 1 after re-add"
        );
    }
}
