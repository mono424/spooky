// --- Circuit ---

use super::store::Store;
use super::view::{MaterializedViewUpdate, QueryPlan, View, ZSet};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LazyCircuit {
    pub views: Vec<View>,
}

impl LazyCircuit {
    pub fn new() -> Self {
        Self { views: Vec::new() }
    }

    // THE NEW BLACK BOX METHOD
    pub fn ingest_record(
        &mut self,
        store: &dyn Store,
        table: String,
        op: String,
        id: String,
        _record: Value, // We don't store the record anymore
        _hash: String,  // We don't store the hash anymore
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

        // 2. Propagate
        self.step(store, table, delta)
    }

    pub fn register_view(
        &mut self,
        store: &dyn Store,
        plan: QueryPlan,
        params: Option<Value>,
    ) -> Option<MaterializedViewUpdate> {
        // If view exists, remove it first (to support updates/param changes)
        if let Some(pos) = self.views.iter().position(|v| v.plan.id == plan.id) {
            self.views.remove(pos);
        }
        let mut view = View::new(plan, params);

        // Initial Hydration: Process with empty delta to force snapshot eval (if we kept snapshot logic)
        // OR better: in incremental land, "register" usually implies "catchup".
        // For now, let's assume we invoke `process` with empty delta?
        // Actually, without an Input Delta, `process` doesn't do much in incremental mode unless it does a full scan.
        // For the purpose of this refactor, let's keep the `view.process` doing a scan if delta is empty/special?
        // OR, the `View` needs to support "Initial Scan" using the Store.

        let initial_update = view.process(store, "", &HashMap::new());

        self.views.push(view);
        initial_update
    }

    #[allow(dead_code)]
    pub fn unregister_view(&mut self, id: &str) {
        self.views.retain(|v| v.plan.id != id);
    }

    pub fn step(
        &mut self,
        store: &dyn Store,
        table: String,
        delta: ZSet,
    ) -> Vec<MaterializedViewUpdate> {
        // 1. Propagate Delta to Views
        let mut updates = Vec::new();
        for i in 0..self.views.len() {
            if let Some(update) = self.views[i].process(store, &table, &delta) {
                updates.push(update);
            }
        }
        updates
    }
}

use crate::StreamProcessor;

impl StreamProcessor for LazyCircuit {
    fn ingest_record(
        &mut self,
        store: &dyn Store,
        table: String,
        op: String,
        id: String,
        record: Value,
        hash: String,
    ) -> Vec<MaterializedViewUpdate> {
        self.ingest_record(store, table, op, id, record, hash)
    }

    fn register_view(
        &mut self,
        store: &dyn Store,
        plan: QueryPlan,
        params: Option<Value>,
    ) -> Option<MaterializedViewUpdate> {
        self.register_view(store, plan, params)
    }

    fn unregister_view(&mut self, id: &str) {
        self.unregister_view(id)
    }
}
