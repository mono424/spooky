use super::view::{
    FastMap, MaterializedViewUpdate, Operator, Projection, QueryPlan, RowKey, SpookyValue, View,
    ZSet,
};
// use rustc_hash::{FxHashMap, FxHasher}; // Unused in this file (used via FastMap)
use serde::{Deserialize, Serialize};
use serde_json::Value;
use smol_str::SmolStr;

// --- Table & Database ---

#[derive(Clone, Debug, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct Table {
    pub name: String,
    pub zset: ZSet,                         // This is the fast FxHashMap
    pub rows: FastMap<RowKey, SpookyValue>, // Using SpookyValue
    pub hashes: FastMap<RowKey, String>,
}

impl Table {
    pub fn new(name: String) -> Self {
        Self {
            name,
            zset: FastMap::default(),
            rows: FastMap::default(),
            hashes: FastMap::default(),
        }
    }

    // Changing signature to use SmolStr is implied by RowKey definition change
    pub fn update_row(&mut self, key: SmolStr, data: SpookyValue, hash: String) {
        self.rows.insert(key.clone(), data);
        self.hashes.insert(key, hash);
    }

    pub fn delete_row(&mut self, key: &SmolStr) {
        self.rows.remove(key);
        self.hashes.remove(key);
    }

    pub fn apply_delta(&mut self, delta: &ZSet) {
        for (key, weight) in delta {
            let entry = self.zset.entry(key.clone()).or_insert(0);
            *entry += weight;
            if *entry == 0 {
                self.zset.remove(key);
            }
        }
    }
}

// I will just use 'Table' name but with new types.

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Database {
    pub tables: FastMap<String, Table>,
}

impl Database {
    pub fn new() -> Self {
        Self {
            tables: FastMap::default(),
        }
    }

    pub fn ensure_table(&mut self, name: &str) -> &mut Table {
        self.tables
            .entry(name.to_string())
            .or_insert_with(|| Table::new(name.to_string()))
    }
}

// --- Circuit ---

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Circuit {
    pub db: Database,
    pub views: Vec<View>,
    // Optimisation: Mapping Table -> List of View-Indices
    #[serde(skip, default)]
    pub dependency_graph: FastMap<String, Vec<usize>>,
}

impl Circuit {
    pub fn new() -> Self {
        Self {
            db: Database::new(),
            views: Vec::new(),
            dependency_graph: FastMap::default(),
        }
    }

    // Must be called after Deserialization to rebuild the Cache!
    pub fn rebuild_dependency_graph(&mut self) {
        self.dependency_graph.clear();
        #[cfg(target_arch = "wasm32")]
        web_sys::console::log_1(
            &format!(
                "DEBUG: Rebuilding dependency graph for {} views",
                self.views.len()
            )
            .into(),
        );
        for (i, view) in self.views.iter().enumerate() {
            let tables = extract_tables(&view.plan.root);
            #[cfg(target_arch = "wasm32")]
            web_sys::console::log_1(
                &format!(
                    "DEBUG: View {} (id: {}) depends on tables: {:?}",
                    i, view.plan.id, tables
                )
                .into(),
            );
            for t in tables {
                self.dependency_graph.entry(t).or_default().push(i);
            }
        }
        #[cfg(target_arch = "wasm32")]
        web_sys::console::log_1(
            &format!("DEBUG: Final dependency graph: {:?}", self.dependency_graph).into(),
        );
    }

    pub fn ingest_record(
        &mut self,
        table: &str,
        op: &str,
        id: &str,
        record: Value,
        hash: &str,
        is_optimistic: bool,
    ) -> Vec<MaterializedViewUpdate> {
        self.ingest_batch_spooky(
            vec![(
                SmolStr::from(table),
                SmolStr::from(op),
                SmolStr::from(id),
                SpookyValue::from(record),
                hash.to_string(),
            )],
            is_optimistic,
        )
    }

    pub fn ingest_batch(
        &mut self,
        batch: Vec<(String, String, String, Value, String)>,
        is_optimistic: bool,
    ) -> Vec<MaterializedViewUpdate> {
        // Convert to SpookyValue
        let batch_spooky: Vec<(SmolStr, SmolStr, SmolStr, SpookyValue, String)> = batch
            .into_iter()
            .map(|(t, o, i, r, h)| {
                (
                    SmolStr::from(t),
                    SmolStr::from(o),
                    SmolStr::from(i),
                    SpookyValue::from(r),
                    h,
                )
            })
            .collect();

        self.ingest_batch_spooky(batch_spooky, is_optimistic)
    }

    pub fn ingest_batch_spooky(
        &mut self,
        batch: Vec<(SmolStr, SmolStr, SmolStr, SpookyValue, String)>,
        is_optimistic: bool,
    ) -> Vec<MaterializedViewUpdate> {
        let mut table_deltas: FastMap<String, ZSet> = FastMap::default();

        // 1. Storage Phase: Update Storage & Accumulate Deltas
        for (table, op, id, record_spooky, hash) in batch {
            let key = id; // Already SmolStr
            let weight: i64 = match op.as_str() {
                "CREATE" | "UPDATE" | "create" | "update" => 1,
                "DELETE" | "delete" => -1,
                _ => 0,
            };

            if weight == 0 {
                continue;
            }

            {
                let tb = self.db.ensure_table(table.as_str());
                if weight > 0 {
                    tb.update_row(key.clone(), record_spooky, hash);
                } else {
                    tb.delete_row(&key);
                }
            }

            let delta_map = table_deltas.entry(table.to_string()).or_default();
            *delta_map.entry(key).or_insert(0) += weight;
        }

        // Apply Deltas to DB ZSets
        let mut changed_tables = Vec::new();
        for (table, delta) in &mut table_deltas {
            delta.retain(|_, w| *w != 0);
            if !delta.is_empty() {
                let tb = self.db.ensure_table(table.as_str());
                tb.apply_delta(delta);
                changed_tables.push(table.to_string());
            }
        }

        // 2. Propagation Phase: Process Deltas with Dependency Graph

        // Optimized Lazy Rebuild Check (once per batch)
        if self.dependency_graph.is_empty() && !self.views.is_empty() {
            self.rebuild_dependency_graph();
        }

        // Identify ALL affected views from ALL changed tables
        let mut impacted_view_indices: Vec<usize> = Vec::new();
        #[cfg(target_arch = "wasm32")]
        web_sys::console::log_1(&format!("DEBUG: Changed tables: {:?}", changed_tables).into());
        for table in changed_tables {
            if let Some(indices) = self.dependency_graph.get(&table) {
                #[cfg(target_arch = "wasm32")]
                web_sys::console::log_1(
                    &format!("DEBUG: Table {} impacts views: {:?}", table, indices).into(),
                );
                impacted_view_indices.extend(indices.iter().copied());
            } else {
                #[cfg(target_arch = "wasm32")]
                web_sys::console::log_1(
                    &format!("DEBUG: Table {} changed, but no views depend on it", table).into(),
                );
            }
        }
        #[cfg(target_arch = "wasm32")]
        web_sys::console::log_1(
            &format!(
                "DEBUG: Total impacted view indices (before dedup): {:?}",
                impacted_view_indices
            )
            .into(),
        );

        // Deduplicate View Indices (Sort + Dedup)
        // This ensures each view is processed EXACTLY ONCE, even if multiple input tables changed
        impacted_view_indices.sort_unstable();
        impacted_view_indices.dedup();

        let mut all_updates: Vec<MaterializedViewUpdate> = Vec::new();

        // 3. Execution Phase
        // 3. Execution Phase
        let db_ref = &self.db;
        let deltas_ref = &table_deltas;

        #[cfg(all(feature = "parallel", not(target_arch = "wasm32")))]
        let updates: Vec<MaterializedViewUpdate> = {
            use rayon::prelude::*;
            self.views
                .par_iter_mut()
                .enumerate()
                .filter_map(|(i, view)| {
                    // Check if this view needs update.
                    // impacted_view_indices is sorted, so binary_search is efficient.
                    if impacted_view_indices.binary_search(&i).is_ok() {
                        view.process_ingest(deltas_ref, db_ref, is_optimistic)
                    } else {
                        None
                    }
                })
                .collect()
        };

        #[cfg(any(target_arch = "wasm32", not(feature = "parallel")))]
        let updates: Vec<MaterializedViewUpdate> = {
            let mut ups = Vec::new();
            for i in impacted_view_indices {
                if i < self.views.len() {
                    let view: &mut View = &mut self.views[i];
                    if let Some(update) = view.process_ingest(deltas_ref, db_ref, is_optimistic) {
                        ups.push(update);
                    }
                }
            }
            ups
        };

        all_updates.extend(updates);
        all_updates
    }

    pub fn register_view(
        &mut self,
        plan: QueryPlan,
        params: Option<Value>,
    ) -> Option<MaterializedViewUpdate> {
        if let Some(pos) = self.views.iter().position(|v| v.plan.id == plan.id) {
            self.views.remove(pos);
            // Rebuild dependencies entirely to be safe (simple but slower)
            self.rebuild_dependency_graph();
        }

        let mut view = View::new(plan, params);

        // Trigger initial full scan by passing None to process_ingest
        // Use is_optimistic=true for initial registration
        let empty_deltas: FastMap<String, ZSet> = FastMap::default();
        let initial_update = view.process_ingest(&empty_deltas, &self.db, true);

        let view_idx = self.views.len();
        self.views.push(view);

        // Update Dependencies for the new view
        // Note: We use self.views.last() to inspect the plan we just pushed
        if let Some(v) = self.views.last() {
            let tables = extract_tables(&v.plan.root);
            for t in tables {
                self.dependency_graph.entry(t).or_default().push(view_idx);
            }
        }

        initial_update
    }

    #[allow(dead_code)]
    pub fn unregister_view(&mut self, id: &str) {
        self.views.retain(|v| v.plan.id != id);
        self.rebuild_dependency_graph();
    }

    pub fn step(
        &mut self,
        table: String,
        delta: ZSet,
        is_optimistic: bool,
    ) -> Vec<MaterializedViewUpdate> {
        {
            let tb = self.db.ensure_table(&table);
            tb.apply_delta(&delta);
        }

        let mut updates = Vec::new();

        // Optimized Lazy Rebuild
        if self.dependency_graph.is_empty() && !self.views.is_empty() {
            self.rebuild_dependency_graph();
        }

        if let Some(indices) = self.dependency_graph.get(&table) {
            // We need to clone indices to avoid borrowing self.dependency_graph while mutably borrowing self.views
            let indices = indices.clone();
            for i in indices {
                if i < self.views.len() {
                    if let Some(update) =
                        self.views[i].process(&table, &delta, &self.db, is_optimistic)
                    {
                        updates.push(update);
                    }
                }
            }
        }

        updates
    }
    pub fn set_record_version(
        &mut self,
        incantation_id: &str,
        record_id: &str,
        version: u64,
    ) -> Option<MaterializedViewUpdate> {
        if let Some(pos) = self.views.iter().position(|v| v.plan.id == incantation_id) {
            return self.views[pos].set_record_version(record_id, version, &self.db);
        }
        None
    }
}

// Helper to find source tables in a plan
fn extract_tables(op: &Operator) -> Vec<String> {
    match op {
        Operator::Scan { table } => vec![table.clone()],
        Operator::Filter { input, .. } => extract_tables(input),
        Operator::Project { input, projections } => {
            let mut tbls = extract_tables(input);
            for p in projections {
                if let Projection::Subquery { plan, .. } = p {
                    tbls.extend(extract_tables(plan));
                }
            }
            tbls
        }
        Operator::Limit { input, .. } => extract_tables(input),
        Operator::Join { left, right, .. } => {
            let mut tbls = extract_tables(left);
            tbls.extend(extract_tables(right));
            tbls
        }
    }
}
