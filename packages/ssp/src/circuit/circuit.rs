use crate::algebra::ZSet;
use crate::circuit::graph::Graph;
use crate::circuit::store::{ChangeSet, Operation, Record, Store};
use crate::circuit::view::{OutputFormat, View};
use crate::operator::QueryPlan;
use crate::types::{make_key, raw_id, Sp00kyValue};
use std::collections::{BTreeMap, HashMap};

/// Operation type for a subquery record delta.
#[derive(Debug, Clone, PartialEq)]
pub enum SubqueryOp {
    Add,
    Update,
    Remove,
}

/// A single subquery record change to be reflected in `_00_list_ref`.
#[derive(Debug, Clone)]
pub struct SubqueryDeltaItem {
    /// The subquery record key (e.g., "comment:abc123").
    pub id: String,
    /// The parent data record key (e.g., "thread:xyz789").
    pub parent_key: String,
    /// The relationship alias (e.g., "comments").
    pub alias: String,
    /// The operation: add, update, or remove.
    pub op: SubqueryOp,
}

/// Output from a materialized view after a step.
#[derive(Debug, Clone)]
pub struct ViewDelta {
    pub query_id: String,
    /// Keys added to the view.
    pub additions: Vec<String>,
    /// Keys removed from the view.
    pub removals: Vec<String>,
    /// Keys whose content changed but remain in the view.
    pub updates: Vec<String>,
    /// All keys currently in the view (for flat/tree modes).
    pub records: Vec<String>,
    /// Hash of the current view state.
    pub result_hash: String,
    /// Subquery record changes (additions/updates/removals for child records).
    pub subquery_items: Vec<SubqueryDeltaItem>,
    /// Authenticated user that owns the originating registration, in
    /// record-id form (e.g. `"user:abc"`). Carried so the SSP server can
    /// route per-user `_00_list_ref_user_<id>` writes without an extra
    /// DB round-trip per delta. Empty when no `$auth.id` was present at
    /// registration time.
    pub auth_id: String,
}

/// The DBSP incremental computation circuit.
///
/// Maintains a set of base collections (tables) and registered queries.
/// When input changes arrive via `step()`, the circuit incrementally
/// updates all affected materialized views and returns their deltas.
pub struct Circuit {
    pub store: Store,
    /// One operator DAG per registered query.
    graphs: HashMap<String, Graph>,
    /// View output state per query.
    views: HashMap<String, View>,
    /// Routing: table_name → [query_id].
    dependency_map: HashMap<String, Vec<String>>,
    /// Per-table raw `PERMISSIONS FOR select WHERE <expr>` text, loaded from
    /// SurrealDB at boot. The registration pipeline routes each scan's
    /// permission through the same converter that handles user queries and
    /// AND-folds the result into the scan's filter.
    permissions: HashMap<String, String>,
}

/// Compute the full set of subquery records visible through the current view.
///
/// Returns: child_key → (parent_key, alias)
/// This operates as a side-channel alongside the main Z-set pipeline.
fn compute_current_subquery_set(
    store: &Store,
    view: &View,
) -> HashMap<String, (String, String)> {
    let mut result = HashMap::new();

    let subquery_infos = view.plan.root.subquery_projection_info();

    // Pass 1: Root-level subqueries (parent_table = None) — parent is in view.cache
    for (alias, subquery_table, parent_key_opt, parent_table) in &subquery_infos {
        if parent_table.is_some() {
            continue;
        }
        let parent_key = match parent_key_opt {
            Some(pk) => pk,
            None => continue,
        };

        let collection = match store.get_collection(subquery_table) {
            Some(c) => c,
            None => continue,
        };

        // Reverse one-to-one: predicate is `child.id = $parent.<fk>`, so the
        // parent stores the child's id. Iterate parents in the view and follow
        // the FK to the child — the forward iteration below would never match
        // because user.id ("user:xxx") isn't a key in a parent (e.g. thread) cache.
        if parent_key.child_field == "id" {
            for parent_full_key in view.cache.keys() {
                let parent_row = match store.get_row_by_key(parent_full_key) {
                    Some(r) => r,
                    None => continue,
                };
                let child_full_key = match parent_row
                    .get(&parent_key.parent_field)
                    .and_then(|v| v.as_str())
                {
                    Some(v) => v,
                    None => continue,
                };
                let child_raw = raw_id(child_full_key);
                if collection.rows.contains_key(child_raw) {
                    let child_key = make_key(subquery_table, child_raw);
                    result.insert(child_key, (parent_full_key.clone(), alias.clone()));
                }
            }
            continue;
        }

        for (child_raw_id, row_data) in &collection.rows {
            let fk_value = match row_data.get(&parent_key.child_field).and_then(|v| v.as_str()) {
                Some(v) => v,
                None => continue,
            };

            if view.cache.contains_key(fk_value) {
                let child_key = make_key(subquery_table, child_raw_id);
                result.insert(child_key, (fk_value.to_string(), alias.clone()));
            }
        }
    }

    // Pass 2: Nested subqueries (parent_table = Some) — parent is a subquery item
    for (alias, subquery_table, parent_key_opt, parent_table_opt) in &subquery_infos {
        let pt = match parent_table_opt {
            Some(pt) => pt,
            None => continue,
        };
        let parent_key = match parent_key_opt {
            Some(pk) => pk,
            None => continue,
        };

        let collection = match store.get_collection(subquery_table) {
            Some(c) => c,
            None => continue,
        };
        let parent_coll = match store.get_collection(pt) {
            Some(c) => c,
            None => continue,
        };

        // Build index: parent's parent_field value → parent full key
        // Only for parent rows already in the result set (level-1 items)
        let mut parent_field_index: HashMap<String, String> = HashMap::new();
        for (parent_raw_id, parent_row_data) in &parent_coll.rows {
            let parent_full_key = make_key(pt, parent_raw_id);
            if result.contains_key(&parent_full_key) {
                if let Some(val) = parent_row_data
                    .get(&parent_key.parent_field)
                    .and_then(|v| v.as_str())
                {
                    parent_field_index.insert(val.to_string(), parent_full_key);
                }
            }
        }

        // For each child row, check if child.child_field matches a parent's field value
        for (child_raw_id, row_data) in &collection.rows {
            let child_value =
                match row_data.get(&parent_key.child_field).and_then(|v| v.as_str()) {
                    Some(v) => v,
                    None => continue,
                };
            if let Some(parent_full_key) = parent_field_index.get(child_value) {
                let child_key = make_key(subquery_table, child_raw_id);
                result.insert(child_key, (parent_full_key.clone(), alias.clone()));
            }
        }
    }

    result
}

/// Diff two subquery sets and produce delta items.
fn diff_subquery_sets(
    old: &HashMap<String, (String, String)>,
    new: &HashMap<String, (String, String)>,
    store: &Store,
) -> Vec<SubqueryDeltaItem> {
    let mut items = Vec::new();

    // Additions: in new but not old
    for (key, (parent_key, alias)) in new {
        if !old.contains_key(key) {
            items.push(SubqueryDeltaItem {
                id: key.clone(),
                parent_key: parent_key.clone(),
                alias: alias.clone(),
                op: SubqueryOp::Add,
            });
        }
    }

    // Removals: in old but not new
    for (key, (parent_key, alias)) in old {
        if !new.contains_key(key) {
            items.push(SubqueryDeltaItem {
                id: key.clone(),
                parent_key: parent_key.clone(),
                alias: alias.clone(),
                op: SubqueryOp::Remove,
            });
        }
    }

    // Updates: in both, check if version changed
    for (key, (parent_key, alias)) in new {
        if old.contains_key(key) {
            // Check if the record version changed
            let old_version = store.get_record_version_by_key(key);
            // We always emit an update for records that exist in both sets
            // when there's any change to subquery tables (the caller determines when to recompute)
            if old_version.is_some() {
                items.push(SubqueryDeltaItem {
                    id: key.clone(),
                    parent_key: parent_key.clone(),
                    alias: alias.clone(),
                    op: SubqueryOp::Update,
                });
            }
        }
    }

    items
}

impl Circuit {
    /// Create an empty circuit.
    pub fn new() -> Self {
        Self {
            store: Store::new(),
            graphs: HashMap::new(),
            views: HashMap::new(),
            dependency_map: HashMap::new(),
            permissions: HashMap::new(),
        }
    }

    /// Read-only access to the per-table permission text (for query rewriting).
    pub fn permissions(&self) -> &HashMap<String, String> {
        &self.permissions
    }

    /// Register a table's raw `PERMISSIONS FOR select WHERE <expr>` text.
    /// Called once per table at boot time after `INFO FOR DB` is parsed.
    pub fn set_permission(&mut self, table: impl Into<String>, where_text: impl Into<String>) {
        self.permissions.insert(table.into(), where_text.into());
    }

    /// Bulk-load initial data into base collections.
    pub fn load(&mut self, records: impl IntoIterator<Item = Record>) {
        for record in records {
            let coll = self.store.ensure_collection(&record.table);
            let key = make_key(&record.table, &record.id);
            let normalized = crate::types::raw_id(&record.id);
            coll.rows.insert(normalized.to_string(), record.data);
            coll.zset.insert(key, 1);
        }
    }

    /// Register a query. Builds the operator DAG, runs initial evaluation,
    /// and returns the first ViewDelta (if data exists). The owning user
    /// id is left empty; callers that need per-user table routing in
    /// `RefMode::Dedicated` should use [`add_query_with_auth`] instead.
    pub fn add_query(
        &mut self,
        plan: QueryPlan,
        params: Option<serde_json::Value>,
        format: Option<OutputFormat>,
    ) -> Option<ViewDelta> {
        self.add_query_with_auth(plan, params, format, String::new())
    }

    /// Like [`add_query`] but stashes `auth_id` on the resulting `View`
    /// so subsequent `step()` deltas know their owning user, which the
    /// SSP server uses to route `_00_list_ref_user_<id>` writes without
    /// an extra DB lookup per delta.
    pub fn add_query_with_auth(
        &mut self,
        plan: QueryPlan,
        params: Option<serde_json::Value>,
        format: Option<OutputFormat>,
        auth_id: String,
    ) -> Option<ViewDelta> {
        let query_id = plan.id.clone();
        let referenced_tables = plan.root.referenced_tables();
        let format = format.unwrap_or_default();
        let params_sv = params.map(Sp00kyValue::from);

        // Build the operator DAG
        let graph = Graph::from_plan(&plan.root);

        // Create view state
        let view = View::new(
            query_id.clone(),
            plan.clone(),
            format,
            params_sv,
            referenced_tables.clone(),
            auth_id,
        );

        self.graphs.insert(query_id.clone(), graph);
        self.views.insert(query_id.clone(), view);

        // Update dependency map
        for table in &referenced_tables {
            self.dependency_map
                .entry(table.clone())
                .or_default()
                .push(query_id.clone());
        }

        // Run initial snapshot evaluation
        self.run_initial_snapshot(&query_id)
    }

    /// Remove a registered query.
    pub fn remove_query(&mut self, query_id: &str) {
        self.graphs.remove(query_id);
        self.views.remove(query_id);

        // Clean up dependency map
        for (_, query_ids) in self.dependency_map.iter_mut() {
            query_ids.retain(|id| id != query_id);
        }
        self.dependency_map.retain(|_, ids| !ids.is_empty());
    }

    /// Advance the circuit by one time step.
    pub fn step(&mut self, changes: ChangeSet) -> Vec<ViewDelta> {
        if changes.changes.is_empty() {
            return vec![];
        }

        // Phase 1: Apply changes to store and build per-table deltas
        let mut table_deltas: HashMap<String, ZSet> = HashMap::new();
        let mut changed_tables: Vec<String> = Vec::new();
        // Track content-only updates (Operation::Update has weight 0)
        let mut content_updates: HashMap<String, Vec<String>> = HashMap::new();

        for change in &changes.changes {
            let (key, weight) = self.store.apply_change(change);
            if weight != 0 {
                let delta = table_deltas.entry(change.table.clone()).or_default();
                *delta.entry(key).or_insert(0) += weight;
            }
            // Track content updates (data changed but membership unchanged)
            if change.op == Operation::Update {
                let key = make_key(&change.table, &change.id);
                content_updates
                    .entry(change.table.clone())
                    .or_default()
                    .push(key);
            }
            if !changed_tables.contains(&change.table) {
                changed_tables.push(change.table.clone());
            }
        }

        // Clean up zero weights in deltas
        for delta in table_deltas.values_mut() {
            delta.retain(|_, w| *w != 0);
        }

        // Phase 2: Determine affected queries
        let mut affected_queries: Vec<String> = Vec::new();
        for table in &changed_tables {
            if let Some(query_ids) = self.dependency_map.get(table) {
                for qid in query_ids {
                    if !affected_queries.contains(qid) {
                        affected_queries.push(qid.clone());
                    }
                }
            }
        }

        // Phase 3: Step each affected query's DAG
        let mut results = Vec::new();
        for query_id in affected_queries {
            if let Some(delta) = self.step_query(&query_id, &table_deltas, &content_updates) {
                results.push(delta);
            }
        }

        results
    }

    /// Get a reference to a view's state.
    pub fn get_view(&self, query_id: &str) -> Option<&View> {
        self.views.get(query_id)
    }

    /// Run initial evaluation for a newly registered query.
    ///
    /// Uses `step()` so that stateful operators (TopK, Join, Aggregate,
    /// Distinct) prime their internal buffers. For Scan nodes, the full
    /// collection Z-set is injected as the initial "delta from empty".
    fn run_initial_snapshot(&mut self, query_id: &str) -> Option<ViewDelta> {
        let graph = self.graphs.get_mut(query_id)?;
        let view = self.views.get_mut(query_id)?;

        let num_nodes = graph.node_count();
        let mut node_outputs: Vec<Option<ZSet>> = vec![None; num_nodes];

        let topo_order: Vec<usize> = graph.topo_order().to_vec();

        for &node_id in &topo_order {
            let input_ids = graph.nodes[node_id].inputs.clone();
            let arity = graph.nodes[node_id].operator.arity();

            let output = if arity == 0 {
                // Scan node: inject the full collection as initial delta
                let table_name = graph.nodes[node_id].operator.collections();
                let full_zset = table_name
                    .first()
                    .and_then(|t| self.store.get_collection(t))
                    .map(|c| c.zset.clone())
                    .unwrap_or_default();
                graph.nodes[node_id]
                    .operator
                    .step(&[&full_zset], &self.store, view.params.as_ref())
            } else {
                let inputs: Vec<&ZSet> = input_ids
                    .iter()
                    .map(|&input_id| node_outputs[input_id].as_ref().unwrap())
                    .collect();
                graph.nodes[node_id]
                    .operator
                    .step(&inputs, &self.store, view.params.as_ref())
            };

            node_outputs[node_id] = Some(output);
        }

        let view_output = node_outputs[graph.output_node].take()?;

        if view_output.is_empty() {
            return None;
        }

        // Apply to view cache
        let additions: Vec<String> = view_output
            .iter()
            .filter(|(_, &w)| w > 0)
            .map(|(k, _)| k.clone())
            .collect();

        view.apply_delta(&view_output);
        view.last_hash = view.compute_hash();

        // Compute initial subquery record set
        let new_subquery_set = compute_current_subquery_set(&self.store, view);
        let subquery_items: Vec<SubqueryDeltaItem> = new_subquery_set
            .iter()
            .map(|(key, (parent_key, alias))| SubqueryDeltaItem {
                id: key.clone(),
                parent_key: parent_key.clone(),
                alias: alias.clone(),
                op: SubqueryOp::Add,
            })
            .collect();
        view.subquery_cache = new_subquery_set;

        let records: Vec<String> = view.cache.keys().cloned().collect();

        Some(ViewDelta {
            query_id: query_id.to_string(),
            additions,
            removals: vec![],
            updates: vec![],
            records,
            result_hash: view.last_hash.clone(),
            subquery_items,
            auth_id: view.auth_id.clone(),
        })
    }

    /// Step a single query's operator DAG with table deltas.
    fn step_query(
        &mut self,
        query_id: &str,
        table_deltas: &HashMap<String, ZSet>,
        content_updates: &HashMap<String, Vec<String>>,
    ) -> Option<ViewDelta> {
        let graph = self.graphs.get_mut(query_id)?;
        let view = self.views.get_mut(query_id)?;

        let num_nodes = graph.node_count();
        let mut node_outputs: Vec<Option<ZSet>> = vec![None; num_nodes];
        let empty_delta: ZSet = HashMap::new();

        // Clone topo order to avoid holding an immutable borrow on graph
        // while we mutably access graph.nodes[..].operator.step()
        let topo_order: Vec<usize> = graph.topo_order().to_vec();

        for &node_id in &topo_order {
            let input_ids = graph.nodes[node_id].inputs.clone();
            let arity = graph.nodes[node_id].operator.arity();

            let output = if arity == 0 {
                // Scan node: inject the table delta
                let table_name = graph.nodes[node_id].operator.collections()[0].clone();
                let delta = table_deltas.get(&table_name).unwrap_or(&empty_delta);
                graph.nodes[node_id]
                    .operator
                    .step(&[delta], &self.store, view.params.as_ref())
            } else {
                let inputs: Vec<&ZSet> = input_ids
                    .iter()
                    .map(|&input_id| node_outputs[input_id].as_ref().unwrap())
                    .collect();
                graph.nodes[node_id]
                    .operator
                    .step(&inputs, &self.store, view.params.as_ref())
            };

            node_outputs[node_id] = Some(output);
        }

        let mut view_delta = node_outputs[graph.output_node].take()?;

        // Membership re-evaluation pass for Operation::Update.
        //
        // Operation::Update has weight 0, so the Scan operator emits
        // an empty delta for the table and the Filter never sees the
        // updated row. That silently breaks cross-user realtime sync:
        // when alice publishes a thread, bob's view's permission
        // filter would NOW admit it, but the empty Scan delta means
        // no addition propagates. Symmetrically, an unpublish leaves
        // a stale row in bob's view.cache.
        //
        // Fix: for each content_update key, ask the output operator
        // "does this key satisfy the view right now?". If yes and not
        // in cache → synthesize +1 (will surface as `additions`). If
        // no and in cache → synthesize a weight that zeroes the cache
        // entry (surfaces as `removals`). The existing additions /
        // removals / updates classification below picks these up
        // naturally.
        for (_, keys) in content_updates {
            for key in keys {
                // Walk the DAG in topo order, computing evaluate_key
                // per node so the output's result reflects whether the
                // row's NEW content satisfies the view right now.
                let mut node_evals: Vec<bool> = vec![false; num_nodes];
                for &node_id in &topo_order {
                    let input_ids = graph.nodes[node_id].inputs.clone();
                    let input_evals: Vec<bool> =
                        input_ids.iter().map(|&i| node_evals[i]).collect();
                    node_evals[node_id] = graph.nodes[node_id].operator.evaluate_key(
                        key,
                        &input_evals,
                        &self.store,
                        view.params.as_ref(),
                    );
                }
                let now_matches = node_evals[graph.output_node];
                let prev_cached = view.cache.get(key).copied().unwrap_or(0);
                let in_cache = prev_cached > 0;
                if now_matches && !in_cache && !view_delta.contains_key(key) {
                    view_delta.insert(key.clone(), 1);
                } else if !now_matches && in_cache && !view_delta.contains_key(key) {
                    view_delta.insert(key.clone(), -prev_cached);
                }
            }
        }

        // Identify content-only updates: keys in the view cache whose data changed
        // but membership didn't (Operation::Update with weight 0).
        let mut updates: Vec<String> = content_updates
            .iter()
            .flat_map(|(_, keys)| keys.iter())
            .filter(|key| view.cache.contains_key(*key) && !view_delta.contains_key(*key))
            .cloned()
            .collect();

        // Detect subquery table changes: if any table referenced in a subquery
        // projection had changes, all cached parent records need re-fetching.
        if !view.subquery_tables.is_empty() {
            let has_subquery_changes = view.subquery_tables.iter().any(|t| {
                table_deltas.contains_key(t) || content_updates.contains_key(t)
            });
            if has_subquery_changes {
                view.bump_content_generation();
                for key in view.cache.keys() {
                    if !updates.contains(key) {
                        updates.push(key.clone());
                    }
                }
            }
        }

        let has_membership_changes = !view_delta.is_empty();
        let has_content_updates = !updates.is_empty();

        if !has_membership_changes && !has_content_updates {
            return None;
        }

        // Categorize membership changes before applying
        let additions: Vec<String> = view_delta
            .iter()
            .filter(|(k, &w)| w > 0 && !view.cache.contains_key(*k))
            .map(|(k, _)| k.clone())
            .collect();
        let removals: Vec<String> = view_delta
            .iter()
            .filter(|(k, &w)| {
                w < 0 && view.cache.get(*k).map(|&old| old + w <= 0).unwrap_or(false)
            })
            .map(|(k, _)| k.clone())
            .collect();

        // Apply delta to view cache
        view.apply_delta(&view_delta);
        let new_hash = view.compute_hash();

        // For content-only updates, the hash won't change (keys unchanged),
        // but we still want to emit the delta so consumers know about data changes.
        if new_hash == view.last_hash && !has_content_updates {
            return None;
        }

        if new_hash != view.last_hash {
            view.last_hash = new_hash.clone();
        }

        // Compute subquery record diffs when relevant tables changed
        let has_subquery_table_changes = view.subquery_tables.iter().any(|t| {
            table_deltas.contains_key(t) || content_updates.contains_key(t)
        });
        let subquery_items = if has_membership_changes || has_subquery_table_changes {
            let new_subquery_set = compute_current_subquery_set(&self.store, view);
            let items = diff_subquery_sets(&view.subquery_cache, &new_subquery_set, &self.store);
            view.subquery_cache = new_subquery_set;
            items
        } else {
            vec![]
        };

        let records: Vec<String> = view.cache.keys().cloned().collect();

        Some(ViewDelta {
            query_id: query_id.to_string(),
            additions,
            removals,
            updates,
            records,
            result_hash: view.last_hash.clone(),
            subquery_items,
            auth_id: view.auth_id.clone(),
        })
    }
}

// --- Serialization support ---

use serde::{Deserialize, Serialize};

/// Serializable snapshot of the circuit state.
#[derive(Serialize, Deserialize)]
struct CircuitState {
    store: Store,
    queries: Vec<QueryState>,
}

/// Serializable snapshot of a single query's state.
#[derive(Serialize, Deserialize)]
struct QueryState {
    plan: QueryPlan,
    #[serde(default)]
    params: Option<serde_json::Value>,
    format: OutputFormat,
    cache: ZSet,
    last_hash: String,
    content_generation: u64,
    #[serde(default)]
    subquery_cache: HashMap<String, (String, String)>,
    /// Owning user record id (e.g. `"user:abc"`). `default` so old
    /// snapshots without this field still deserialize (auth_id falls
    /// back to "" and the SSP routes those views to the global tables).
    #[serde(default)]
    auth_id: String,
}

impl Circuit {
    /// Serialize the circuit state to a JSON string.
    ///
    /// The operator DAG (which contains trait objects) is NOT serialized.
    /// Instead, we serialize the query plans and rebuild graphs on restore.
    pub fn save(&self) -> serde_json::Result<String> {
        let queries: Vec<QueryState> = self
            .views
            .values()
            .map(|view| QueryState {
                plan: view.plan.clone(),
                params: view.params.as_ref().map(|sv| serde_json::Value::from(sv.clone())),
                format: view.format,
                cache: view.cache.clone(),
                last_hash: view.last_hash.clone(),
                content_generation: view.content_generation,
                subquery_cache: view.subquery_cache.clone(),
                auth_id: view.auth_id.clone(),
            })
            .collect();

        let state = CircuitState {
            store: self.store.clone(),
            queries,
        };

        serde_json::to_string(&state)
    }

    /// Restore a circuit from a JSON string.
    ///
    /// Rebuilds operator DAGs from the stored query plans and
    /// restores view caches to their saved state.
    pub fn restore(json: &str) -> serde_json::Result<Self> {
        let state: CircuitState = serde_json::from_str(json)?;

        let mut circuit = Self {
            store: state.store,
            graphs: HashMap::new(),
            views: HashMap::new(),
            dependency_map: HashMap::new(),
            permissions: HashMap::new(),
        };

        for qs in state.queries {
            let query_id = qs.plan.id.clone();
            let referenced_tables = qs.plan.root.referenced_tables();
            let params_sv = qs.params.map(Sp00kyValue::from);

            // Rebuild the operator DAG from the plan
            let graph = Graph::from_plan(&qs.plan.root);

            // Restore view state
            let mut view = View::new(
                query_id.clone(),
                qs.plan,
                qs.format,
                params_sv,
                referenced_tables.clone(),
                qs.auth_id,
            );
            view.cache = qs.cache;
            view.last_hash = qs.last_hash;
            view.content_generation = qs.content_generation;
            view.subquery_cache = qs.subquery_cache;

            circuit.graphs.insert(query_id.clone(), graph);
            circuit.views.insert(query_id.clone(), view);

            // Rebuild dependency map
            for table in &referenced_tables {
                circuit
                    .dependency_map
                    .entry(table.clone())
                    .or_default()
                    .push(query_id.clone());
            }
        }

        Ok(circuit)
    }

    // --- Accessor methods ---

    /// Number of registered views.
    pub fn view_count(&self) -> usize {
        self.views.len()
    }

    /// IDs of all registered views.
    pub fn view_ids(&self) -> Vec<String> {
        self.views.keys().cloned().collect()
    }

    /// Names of all tables in the store.
    pub fn table_names(&self) -> Vec<String> {
        self.store.collections.keys().cloned().collect()
    }

    /// Per-table row counts in the in-memory store. Used by `/info` and
    /// `spky verify` to compare circuit state against the upstream snapshot.
    pub fn table_record_counts(&self) -> Vec<(String, usize)> {
        let mut out: Vec<_> = self.store.collections
            .iter()
            .map(|(name, coll)| (name.clone(), coll.rows.len()))
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    /// Per-table content hashes in the in-memory store. Bit-identical to
    /// `Replica::compute_table_hashes` on the scheduler side when the
    /// contents agree — used by SSP self-verify (`self_bootstrap`) and
    /// `spky verify` to detect drift.
    pub fn compute_table_hashes(&self) -> BTreeMap<String, String> {
        self.store
            .collections
            .iter()
            .map(|(name, coll)| {
                let pairs: Vec<(String, serde_json::Value)> = coll
                    .rows
                    .iter()
                    .map(|(id, val)| (id.clone(), serde_json::Value::from(val.clone())))
                    .collect();
                (name.clone(), ssp_protocol::snapshot_hash::hash_table(pairs))
            })
            .collect()
    }

    /// Dependency map: table → [query_ids] for debugging.
    pub fn dependency_map_dump(&self) -> &HashMap<String, Vec<String>> {
        &self.dependency_map
    }
}

impl Default for Circuit {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::algebra::ZSetOps;
    use crate::operator::plan::{OperatorPlan, OrderSpec, Projection};
    use crate::circuit::store::Change;
    use crate::types::Path;
    use serde_json::json;

    fn scan_query(id: &str, table: &str) -> QueryPlan {
        QueryPlan {
            id: id.to_string(),
            root: OperatorPlan::Scan {
                table: table.to_string(),
            },
        }
    }

    /// SELECT * FROM <table> ORDER BY <field> <dir> LIMIT <limit> START <start>
    fn limit_start_query(
        id: &str,
        table: &str,
        limit: usize,
        start: usize,
        order_field: &str,
        dir: &str,
    ) -> QueryPlan {
        QueryPlan {
            id: id.to_string(),
            root: OperatorPlan::Limit {
                input: Box::new(OperatorPlan::Scan {
                    table: table.to_string(),
                }),
                limit,
                start,
                order_by: Some(vec![OrderSpec {
                    field: Path::new(order_field),
                    direction: dir.to_string(),
                }]),
            },
        }
    }

    #[test]
    fn limit_start_window_paginates_by_offset() {
        let mut circuit = Circuit::new();
        circuit.load(vec![
            Record::new("posts", "post:1", json!({"score": 10})),
            Record::new("posts", "post:2", json!({"score": 40})),
            Record::new("posts", "post:3", json!({"score": 30})),
            Record::new("posts", "post:4", json!({"score": 20})),
        ]);

        // ORDER BY score DESC → [40, 30, 20, 10]. LIMIT 2 START 1 → [30, 20].
        circuit.add_query(limit_start_query("p1", "posts", 2, 1, "score", "DESC"), None, None);

        let view = circuit.get_view("p1").unwrap();
        assert!(view.cache.is_present("posts:3")); // score 30
        assert!(view.cache.is_present("posts:4")); // score 20
        assert!(!view.cache.is_present("posts:2")); // score 40 — skipped by START 1
        assert!(!view.cache.is_present("posts:1")); // score 10 — below the window
    }

    #[test]
    fn limit_start_window_shifts_on_insert() {
        let mut circuit = Circuit::new();
        circuit.load(vec![
            Record::new("posts", "post:1", json!({"score": 10})),
            Record::new("posts", "post:2", json!({"score": 40})),
            Record::new("posts", "post:3", json!({"score": 20})),
        ]);

        // [40, 20, 10]. LIMIT 1 START 1 → 2nd row = post:3 (score 20).
        circuit.add_query(limit_start_query("p1", "posts", 1, 1, "score", "DESC"), None, None);
        assert!(circuit.get_view("p1").unwrap().cache.is_present("posts:3"));

        // Insert score 30 → [40, 30, 20, 10]. 2nd row is now post:4 (30); post:3
        // is pushed out of the window.
        let deltas = circuit.step(ChangeSet {
            changes: vec![Change::create("posts", "post:4", json!({"score": 30}))],
        });
        assert_eq!(deltas.len(), 1);
        assert!(deltas[0].additions.contains(&"posts:4".to_string()));
        assert!(deltas[0].removals.contains(&"posts:3".to_string()));

        let view = circuit.get_view("p1").unwrap();
        assert!(view.cache.is_present("posts:4"));
        assert!(!view.cache.is_present("posts:3"));
    }

    #[test]
    fn load_and_add_query_returns_initial_delta() {
        let mut circuit = Circuit::new();
        circuit.load(vec![
            Record::new("users", "user:1", json!({"name": "alice"})),
            Record::new("users", "user:2", json!({"name": "bob"})),
        ]);

        let delta = circuit.add_query(scan_query("q1", "users"), None, None);

        assert!(delta.is_some());
        let d = delta.unwrap();
        assert_eq!(d.additions.len(), 2);
    }

    #[test]
    fn step_returns_delta_for_affected_queries() {
        let mut circuit = Circuit::new();
        circuit.load(vec![Record::new(
            "users",
            "user:1",
            json!({"name": "alice"}),
        )]);
        circuit.add_query(scan_query("q1", "users"), None, None);

        let changes = ChangeSet {
            changes: vec![Change::create("users", "user:2", json!({"name": "bob"}))],
        };
        let deltas = circuit.step(changes);

        assert_eq!(deltas.len(), 1);
        assert!(deltas[0].additions.contains(&"users:2".to_string()));
    }

    #[test]
    fn step_returns_empty_for_unaffected_queries() {
        let mut circuit = Circuit::new();
        circuit.add_query(scan_query("q1", "users"), None, None);

        let changes = ChangeSet {
            changes: vec![Change::create("posts", "post:1", json!({"title": "hi"}))],
        };
        let deltas = circuit.step(changes);

        assert!(deltas.is_empty());
    }

    #[test]
    fn remove_query_stops_producing_deltas() {
        let mut circuit = Circuit::new();
        circuit.add_query(scan_query("q1", "users"), None, None);
        circuit.remove_query("q1");

        let changes = ChangeSet {
            changes: vec![Change::create(
                "users",
                "user:1",
                json!({"name": "alice"}),
            )],
        };
        let deltas = circuit.step(changes);
        assert!(deltas.is_empty());
    }

    #[test]
    fn roundtrip_incremental_equals_snapshot() {
        let mut circuit = Circuit::new();
        circuit.add_query(scan_query("q1", "users"), None, None);

        // Step 1: add alice
        circuit.step(ChangeSet {
            changes: vec![Change::create(
                "users",
                "user:1",
                json!({"name": "alice"}),
            )],
        });

        // Step 2: add bob
        circuit.step(ChangeSet {
            changes: vec![Change::create("users", "user:2", json!({"name": "bob"}))],
        });

        // Step 3: remove alice
        circuit.step(ChangeSet {
            changes: vec![Change::delete("users", "user:1")],
        });

        // Incremental result
        let view = circuit.get_view("q1").unwrap();
        assert!(view.cache.is_present("users:2"));
        assert!(!view.cache.is_present("users:1"));

        // Fresh snapshot should agree
        let mut fresh = Circuit::new();
        fresh.load(vec![Record::new(
            "users",
            "user:2",
            json!({"name": "bob"}),
        )]);
        fresh.add_query(scan_query("q1", "users"), None, None);
        let fresh_view = fresh.get_view("q1").unwrap();

        assert_eq!(view.cache, fresh_view.cache);
    }

    // ── Subquery change detection tests ─────────────────────────────

    /// Helper: build a query with a subquery projection.
    /// SELECT * , (SELECT * FROM child_table) AS children FROM parent_table
    fn subquery_query(id: &str, parent_table: &str, child_table: &str) -> QueryPlan {
        QueryPlan {
            id: id.to_string(),
            root: OperatorPlan::Project {
                input: Box::new(OperatorPlan::Scan {
                    table: parent_table.to_string(),
                }),
                projections: vec![
                    Projection::All,
                    Projection::Subquery {
                        alias: "children".to_string(),
                        plan: Box::new(OperatorPlan::Scan {
                            table: child_table.to_string(),
                        }),
                        parent_key: None,
                    },
                ],
            },
        }
    }

    #[test]
    fn subquery_table_create_emits_content_update() {
        let mut circuit = Circuit::new();

        // Load a parent record
        circuit.load(vec![Record::new(
            "thread",
            "thread:1",
            json!({"title": "Hello"}),
        )]);

        // Register a query with a subquery on "comment"
        let delta = circuit.add_query(subquery_query("q1", "thread", "comment"), None, None);
        assert!(delta.is_some());
        let initial_hash = delta.unwrap().result_hash;

        // Create a comment (subquery table change)
        let deltas = circuit.step(ChangeSet {
            changes: vec![Change::create(
                "comment",
                "comment:1",
                json!({"text": "hi", "thread": "thread:1"}),
            )],
        });

        // Must emit a ViewDelta with content updates
        assert_eq!(deltas.len(), 1);
        let d = &deltas[0];
        assert_eq!(d.query_id, "q1");
        assert!(d.additions.is_empty(), "no membership additions");
        assert!(d.removals.is_empty(), "no membership removals");
        assert!(d.updates.contains(&"thread:1".to_string()), "parent record is content-updated");
        assert_ne!(d.result_hash, initial_hash, "hash must change");
    }

    #[test]
    fn subquery_table_delete_emits_content_update() {
        let mut circuit = Circuit::new();

        circuit.load(vec![
            Record::new("thread", "thread:1", json!({"title": "Hello"})),
            Record::new("comment", "comment:1", json!({"text": "hi"})),
        ]);

        circuit.add_query(subquery_query("q1", "thread", "comment"), None, None);

        // Delete the comment
        let deltas = circuit.step(ChangeSet {
            changes: vec![Change::delete("comment", "comment:1")],
        });

        assert_eq!(deltas.len(), 1);
        assert!(deltas[0].updates.contains(&"thread:1".to_string()));
    }

    #[test]
    fn subquery_table_update_emits_content_update() {
        let mut circuit = Circuit::new();

        circuit.load(vec![
            Record::new("thread", "thread:1", json!({"title": "Hello"})),
            Record::new("comment", "comment:1", json!({"text": "hi"})),
        ]);

        circuit.add_query(subquery_query("q1", "thread", "comment"), None, None);

        // Update the comment (Operation::Update has weight 0)
        let deltas = circuit.step(ChangeSet {
            changes: vec![Change::update(
                "comment",
                "comment:1",
                json!({"text": "updated"}),
            )],
        });

        assert_eq!(deltas.len(), 1);
        assert!(deltas[0].updates.contains(&"thread:1".to_string()));
    }

    #[test]
    fn scan_query_emits_content_update_for_in_set_row() {
        // Regression guard for the cross-session title-edit path:
        // when a row that's already in the result set has its content
        // updated (Operation::Update with weight 0), the circuit must
        // emit a ViewDelta with that row's id in `updates`. Otherwise
        // the WasmStreamUpdate.updates is empty and the client never
        // re-queries the local DB, leaving stale data in `useQuery`.
        let mut circuit = Circuit::new();

        circuit.load(vec![Record::new(
            "thread",
            "thread:1",
            json!({"title": "Hello"}),
        )]);

        let initial = circuit.add_query(scan_query("q1", "thread"), None, None);
        let initial_hash = initial.unwrap().result_hash;

        // Content-only update: title changes, row stays in result set.
        let deltas = circuit.step(ChangeSet {
            changes: vec![Change::update(
                "thread",
                "thread:1",
                json!({"title": "Renamed"}),
            )],
        });

        assert_eq!(deltas.len(), 1);
        let d = &deltas[0];
        assert!(d.additions.is_empty(), "no membership additions");
        assert!(d.removals.is_empty(), "no membership removals");
        assert!(
            d.updates.contains(&"thread:1".to_string()),
            "in-set row content update must surface in `updates`, got {:?}",
            d.updates
        );
        // result_hash doesn't change for content-only updates on a plain
        // scan because the cache keys are unchanged; that's expected and
        // is exactly why we need the `updates` field as the change signal.
        assert_eq!(d.result_hash, initial_hash);
    }

    // ── Filter re-evaluation on Operation::Update ─────────────────
    //
    // Regression guards for the cross-user realtime sync fix.
    // Operation::Update has weight 0, so the Scan operator emits an
    // empty delta and the Filter never re-evaluates the predicate.
    // Without a fix, a row that newly matches (publish) or newly
    // fails (unpublish) the filter predicate via a content change is
    // invisible to the circuit's step output.

    /// Build a `Filter(Scan(table), predicate)` plan — minimal shape
    /// for testing predicate re-evaluation on Updates.
    fn filter_scan_query(
        id: &str,
        table: &str,
        predicate: crate::operator::predicate::Predicate,
    ) -> QueryPlan {
        QueryPlan {
            id: id.to_string(),
            root: OperatorPlan::Filter {
                input: Box::new(OperatorPlan::Scan {
                    table: table.to_string(),
                }),
                predicate,
            },
        }
    }

    #[test]
    fn content_update_admits_row_into_filtered_view() {
        use crate::operator::predicate::Predicate;
        use crate::types::Path;

        let mut circuit = Circuit::new();
        circuit.load(vec![Record::new(
            "thread",
            "thread:1",
            json!({"title": "Hello", "published": false}),
        )]);

        // View admits only published threads. Initial snapshot is
        // empty because the only row has published=false.
        let plan = filter_scan_query(
            "q1",
            "thread",
            Predicate::Eq {
                field: Path::new("published"),
                value: json!(true),
            },
        );
        let initial = circuit.add_query(plan, None, None);
        assert!(
            initial.is_none(),
            "filtered view is empty at registration time"
        );

        // Alice flips published to true. Operation::Update has weight 0
        // in the table delta; before the fix, the Scan emitted an empty
        // delta and the Filter never re-evaluated. With the fix,
        // evaluate_key catches the transition.
        let deltas = circuit.step(ChangeSet {
            changes: vec![Change::update(
                "thread",
                "thread:1",
                json!({"title": "Hello", "published": true}),
            )],
        });

        assert_eq!(deltas.len(), 1, "view should be affected by publish");
        let d = &deltas[0];
        assert!(
            d.additions.contains(&"thread:1".to_string()),
            "publish must surface as an addition; got additions={:?}, updates={:?}",
            d.additions,
            d.updates
        );
        assert!(d.removals.is_empty());
    }

    #[test]
    fn content_update_removes_row_from_filtered_view() {
        use crate::operator::predicate::Predicate;
        use crate::types::Path;

        let mut circuit = Circuit::new();
        circuit.load(vec![Record::new(
            "thread",
            "thread:1",
            json!({"title": "Hello", "published": true}),
        )]);

        // View admits only published threads. Initial snapshot has the
        // row.
        let plan = filter_scan_query(
            "q1",
            "thread",
            Predicate::Eq {
                field: Path::new("published"),
                value: json!(true),
            },
        );
        let initial = circuit.add_query(plan, None, None);
        assert!(initial.is_some());

        // Alice unpublishes. Without the fix, the row stays in
        // view.cache forever because Scan emits nothing for Update.
        let deltas = circuit.step(ChangeSet {
            changes: vec![Change::update(
                "thread",
                "thread:1",
                json!({"title": "Hello", "published": false}),
            )],
        });

        assert_eq!(deltas.len(), 1, "view should be affected by unpublish");
        let d = &deltas[0];
        assert!(
            d.removals.contains(&"thread:1".to_string()),
            "unpublish must surface as a removal; got removals={:?}, updates={:?}",
            d.removals,
            d.updates
        );
        assert!(d.additions.is_empty());
    }

    #[test]
    fn content_update_no_match_change_still_emits_update() {
        // Companion guard: when an in-cache row's content changes but
        // the predicate result is unchanged (e.g. title rename on an
        // already-published thread), the row should appear in `updates`
        // (the pre-existing content-update path), not `additions` or
        // `removals`.
        use crate::operator::predicate::Predicate;
        use crate::types::Path;

        let mut circuit = Circuit::new();
        circuit.load(vec![Record::new(
            "thread",
            "thread:1",
            json!({"title": "Original", "published": true}),
        )]);

        let plan = filter_scan_query(
            "q1",
            "thread",
            Predicate::Eq {
                field: Path::new("published"),
                value: json!(true),
            },
        );
        circuit.add_query(plan, None, None);

        let deltas = circuit.step(ChangeSet {
            changes: vec![Change::update(
                "thread",
                "thread:1",
                json!({"title": "Renamed", "published": true}),
            )],
        });

        assert_eq!(deltas.len(), 1);
        let d = &deltas[0];
        assert!(d.additions.is_empty(), "no membership change expected");
        assert!(d.removals.is_empty(), "no membership change expected");
        assert!(
            d.updates.contains(&"thread:1".to_string()),
            "in-cache content change should surface in `updates`"
        );
    }

    #[test]
    fn no_subquery_means_no_spurious_updates() {
        let mut circuit = Circuit::new();

        circuit.load(vec![Record::new(
            "thread",
            "thread:1",
            json!({"title": "Hello"}),
        )]);

        // Simple scan query — no subqueries
        circuit.add_query(scan_query("q1", "thread"), None, None);

        // Change to an unrelated table should not affect this query
        let deltas = circuit.step(ChangeSet {
            changes: vec![Change::create("comment", "comment:1", json!({"text": "hi"}))],
        });

        assert!(deltas.is_empty());
    }

    #[test]
    fn empty_view_cache_no_update_on_subquery_change() {
        let mut circuit = Circuit::new();

        // Register query but load NO parent records
        circuit.add_query(subquery_query("q1", "thread", "comment"), None, None);

        // Create a comment — no parent records to update
        let deltas = circuit.step(ChangeSet {
            changes: vec![Change::create("comment", "comment:1", json!({"text": "hi"}))],
        });

        assert!(deltas.is_empty());
    }

    #[test]
    fn self_referencing_subquery_detects_changes() {
        let mut circuit = Circuit::new();

        circuit.load(vec![Record::new(
            "thread",
            "thread:root",
            json!({"title": "Root", "is_root": true}),
        )]);

        // Query: SELECT *, (SELECT * FROM thread WHERE ...) AS children FROM thread
        // "thread" is both primary AND subquery table
        circuit.add_query(subquery_query("q1", "thread", "thread"), None, None);

        // Create a child thread — this is both a membership change (new thread in Scan)
        // AND a subquery table change
        let deltas = circuit.step(ChangeSet {
            changes: vec![Change::create(
                "thread",
                "thread:child",
                json!({"title": "Child", "parent": "thread:root"}),
            )],
        });

        assert_eq!(deltas.len(), 1);
        let d = &deltas[0];
        // Should have both: membership addition (new thread) AND content update (root's subquery changed)
        assert!(d.additions.contains(&"thread:child".to_string()));
        assert!(d.updates.contains(&"thread:root".to_string()));
    }

    // ── Subquery item tracking tests ─────────────────────────────

    use crate::operator::plan::SubqueryParentKey;
    use crate::operator::predicate::Predicate;

    /// Helper: build a query with a subquery projection that has parent_key set.
    /// SELECT *, (SELECT * FROM child_table WHERE child_fk = $parent.id) AS alias FROM parent_table
    fn subquery_query_with_parent_key(
        id: &str,
        parent_table: &str,
        child_table: &str,
        alias: &str,
        child_field: &str,
    ) -> QueryPlan {
        QueryPlan {
            id: id.to_string(),
            root: OperatorPlan::Project {
                input: Box::new(OperatorPlan::Scan {
                    table: parent_table.to_string(),
                }),
                projections: vec![
                    Projection::All,
                    Projection::Subquery {
                        alias: alias.to_string(),
                        plan: Box::new(OperatorPlan::Filter {
                            input: Box::new(OperatorPlan::Scan {
                                table: child_table.to_string(),
                            }),
                            predicate: Predicate::Eq {
                                field: crate::types::Path::new(child_field),
                                value: json!({"$param": "parent.id"}),
                            },
                        }),
                        parent_key: Some(SubqueryParentKey {
                            child_field: child_field.to_string(),
                            parent_field: "id".to_string(),
                        }),
                    },
                ],
            },
        }
    }

    /// Helper: build a reverse one-to-one query (parent stores the child's id).
    /// SELECT *, (SELECT * FROM child_table WHERE id = $parent.<parent_fk> LIMIT 1)[0] AS alias FROM parent_table
    fn reverse_one_to_one_query(
        id: &str,
        parent_table: &str,
        child_table: &str,
        alias: &str,
        parent_fk: &str,
    ) -> QueryPlan {
        QueryPlan {
            id: id.to_string(),
            root: OperatorPlan::Project {
                input: Box::new(OperatorPlan::Scan {
                    table: parent_table.to_string(),
                }),
                projections: vec![
                    Projection::All,
                    Projection::Subquery {
                        alias: alias.to_string(),
                        plan: Box::new(OperatorPlan::Filter {
                            input: Box::new(OperatorPlan::Scan {
                                table: child_table.to_string(),
                            }),
                            predicate: Predicate::Eq {
                                field: crate::types::Path::new("id"),
                                value: json!({ "$param": format!("parent.{parent_fk}") }),
                            },
                        }),
                        parent_key: Some(SubqueryParentKey {
                            child_field: "id".to_string(),
                            parent_field: parent_fk.to_string(),
                        }),
                    },
                ],
            },
        }
    }

    #[test]
    fn initial_snapshot_tracks_reverse_one_to_one_subquery() {
        // Regression: top-level reverse one-to-one (e.g. thread.author -> user)
        // wasn't being tracked because pass 1 only handled child-has-FK-to-parent.
        let mut circuit = Circuit::new();

        circuit.load(vec![
            Record::new("user", "user:khadim", json!({"username": "khadim"})),
            Record::new("user", "user:lisa", json!({"username": "lisa"})),
            Record::new(
                "thread",
                "thread:1",
                json!({"title": "Hello", "author": "user:khadim"}),
            ),
            Record::new(
                "thread",
                "thread:2",
                json!({"title": "World", "author": "user:lisa"}),
            ),
        ]);

        let delta = circuit.add_query(
            reverse_one_to_one_query("q1", "thread", "user", "author", "author"),
            None,
            None,
        );

        let d = delta.expect("expected initial delta");
        assert_eq!(d.additions.len(), 2);

        let mut author_items: Vec<_> = d
            .subquery_items
            .iter()
            .filter(|i| i.op == SubqueryOp::Add && i.alias == "author")
            .collect();
        author_items.sort_by(|a, b| a.id.cmp(&b.id));
        assert_eq!(author_items.len(), 2);
        assert_eq!(author_items[0].id, "user:khadim");
        assert_eq!(author_items[0].parent_key, "thread:1");
        assert_eq!(author_items[1].id, "user:lisa");
        assert_eq!(author_items[1].parent_key, "thread:2");
    }

    #[test]
    fn step_adds_reverse_one_to_one_when_parent_added() {
        let mut circuit = Circuit::new();

        circuit.load(vec![Record::new(
            "user",
            "user:khadim",
            json!({"username": "khadim"}),
        )]);

        circuit.add_query(
            reverse_one_to_one_query("q1", "thread", "user", "author", "author"),
            None,
            None,
        );

        // New thread referencing an existing user — author must show up as Add.
        let deltas = circuit.step(ChangeSet {
            changes: vec![Change::create(
                "thread",
                "thread:1",
                json!({"title": "Hi", "author": "user:khadim"}),
            )],
        });

        assert_eq!(deltas.len(), 1);
        let adds: Vec<_> = deltas[0]
            .subquery_items
            .iter()
            .filter(|i| i.op == SubqueryOp::Add)
            .collect();
        assert_eq!(adds.len(), 1);
        assert_eq!(adds[0].id, "user:khadim");
        assert_eq!(adds[0].parent_key, "thread:1");
        assert_eq!(adds[0].alias, "author");
    }

    #[test]
    fn initial_snapshot_includes_subquery_items() {
        let mut circuit = Circuit::new();

        // Load parent + child records
        circuit.load(vec![
            Record::new("thread", "thread:1", json!({"title": "Hello"})),
            Record::new("comment", "comment:1", json!({"text": "hi", "thread": "thread:1"})),
            Record::new("comment", "comment:2", json!({"text": "yo", "thread": "thread:1"})),
        ]);

        let delta = circuit.add_query(
            subquery_query_with_parent_key("q1", "thread", "comment", "comments", "thread"),
            None,
            None,
        );

        assert!(delta.is_some());
        let d = delta.unwrap();
        assert_eq!(d.additions.len(), 1); // thread:1
        assert_eq!(d.subquery_items.len(), 2); // comment:1, comment:2
        assert!(d.subquery_items.iter().all(|item| item.op == SubqueryOp::Add));
        assert!(d.subquery_items.iter().all(|item| item.parent_key == "thread:1"));
        assert!(d.subquery_items.iter().all(|item| item.alias == "comments"));
    }

    #[test]
    fn step_adds_subquery_items_for_new_child() {
        let mut circuit = Circuit::new();

        circuit.load(vec![
            Record::new("thread", "thread:1", json!({"title": "Hello"})),
        ]);

        circuit.add_query(
            subquery_query_with_parent_key("q1", "thread", "comment", "comments", "thread"),
            None,
            None,
        );

        // Create a comment linked to thread:1
        let deltas = circuit.step(ChangeSet {
            changes: vec![Change::create(
                "comment",
                "comment:1",
                json!({"text": "hi", "thread": "thread:1"}),
            )],
        });

        assert_eq!(deltas.len(), 1);
        let d = &deltas[0];
        let adds: Vec<_> = d.subquery_items.iter().filter(|i| i.op == SubqueryOp::Add).collect();
        assert_eq!(adds.len(), 1);
        assert_eq!(adds[0].id, "comment:1");
        assert_eq!(adds[0].parent_key, "thread:1");
        assert_eq!(adds[0].alias, "comments");
    }

    #[test]
    fn step_removes_subquery_items_when_child_deleted() {
        let mut circuit = Circuit::new();

        circuit.load(vec![
            Record::new("thread", "thread:1", json!({"title": "Hello"})),
            Record::new("comment", "comment:1", json!({"text": "hi", "thread": "thread:1"})),
        ]);

        circuit.add_query(
            subquery_query_with_parent_key("q1", "thread", "comment", "comments", "thread"),
            None,
            None,
        );

        // Delete the comment
        let deltas = circuit.step(ChangeSet {
            changes: vec![Change::delete("comment", "comment:1")],
        });

        assert_eq!(deltas.len(), 1);
        let d = &deltas[0];
        let removes: Vec<_> = d.subquery_items.iter().filter(|i| i.op == SubqueryOp::Remove).collect();
        assert_eq!(removes.len(), 1);
        assert_eq!(removes[0].id, "comment:1");
    }

    #[test]
    fn step_removes_subquery_items_when_parent_removed() {
        let mut circuit = Circuit::new();

        circuit.load(vec![
            Record::new("thread", "thread:1", json!({"title": "Hello"})),
            Record::new("comment", "comment:1", json!({"text": "hi", "thread": "thread:1"})),
        ]);

        circuit.add_query(
            subquery_query_with_parent_key("q1", "thread", "comment", "comments", "thread"),
            None,
            None,
        );

        // Delete the parent thread — all child subquery items should be removed
        let deltas = circuit.step(ChangeSet {
            changes: vec![Change::delete("thread", "thread:1")],
        });

        assert_eq!(deltas.len(), 1);
        let d = &deltas[0];
        assert!(d.removals.contains(&"thread:1".to_string()));
        let removes: Vec<_> = d.subquery_items.iter().filter(|i| i.op == SubqueryOp::Remove).collect();
        assert_eq!(removes.len(), 1);
        assert_eq!(removes[0].id, "comment:1");
    }

    #[test]
    fn no_subquery_items_for_unrelated_child() {
        let mut circuit = Circuit::new();

        circuit.load(vec![
            Record::new("thread", "thread:1", json!({"title": "Hello"})),
        ]);

        circuit.add_query(
            subquery_query_with_parent_key("q1", "thread", "comment", "comments", "thread"),
            None,
            None,
        );

        // Create a comment linked to a non-existent thread
        let deltas = circuit.step(ChangeSet {
            changes: vec![Change::create(
                "comment",
                "comment:1",
                json!({"text": "hi", "thread": "thread:999"}),
            )],
        });

        // Should still emit delta (subquery table change bumps content_generation)
        // but NO subquery items since parent not in view
        assert_eq!(deltas.len(), 1);
        let adds: Vec<_> = deltas[0].subquery_items.iter().filter(|i| i.op == SubqueryOp::Add).collect();
        assert!(adds.is_empty());
    }

    // ── Nested subquery tracking tests ─────────────────────────────

    /// Helper: build a query with a nested subquery projection.
    /// SELECT *, (SELECT *, (SELECT * FROM grandchild_table WHERE id=$parent.gc_field LIMIT 1)[0] AS gc_alias
    ///   FROM child_table WHERE child_fk=$parent.id) AS child_alias FROM parent_table
    fn nested_subquery_query(
        id: &str,
        parent_table: &str,
        child_table: &str,
        child_alias: &str,
        child_fk: &str,
        grandchild_table: &str,
        grandchild_alias: &str,
        grandchild_fk_child: &str,
        grandchild_fk_parent: &str,
    ) -> QueryPlan {
        QueryPlan {
            id: id.to_string(),
            root: OperatorPlan::Project {
                input: Box::new(OperatorPlan::Scan {
                    table: parent_table.to_string(),
                }),
                projections: vec![
                    Projection::All,
                    Projection::Subquery {
                        alias: child_alias.to_string(),
                        plan: Box::new(OperatorPlan::Project {
                            input: Box::new(OperatorPlan::Filter {
                                input: Box::new(OperatorPlan::Scan {
                                    table: child_table.to_string(),
                                }),
                                predicate: Predicate::Eq {
                                    field: crate::types::Path::new(child_fk),
                                    value: json!({"$param": "parent.id"}),
                                },
                            }),
                            projections: vec![
                                Projection::All,
                                Projection::Subquery {
                                    alias: grandchild_alias.to_string(),
                                    plan: Box::new(OperatorPlan::Limit {
                                        input: Box::new(OperatorPlan::Filter {
                                            input: Box::new(OperatorPlan::Scan {
                                                table: grandchild_table.to_string(),
                                            }),
                                            predicate: Predicate::Eq {
                                                field: crate::types::Path::new(grandchild_fk_child),
                                                value: json!({"$param": "parent.id"}),
                                            },
                                        }),
                                        limit: 1,
                                        start: 0,
                                        order_by: None,
                                    }),
                                    parent_key: Some(SubqueryParentKey {
                                        child_field: grandchild_fk_child.to_string(),
                                        parent_field: grandchild_fk_parent.to_string(),
                                    }),
                                },
                            ],
                        }),
                        parent_key: Some(SubqueryParentKey {
                            child_field: child_fk.to_string(),
                            parent_field: "id".to_string(),
                        }),
                    },
                ],
            },
        }
    }

    #[test]
    fn nested_subquery_items_tracked_on_initial_snapshot() {
        let mut circuit = Circuit::new();

        // thread → comment → user (comment.author references user.id)
        circuit.load(vec![
            Record::new("thread", "thread:1", json!({"title": "Hello"})),
            Record::new("comment", "comment:1", json!({"text": "hi", "thread": "thread:1", "author": "user:alice"})),
            Record::new("user", "user:alice", json!({"name": "Alice", "id": "user:alice"})),
        ]);

        let delta = circuit.add_query(
            nested_subquery_query(
                "q1", "thread", "comment", "comments", "thread",
                "user", "author", "id", "author",
            ),
            None,
            None,
        );

        assert!(delta.is_some());
        let d = delta.unwrap();
        // Should have thread:1 in additions
        assert_eq!(d.additions.len(), 1);
        // Should track comment:1 (level-1) and user:alice (level-2) as subquery items
        let adds: Vec<_> = d.subquery_items.iter().filter(|i| i.op == SubqueryOp::Add).collect();
        assert!(adds.iter().any(|i| i.id == "comment:1" && i.alias == "comments"));
        assert!(adds.iter().any(|i| i.id == "user:alice" && i.alias == "author"));
    }

    #[test]
    fn nested_subquery_items_added_on_step() {
        let mut circuit = Circuit::new();

        circuit.load(vec![
            Record::new("thread", "thread:1", json!({"title": "Hello"})),
        ]);

        circuit.add_query(
            nested_subquery_query(
                "q1", "thread", "comment", "comments", "thread",
                "user", "author", "id", "author",
            ),
            None,
            None,
        );

        // First add a comment with an author reference
        circuit.step(ChangeSet {
            changes: vec![Change::create(
                "comment",
                "comment:1",
                json!({"text": "hi", "thread": "thread:1", "author": "user:alice"}),
            )],
        });

        // Then add the user record
        let deltas = circuit.step(ChangeSet {
            changes: vec![Change::create(
                "user",
                "user:alice",
                json!({"name": "Alice", "id": "user:alice"}),
            )],
        });

        assert_eq!(deltas.len(), 1);
        let adds: Vec<_> = deltas[0].subquery_items.iter().filter(|i| i.op == SubqueryOp::Add).collect();
        assert!(adds.iter().any(|i| i.id == "user:alice" && i.alias == "author"));
    }
}
