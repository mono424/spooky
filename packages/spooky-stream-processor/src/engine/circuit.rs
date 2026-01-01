use super::view::{MaterializedViewUpdate, QueryPlan, RowKey, View, ZSet};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

// --- Table & Database ---

#[derive(Clone, Debug, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct Table {
    pub name: String,
    pub zset: ZSet,
    pub rows: HashMap<RowKey, Value>,
    pub hashes: HashMap<RowKey, String>,
}

impl Table {
    pub fn new(name: String) -> Self {
        Self {
            name,
            zset: HashMap::new(),
            rows: HashMap::new(),
            hashes: HashMap::new(),
        }
    }

    pub fn update_row(&mut self, key: String, data: Value, hash: String) {
        // println!("DEBUG: Table {} update_row key={}", self.name, key);
        self.rows.insert(key.clone(), data);
        self.hashes.insert(key, hash);
    }

    pub fn delete_row(&mut self, key: &str) {
        self.rows.remove(key);
        self.hashes.remove(key);
    }

    /// Apply a delta to this table's state.
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
}

impl Circuit {
    pub fn new() -> Self {
        Self {
            db: Database::new(),
            views: Vec::new(),
        }
    }

    // THE NEW BLACK BOX METHOD
    pub fn ingest_record(
        &mut self,
        table: String,
        op: String,
        id: String,
        record: Value,
        hash: String,
    ) -> Vec<MaterializedViewUpdate> {
        let key = id;

        // 1. Calculate Delta internally
        let weight: i64 = match op.as_str() {
            "CREATE" | "UPDATE" => 1,
            "DELETE" => -1,
            _ => 0,
        };

        if weight == 0 {
            return vec![];
        }

        let mut delta = std::collections::HashMap::new();
        delta.insert(key.clone(), weight);

        // 2. Update Storage
        {
            let tb = self.db.ensure_table(&table);
            if weight > 0 {
                tb.update_row(key.clone(), record, hash);
            } else {
                tb.delete_row(&key);
            }
        } // End borrow of self.db

        // 3. Propagate
        self.step(table, delta)
    }

    pub fn register_view(
        &mut self,
        plan: QueryPlan,
        params: Option<Value>,
    ) -> Option<MaterializedViewUpdate> {
        // If view exists, remove it first (to support updates/param changes)
        if let Some(pos) = self.views.iter().position(|v| v.plan.id == plan.id) {
            self.views.remove(pos);
        }
        let mut view = View::new(plan, params);

        // Initial Hydration: Process with empty delta to force snapshot eval against current DB
        let initial_update = view.process("", &HashMap::new(), &self.db);

        self.views.push(view);
        initial_update
    }

    #[allow(dead_code)]
    pub fn unregister_view(&mut self, id: &str) {
        self.views.retain(|v| v.plan.id != id);
    }

    pub fn step(&mut self, table: String, delta: ZSet) -> Vec<MaterializedViewUpdate> {
        // 1. Update DB State (Z-Set)
        {
            let tb = self.db.ensure_table(&table);
            tb.apply_delta(&delta);
        }

        // 2. Propagate Delta to Views
        let mut updates = Vec::new();
        for i in 0..self.views.len() {
            if let Some(update) = self.views[i].process(&table, &delta, &self.db) {
                updates.push(update);
            }
        }
        updates
    }
}

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
