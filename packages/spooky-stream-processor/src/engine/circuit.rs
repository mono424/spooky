use super::view::{MaterializedViewUpdate, Operator, QueryPlan, RowKey, View, ZSet};
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use smol_str::SmolStr;
use std::collections::HashMap;

// --- Table & Database ---

#[derive(Clone, Debug, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct Table {
    pub name: String,
    pub zset: ZSet,                   // This is the fast FxHashMap
    pub rows: HashMap<RowKey, Value>, // Use standard HashMap for storage or upgrade to FxHashMap
    pub hashes: HashMap<RowKey, String>,
}

impl Table {
    pub fn new(name: String) -> Self {
        Self {
            name,
            zset: FxHashMap::default(),
            rows: HashMap::new(),
            hashes: HashMap::new(),
        }
    }

    pub fn update_row(&mut self, key: SmolStr, data: Value, hash: String) {
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

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Database {
    pub tables: HashMap<String, Table>,
}

impl Database {
    pub fn new() -> Self {
        Self {
            tables: HashMap::new(),
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
    pub dependencies: FxHashMap<String, Vec<usize>>,
}

impl Circuit {
    pub fn new() -> Self {
        Self {
            db: Database::new(),
            views: Vec::new(),
            dependencies: FxHashMap::default(),
        }
    }

    // Must be called after Deserialization to rebuild the Cache!
    pub fn rebuild_dependencies(&mut self) {
        self.dependencies.clear();
        for (i, view) in self.views.iter().enumerate() {
            let tables = extract_tables(&view.plan.root);
            for t in tables {
                self.dependencies.entry(t).or_default().push(i);
            }
        }
    }

    pub fn ingest_record(
        &mut self,
        table: String,
        op: String,
        id: String,
        record: Value,
        hash: String,
    ) -> Vec<MaterializedViewUpdate> {
        let key = SmolStr::new(id);
        let weight: i64 = match op.as_str() {
            "CREATE" | "UPDATE" => 1,
            "DELETE" => -1,
            _ => 0,
        };

        if weight == 0 {
            return vec![];
        }

        let mut delta: ZSet = FxHashMap::default();
        delta.insert(key.clone(), weight);

        // Update Storage
        {
            let tb = self.db.ensure_table(&table);
            if weight > 0 {
                tb.update_row(key.clone(), record, hash);
            } else {
                tb.delete_row(&key);
            }
        }

        self.step(table, delta)
    }

    pub fn register_view(
        &mut self,
        plan: QueryPlan,
        params: Option<Value>,
    ) -> Option<MaterializedViewUpdate> {
        if let Some(pos) = self.views.iter().position(|v| v.plan.id == plan.id) {
            self.views.remove(pos);
            // Rebuild dependencies entirely to be safe (simple but slower)
            self.rebuild_dependencies();
        }

        let mut view = View::new(plan, params);

        let empty_delta: ZSet = FxHashMap::default();
        let initial_update = view.process("", &empty_delta, &self.db);

        let view_idx = self.views.len();
        self.views.push(view);

        // Update Dependencies for the new view
        // Note: We use self.views.last() to inspect the plan we just pushed
        if let Some(v) = self.views.last() {
            let tables = extract_tables(&v.plan.root);
            for t in tables {
                self.dependencies.entry(t).or_default().push(view_idx);
            }
        }

        initial_update
    }

    #[allow(dead_code)]
    pub fn unregister_view(&mut self, id: &str) {
        self.views.retain(|v| v.plan.id != id);
        self.rebuild_dependencies();
    }

    pub fn step(&mut self, table: String, delta: ZSet) -> Vec<MaterializedViewUpdate> {
        {
            let tb = self.db.ensure_table(&table);
            tb.apply_delta(&delta);
        }

        let mut updates = Vec::new();

        // Optimization: iterate only relevant views
        // If dependencies map is empty (e.g. after fresh deserialization), we should rebuild it?
        // Or we assume the user calls rebuild_dependencies()?
        // For safety, let's check if empty and views not empty
        if self.dependencies.is_empty() && !self.views.is_empty() {
            self.rebuild_dependencies();
        }

        if let Some(indices) = self.dependencies.get(&table) {
            // We need to clone indices to avoid borrowing self.dependencies while mutably borrowing self.views
            let indices = indices.clone();
            for i in indices {
                if i < self.views.len() {
                    if let Some(update) = self.views[i].process(&table, &delta, &self.db) {
                        updates.push(update);
                    }
                }
            }
        }

        updates
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
                if let super::view::Projection::Subquery { plan, .. } = p {
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

/*
use crate::StreamProcessor;

impl StreamProcessor for Circuit {
    fn ingest_record(
        &mut self,
        table: String,
        op: String,
        id: String,
        record: Value,
        hash: String,
    ) -> Vec<MaterializedViewUpdate> {
        self.ingest_record(table, op, id, record, hash)
    }

    fn register_view(
        &mut self,
        plan: QueryPlan,
        params: Option<Value>,
    ) -> Option<MaterializedViewUpdate> {
        self.register_view(plan, params)
    }

    fn unregister_view(&mut self, id: &str) {
        self.unregister_view(id)
    }
}
*/
