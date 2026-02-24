use crate::algebra::ZSet;
use crate::circuit::graph::Graph;
use crate::circuit::store::{ChangeSet, Operation, Record, Store};
use crate::circuit::view::{OutputFormat, View};
use crate::operator::QueryPlan;
use crate::types::{make_key, SpookyValue};
use std::collections::HashMap;

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
}

impl Circuit {
    /// Create an empty circuit.
    pub fn new() -> Self {
        Self {
            store: Store::new(),
            graphs: HashMap::new(),
            views: HashMap::new(),
            dependency_map: HashMap::new(),
        }
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
    /// and returns the first ViewDelta (if data exists).
    pub fn add_query(
        &mut self,
        plan: QueryPlan,
        params: Option<serde_json::Value>,
        format: Option<OutputFormat>,
    ) -> Option<ViewDelta> {
        let query_id = plan.id.clone();
        let referenced_tables = plan.root.referenced_tables();
        let format = format.unwrap_or_default();
        let params_sv = params.map(SpookyValue::from);

        // Build the operator DAG
        let graph = Graph::from_plan(&plan.root);

        // Create view state
        let view = View::new(
            query_id.clone(),
            plan.clone(),
            format,
            params_sv,
            referenced_tables.clone(),
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

        let records: Vec<String> = view.cache.keys().cloned().collect();

        Some(ViewDelta {
            query_id: query_id.to_string(),
            additions,
            removals: vec![],
            updates: vec![],
            records,
            result_hash: view.last_hash.clone(),
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

        let view_delta = node_outputs[graph.output_node].take()?;

        // Identify content-only updates: keys in the view cache whose data changed
        // but membership didn't (Operation::Update with weight 0).
        let updates: Vec<String> = content_updates
            .iter()
            .flat_map(|(_, keys)| keys.iter())
            .filter(|key| view.cache.contains_key(*key) && !view_delta.contains_key(*key))
            .cloned()
            .collect();

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
        let records: Vec<String> = view.cache.keys().cloned().collect();

        Some(ViewDelta {
            query_id: query_id.to_string(),
            additions,
            removals,
            updates,
            records,
            result_hash: view.last_hash.clone(),
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
        };

        for qs in state.queries {
            let query_id = qs.plan.id.clone();
            let referenced_tables = qs.plan.root.referenced_tables();
            let params_sv = qs.params.map(SpookyValue::from);

            // Rebuild the operator DAG from the plan
            let graph = Graph::from_plan(&qs.plan.root);

            // Restore view state
            let mut view = View::new(
                query_id.clone(),
                qs.plan,
                qs.format,
                params_sv,
                referenced_tables.clone(),
            );
            view.cache = qs.cache;
            view.last_hash = qs.last_hash;

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
    use crate::operator::plan::OperatorPlan;
    use crate::circuit::store::Change;
    use serde_json::json;

    fn scan_query(id: &str, table: &str) -> QueryPlan {
        QueryPlan {
            id: id.to_string(),
            root: OperatorPlan::Scan {
                table: table.to_string(),
            },
        }
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
}
