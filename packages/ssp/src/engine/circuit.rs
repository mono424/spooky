use super::types::{Delta, FastMap, Operation, RowKey, SpookyValue, ZSet};
use super::view::{QueryPlan, View};
use super::update::{ViewResultFormat, ViewUpdate};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use smol_str::SmolStr;
use smallvec::SmallVec;

#[cfg(feature = "parallel")]
use rayon::prelude::*;

// --- Modules ---

pub mod types {
    use super::*;
    
    /// Index of a view in the circuit's storage.
    pub type ViewIndex = usize;
    
    /// Optimized string type for table names (inlines strings <= 23 bytes).
    pub type TableName = SmolStr;
    
    /// Optimized storage for dependency lists (inline stack allocation for <4 items).
    pub type DependencyList = SmallVec<[ViewIndex; 4]>;
    
    /// Return type for ingest_single - inline storage for ≤2 updates
    pub type ViewUpdateList = SmallVec<[ViewUpdate; 2]>;
}

pub mod dto {
    use super::types::*;
    use super::*;

    #[derive(Clone, Debug)]
    pub struct BatchEntry {
        pub table: TableName,
        pub op: Operation,
        pub id: SmolStr,
        pub data: SpookyValue,
    }

    impl BatchEntry {
        pub fn new(table: impl Into<TableName>, op: Operation, id: impl Into<SmolStr>, data: SpookyValue) -> Self {
            Self { table: table.into(), op, id: id.into(), data }
        }
        
        pub fn create(table: impl Into<TableName>, id: impl Into<SmolStr>, data: SpookyValue) -> Self {
            Self::new(table, Operation::Create, id, data)
        }

        pub fn update(table: impl Into<TableName>, id: impl Into<SmolStr>, data: SpookyValue) -> Self {
            Self::new(table, Operation::Update, id, data)
        }

        pub fn delete(table: impl Into<TableName>, id: impl Into<SmolStr>) -> Self {
            Self::new(table, Operation::Delete, id, SpookyValue::Null)
        }
    }

    #[derive(Clone, Debug)]
    pub struct LoadRecord {
        pub table: TableName,
        pub id: SmolStr,
        pub data: SpookyValue,
    }

    impl LoadRecord {
        pub fn new(table: impl Into<TableName>, id: impl Into<SmolStr>, data: SpookyValue) -> Self {
            Self { table: table.into(), id: id.into(), data }
        }
    }
}

// --- Core Implementation ---

use self::dto::*;
use self::types::*;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Table {
    pub name: TableName,
    pub zset: ZSet,
    pub rows: FastMap<RowKey, SpookyValue>,
}

impl Table {
    pub fn new(name: TableName) -> Self {
        Self {
            name,
            zset: FastMap::default(),
            rows: FastMap::default(),
        }
    }

    pub fn reserve(&mut self, additional: usize) {
        self.rows.reserve(additional);
        self.zset.reserve(additional);
    }

    pub fn apply_mutation(&mut self, op: Operation, key: SmolStr, data: SpookyValue) -> (SmolStr, i64) {
        let weight = op.weight();
        match op {
            Operation::Create | Operation::Update => { self.rows.insert(key.clone(), data); }
            Operation::Delete => { self.rows.remove(&key); }
        }

        let zset_key = build_zset_key(&self.name, &key);
        if weight != 0 {
            let entry = self.zset.entry(zset_key.clone()).or_insert(0);
            *entry += weight;
            if *entry == 0 { self.zset.remove(&zset_key); }
        }
        (zset_key, weight)
    }

    pub fn apply_delta(&mut self, delta: &ZSet) {
        for (key, weight) in delta {
            let entry = self.zset.entry(key.clone()).or_insert(0);
            *entry += weight;
            if *entry == 0 { self.zset.remove(key); }
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Database {
    // FIX: Reverted keys to String to maintain compatibility with view.rs lookups.
    // Optimization (SmolStr) is kept for Table internals and DTOs, but Database
    // map must accept &String lookups from legacy code.
    pub tables: FastMap<String, Table>,
}

impl Database {
    pub fn new() -> Self {
        Self { tables: FastMap::default() }
    }

    pub fn ensure_table(&mut self, name: &str) -> &mut Table {
        self.tables
            .entry(name.to_string())
            .or_insert_with(|| Table::new(SmolStr::new(name)))
    }
    
    pub fn get_table(&self, name: &str) -> Option<&Table> {
        self.tables.get(name)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Circuit {
    pub db: Database,
    // Using Vec<View> + manual swap_remove for O(1) removal without external crate deps
    pub views: Vec<View>,
    #[serde(skip, default)]
    pub dependency_list: FastMap<TableName, DependencyList>,
}

impl Circuit {
    pub fn new() -> Self {
        Self {
            db: Database::new(),
            views: Vec::new(),
            dependency_list: FastMap::default(),
        }
    }
}

impl Default for Circuit {
    fn default() -> Self {
        Self::new()
    }
}

impl Circuit {

    // --- Ingestion API 1: Single Record ---

    /// Future optimization: ViewUpdateList is a Vec<ViewUpdate> but with inline storage for ≤2 updates
    /// Single record ingestion - returns ALL affected view updates
    /// 
    /// # Performance
    /// - Optimized for single-record mutations
    /// - Returns `SmallVec` (no heap allocation for ≤2 updates)
    /// - Processes all dependent views

    pub fn ingest_single(
        &mut self,
        entrie: BatchEntry
    ) -> ViewUpdateList {
        let key = SmolStr::new(entrie.id);
        let (zset_key, weight) = self.db.ensure_table(entrie.table.as_str()).apply_mutation(entrie.op, key, entrie.data);

        if weight == 0 {
            return SmallVec::new();
        }

        self.ensure_dependency_list();

        let table_key = SmolStr::new(entrie.table);
        
        // Clone indices to avoid borrow conflict with self.views
        let view_indices: SmallVec<[ViewIndex; 4]> = self
            .dependency_list
            .get(&table_key)
            .map(|v| v.iter().copied().collect())
            .unwrap_or_default();

        if view_indices.is_empty() {
            return SmallVec::new();
        }

        let delta = Delta::new(table_key, zset_key, weight);
        let mut updates: ViewUpdateList = SmallVec::new();
        
        for view_idx in view_indices {
            if let Some(view) = self.views.get_mut(view_idx) {
                if let Some(update) = view.process_single(&delta, &self.db) {
                    updates.push(update);
                }
            }
        }

        updates
    }

    // --- Ingestion API 2: Batch ---

    pub fn ingest_batch(&mut self, entries: Vec<BatchEntry>) -> Vec<ViewUpdate> {
        if entries.is_empty() { return Vec::new(); }

        let mut by_table: FastMap<TableName, Vec<BatchEntry>> = FastMap::default();
        for entry in entries {
            by_table.entry(entry.table.clone()).or_default().push(entry);
        }

        let mut table_deltas: FastMap<String, ZSet> = FastMap::default();
        let mut changed_tables: Vec<TableName> = Vec::with_capacity(by_table.len());

        // Parallel Storage Phase
        #[cfg(all(feature = "parallel", not(target_arch = "wasm32")))]
        {
            // P2.1: Ensure tables exist sequentially
            for name in by_table.keys() {
                self.db.ensure_table(name.as_str());
            }

            // P2.2: Parallel Delta Computation
            // We iterate via database tables (String keys) but filter by our batch (SmolStr keys)
            let results: Vec<(String, ZSet)> = self.db.tables
                .par_iter_mut()
                .filter_map(|(name, table)| {
                    // Check if this table is in our batch (requires SmolStr conversion for lookup)
                    let name_smol = SmolStr::new(name);
                    let entries = by_table.get(&name_smol)?;
                    
                    let mut delta = ZSet::default();
                    for entry in entries {
                        let (zset_key, weight) = table.apply_mutation(entry.op, entry.id.clone(), entry.data.clone());
                        if weight != 0 {
                            *delta.entry(zset_key).or_insert(0) += weight;
                        }
                    }
                    delta.retain(|_, w| *w != 0);
                    
                    if !delta.is_empty() {
                        Some((name.clone(), delta))
                    } else {
                        None
                    }
                })
                .collect();

            for (name_str, delta) in results {
                let smol_name = SmolStr::new(&name_str);
                table_deltas.insert(name_str, delta);
                changed_tables.push(smol_name);
            }
        }

        // Sequential Fallback
        #[cfg(any(target_arch = "wasm32", not(feature = "parallel")))]
        {
            for (table_name, table_entries) in by_table {
                let tb = self.db.ensure_table(table_name.as_str());
                let delta = table_deltas.entry(table_name.to_string()).or_default();

                for entry in table_entries {
                    let (zset_key, weight) = tb.apply_mutation(entry.op, entry.id, entry.data);
                    if weight != 0 {
                        *delta.entry(zset_key).or_insert(0) += weight;
                    }
                }
                delta.retain(|_, w| *w != 0);
                if !delta.is_empty() {
                    changed_tables.push(table_name);
                }
            }
        }

        self.propagate_deltas(&table_deltas, &changed_tables)
    }

    // --- Ingestion API 3: Init Load ---

    pub fn init_load(&mut self, records: impl IntoIterator<Item = LoadRecord>) {
        for record in records {
            let tb = self.db.ensure_table(record.table.as_str());
            let zset_key = build_zset_key(&record.table, &record.id);
            tb.rows.insert(record.id, record.data);
            tb.zset.insert(zset_key, 1);
        }
    }

    pub fn init_load_grouped(
        &mut self,
        by_table: impl IntoIterator<Item = (TableName, Vec<(SmolStr, SpookyValue)>)>,
    ) {
        for (table_name, records) in by_table {
            let tb = self.db.ensure_table(table_name.as_str());
            tb.reserve(records.len());
            for (id, data) in records {
                let zset_key = build_zset_key(&table_name, &id);
                tb.rows.insert(id, data);
                tb.zset.insert(zset_key, 1);
            }
        }
    }

    // --- Propagation Logic ---

    fn propagate_deltas(
        &mut self,
        table_deltas: &FastMap<String, ZSet>,
        changed_tables: &[TableName],
    ) -> Vec<ViewUpdate> {
        self.ensure_dependency_list();

        let mut impacted_view_indices: Vec<ViewIndex> = Vec::with_capacity(changed_tables.len() * 2);
        
        for table in changed_tables {
            if let Some(indices) = self.dependency_list.get(table) {
                impacted_view_indices.extend(indices.iter().copied());
            }
        }

        if impacted_view_indices.is_empty() { return Vec::new(); }

        impacted_view_indices.sort_unstable();
        impacted_view_indices.dedup();

        let db_ref = &self.db;

        #[cfg(all(feature = "parallel", not(target_arch = "wasm32")))]
        {
            use rayon::prelude::*;
            self.views
                .par_iter_mut()
                .enumerate()
                .filter_map(|(i, view)| -> Option<ViewUpdate> {
                    if impacted_view_indices.binary_search(&i).is_ok() {
                        view.process_ingest(table_deltas, db_ref) 
                    } else {
                        None
                    }
                })
                .collect()
        }

        #[cfg(any(target_arch = "wasm32", not(feature = "parallel")))]
        {
            let mut updates = Vec::with_capacity(impacted_view_indices.len());
            for i in impacted_view_indices {
                if let Some(view) = self.views.get_mut(i) {
                     if let Some(update) = view.process_ingest(table_deltas, db_ref) {
                        updates.push(update);
                    }
                }
            }
            updates
        }
    }

    // --- Management ---

    pub fn register_view(
        &mut self,
        plan: QueryPlan,
        params: Option<Value>,
        format: Option<ViewResultFormat>,
    ) -> Option<ViewUpdate> {
        if let Some(pos) = self.views.iter().position(|v| v.plan.id == plan.id) {
            self.unregister_view_by_index(pos);
        }

        let mut view = View::new(plan.clone(), params, format);

        let empty_deltas: FastMap<String, ZSet> = FastMap::default();
        let initial_update = view.process_ingest(&empty_deltas, &self.db);

        self.views.push(view);
        let view_idx = self.views.len() - 1;
        
        for t in plan.root.referenced_tables() {
            self.dependency_list
                .entry(SmolStr::new(t))
                .or_default()
                .push(view_idx);
        }

        initial_update
    }

    pub fn unregister_view(&mut self, id: &str) {
        if let Some(pos) = self.views.iter().position(|v| v.plan.id == id) {
            self.unregister_view_by_index(pos);
        }
    }

    fn unregister_view_by_index(&mut self, index: usize) {
        self.views.swap_remove(index);
        self.rebuild_dependency_list();
    }

    pub fn rebuild_dependency_list(&mut self) {
        self.dependency_list.clear();
        for (i, view) in self.views.iter().enumerate() {
            for t in view.plan.root.referenced_tables() {
                self.dependency_list
                    .entry(SmolStr::new(t))
                    .or_default()
                    .push(i);
            }
        }
    }

    #[inline]
    fn ensure_dependency_list(&mut self) {
        if self.dependency_list.is_empty() && !self.views.is_empty() {
            self.rebuild_dependency_list();
        }
    }
}

// --- Helpers ---

#[inline]
fn build_zset_key(table: &str, id: &str) -> SmolStr {
    let combined_len = table.len() + 1 + id.len();
    if combined_len <= 23 {
        let mut buf = String::with_capacity(combined_len);
        buf.push_str(table);
        buf.push(':');
        buf.push_str(id);
        SmolStr::new(buf)
    } else {
        SmolStr::new(format!("{}:{}", table, id))
    }
}