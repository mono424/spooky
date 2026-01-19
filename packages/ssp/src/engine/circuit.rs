use crate::debug_log;
use super::view::{
    FastMap, QueryPlan, RowKey, SpookyValue, View, ViewUpdate, ZSet,
};
use super::update::ViewResultFormat;
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Operation {
    Create,
    Update, 
    Delete,
}

impl Operation {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_ascii_uppercase().as_str() {
            "CREATE" => Some(Operation::Create),
            "UPDATE" => Some(Operation::Update),
            "DELETE" => Some(Operation::Delete),
            _ => None,
        }
    }

    pub fn weight(&self) -> i64 {
        match self {
            Operation::Create | Operation::Update => 1,
            Operation::Delete => -1,
        }
    }

    pub fn is_additive(&self) -> bool {
        matches!(self, Operation::Create | Operation::Update)
    }
}

#[derive(Clone, Debug)]
pub struct BatchEntry {
    pub table: SmolStr,
    pub op: Operation,
    pub id: SmolStr,
    pub record: SpookyValue,
    pub hash: String,
}

impl BatchEntry {
    pub fn new(table: impl Into<SmolStr>, op: Operation, id: impl Into<SmolStr>, record: SpookyValue, hash: String) -> Self {
        Self {
            table: table.into(),
            op,
            id: id.into(),
            record,
            hash,
        }
    }

    pub fn from_tuple(tuple: (String, String, String, Value, String)) -> Option<Self> {
        let (table, op_str, id, record, hash) = tuple;
        let op = Operation::from_str(&op_str)?;
        Some(Self {
            table: SmolStr::from(table),
            op,
            id: SmolStr::from(id),
            record: SpookyValue::from(record),
            hash,
        })
    }
}

pub struct IngestBatch {
    entries: Vec<BatchEntry>,
}

impl IngestBatch {
    pub fn new() -> Self {
        Self { entries: Vec::new() }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self { entries: Vec::with_capacity(capacity) }
    }

    pub fn create(mut self, table: &str, id: &str, record: SpookyValue, hash: String) -> Self {
        self.entries.push(BatchEntry {
            table: SmolStr::new(table),
            op: Operation::Create,
            id: SmolStr::new(id),
            record,
            hash,
        });
        self
    }

    pub fn update(mut self, table: &str, id: &str, record: SpookyValue, hash: String) -> Self {
        self.entries.push(BatchEntry {
            table: SmolStr::new(table),
            op: Operation::Update,
            id: SmolStr::new(id),
            record,
            hash,
        });
        self
    }

    pub fn delete(mut self, table: &str, id: &str) -> Self {
        self.entries.push(BatchEntry {
            table: SmolStr::new(table),
            op: Operation::Delete,
            id: SmolStr::new(id),
            record: SpookyValue::Null,
            hash: String::new(),
        });
        self
    }

    pub fn entry(mut self, entry: BatchEntry) -> Self {
        self.entries.push(entry);
        self
    }

    pub fn build(self) -> Vec<BatchEntry> {
        self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for IngestBatch {
    fn default() -> Self {
        Self::new()
    }
}
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
            let tables = view.plan.root.referenced_tables();
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
 
    /// Ingest using the builder pattern
    pub fn ingest(&mut self, batch: IngestBatch, is_optimistic: bool) -> Vec<ViewUpdate> {
        self.ingest_entries(batch.build(), is_optimistic)
    }

    /// Ingest pre-built entries with group-by-table optimization
    pub fn ingest_entries(&mut self, entries: Vec<BatchEntry>, is_optimistic: bool) -> Vec<ViewUpdate> {
        if entries.is_empty() {
            return Vec::new();
        }

        // Group entries by table for cache-friendly processing
        let mut by_table: FastMap<SmolStr, Vec<BatchEntry>> = FastMap::default();
        for entry in entries {
            by_table.entry(entry.table.clone()).or_default().push(entry);
        }

        let mut table_deltas: FastMap<String, ZSet> = FastMap::default();

        // Process each table's entries together (better cache locality)
        for (table, table_entries) in by_table {
            let tb = self.db.ensure_table(table.as_str());
            let delta = table_deltas.entry(table.to_string()).or_default();

            for entry in table_entries {
                let weight = entry.op.weight();

                if entry.op.is_additive() {
                    tb.update_row(entry.id.clone(), entry.record, entry.hash);
                } else {
                    tb.delete_row(&entry.id);
                }

                *delta.entry(entry.id).or_insert(0) += weight;
            }
        }

        self.propagate_deltas(table_deltas, is_optimistic)
    }

    fn propagate_deltas(
        &mut self,
        mut table_deltas: FastMap<String, ZSet>,
        is_optimistic: bool,
    ) -> Vec<ViewUpdate> {
        // Apply deltas to DB ZSets and collect changed tables
        let mut changed_tables: Vec<String> = Vec::with_capacity(table_deltas.len());

        for (table, delta) in &mut table_deltas {
            delta.retain(|_, w| *w != 0);
            if !delta.is_empty() {
                let tb = self.db.ensure_table(table.as_str());
                tb.apply_delta(delta);
                changed_tables.push(table.clone());
            }
        }

        if changed_tables.is_empty() {
            return Vec::new();
        }

        // Lazy rebuild of dependency graph
        if self.dependency_graph.is_empty() && !self.views.is_empty() {
            self.rebuild_dependency_graph();
        }

        // Collect impacted view indices
        let mut impacted_view_indices: Vec<usize> = Vec::new();
        for table in &changed_tables {
            if let Some(indices) = self.dependency_graph.get(table) {
                impacted_view_indices.extend(indices.iter().copied());
            }
        }

        // Deduplicate
        impacted_view_indices.sort_unstable();
        impacted_view_indices.dedup();

        if impacted_view_indices.is_empty() {
            return Vec::new();
        }

        // Process views
        self.process_impacted_views(&impacted_view_indices, &table_deltas, is_optimistic)
    }

    fn process_impacted_views(
        &mut self,
        indices: &[usize],
        deltas: &FastMap<String, ZSet>,
        is_optimistic: bool,
    ) -> Vec<ViewUpdate> {
        const PARALLEL_VIEW_THRESHOLD: usize = 10;
        let db_ref = &self.db;

        #[cfg(all(feature = "parallel", not(target_arch = "wasm32")))]
        {
            if indices.len() >= PARALLEL_VIEW_THRESHOLD {
                use rayon::prelude::*;
                return self.views
                    .par_iter_mut()
                    .enumerate()
                    .filter_map(|(i, view)| {
                        if indices.binary_search(&i).is_ok() {
                            view.process_ingest(deltas, db_ref, is_optimistic)
                        } else {
                            None
                        }
                    })
                    .collect();
            }
        }

        // Sequential path (small batch or WASM)
        let mut updates = Vec::with_capacity(indices.len());
        for &i in indices {
            if i < self.views.len() {
                if let Some(update) = self.views[i].process_ingest(deltas, db_ref, is_optimistic) {
                    updates.push(update);
                }
            }
        }
        updates
    }

    pub fn ingest_record(
        &mut self,
        table: &str,
        op: &str,
        id: &str,
        record: Value,
        hash: &str,
        is_optimistic: bool,
    ) -> Vec<ViewUpdate> {
        let op = match Operation::from_str(op) {
            Some(o) => o,
            None => return Vec::new(),
        };

        self.ingest_entries(
            vec![BatchEntry {
                table: SmolStr::from(table),
                op,
                id: SmolStr::from(id),
                record: SpookyValue::from(record),
                hash: hash.to_string(),
            }],
            is_optimistic,
        )
    }

    pub fn ingest_batch(
        &mut self,
        batch: Vec<(String, String, String, Value, String)>,
        is_optimistic: bool,
    ) -> Vec<ViewUpdate> {
        let entries: Vec<BatchEntry> = batch
            .into_iter()
            .filter_map(BatchEntry::from_tuple)
            .collect();
        
        self.ingest_entries(entries, is_optimistic)
    }

    pub fn ingest_batch_spooky(
        &mut self,
        batch: Vec<(SmolStr, SmolStr, SmolStr, SpookyValue, String)>,
        is_optimistic: bool,
    ) -> Vec<ViewUpdate> {
        // Compatibility wrapper for internal tests utilizing SpookyValue variants directly
         let entries: Vec<BatchEntry> = batch
            .into_iter()
            .filter_map(|(table, op_str, id, record, hash)| {
                 let op = Operation::from_str(&op_str)?;
                 Some(BatchEntry {
                    table,
                    op,
                    id,
                    record,
                    hash,
                })
            })
            .collect();
        
        self.ingest_entries(entries, is_optimistic)
    }

    pub fn register_view(
        &mut self,
        plan: QueryPlan,
        params: Option<Value>,
        format: Option<ViewResultFormat>,
    ) -> Option<ViewUpdate> {
        if let Some(pos) = self.views.iter().position(|v| v.plan.id == plan.id) {
            self.views.remove(pos);
            // Rebuild dependencies entirely to be safe (simple but slower)
            self.rebuild_dependency_graph();
        }

        let mut view = View::new(plan, params, format);

        // Trigger initial full scan by passing None to process_ingest
        // Use is_optimistic=true for initial registration
        let empty_deltas: FastMap<String, ZSet> = FastMap::default();
        let initial_update = view.process_ingest(&empty_deltas, &self.db, true);

        let view_idx = self.views.len();
        self.views.push(view);

        // Update Dependencies for the new view
        // Note: We use self.views.last() to inspect the plan we just pushed
        if let Some(v) = self.views.last() {
            let tables = v.plan.root.referenced_tables();
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

    pub fn step(&mut self, table: String, delta: ZSet, is_optimistic: bool) -> Vec<ViewUpdate> {
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
    ) -> Option<ViewUpdate> {
        if let Some(pos) = self.views.iter().position(|v| v.plan.id == incantation_id) {
            return self.views[pos].set_record_version(record_id, version, &self.db);
        }
        None
    }
}
