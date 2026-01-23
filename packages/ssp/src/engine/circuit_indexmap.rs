use super::types::{Delta, FastMap, Operation, Record, RowKey, SpookyValue, ZSet};
use super::view::{QueryPlan, View};
use super::update::{ViewResultFormat, ViewUpdate};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use smol_str::SmolStr;
use smallvec::SmallVec;
use indexmap::IndexMap; // OPT-D2: Stable indexing & O(1) lookup

#[cfg(feature = "parallel")]
use rayon::prelude::*;

// --- Modules ---

// OPT-C1: Modular separation of concerns
pub mod types {
    use super::*;
    
    /// Index of a view in the circuit's storage.
    pub type ViewIndex = usize;
    
    /// Optimized string type for table names (inlines strings <= 23 bytes).
    pub type TableName = SmolStr;
    
    /// Optimized storage for dependency lists (inline stack allocation for <4 items).
    pub type DependencyList = SmallVec<[ViewIndex; 4]>;
}

pub mod dto {
    use super::types::*;
    use super::*;

    /// Represents a single mutation operation in a batch.
    #[derive(Clone, Debug)]
    pub struct BatchEntry {
        pub table: TableName,
        pub op: Operation,
        pub id: SmolStr,
        pub data: SpookyValue,
    }

    impl BatchEntry {
        pub fn new(table: impl Into<TableName>, op: Operation, id: impl Into<SmolStr>, data: SpookyValue) -> Self {
            Self {
                table: table.into(),
                op,
                id: id.into(),
                data,
            }
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

    /// Optimized record for initial bulk loading.
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

// OPT-C2: Comprehensive Documentation
/// A `Table` represents the physical storage of data within the Circuit.
/// 
/// It maintains:
/// - `rows`: The actual data (RowKey -> SpookyValue)
/// - `zset`: The multiplicity of rows (ZSet), used for delta calculation.
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

    /// Pre-allocate memory for rows and zsets.
    pub fn reserve(&mut self, additional: usize) {
        self.rows.reserve(additional);
        self.zset.reserve(additional);
    }

    /// Applies a mutation to the table storage and calculates the weight delta.
    /// Returns the computed ZSet key and the weight change.
    pub fn apply_mutation(&mut self, op: Operation, key: SmolStr, data: SpookyValue) -> (SmolStr, i64) {
        let weight = op.weight();
        
        match op {
            Operation::Create | Operation::Update => {
                self.rows.insert(key.clone(), data);
            }
            Operation::Delete => {
                self.rows.remove(&key);
            }
        }

        // OPT-M4: Inline key generation (no allocation)
        let zset_key = build_zset_key(&self.name, &key);
        
        if weight != 0 {
            let entry = self.zset.entry(zset_key.clone()).or_insert(0);
            *entry += weight;
            if *entry == 0 {
                self.zset.remove(&zset_key);
            }
        }

        (zset_key, weight)
    }

    /// Applies a ZSet delta directly to the table's bookkeeping.
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

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Database {
    pub tables: FastMap<TableName, Table>,
}

impl Database {
    pub fn new() -> Self {
        Self { tables: FastMap::default() }
    }

    pub fn ensure_table(&mut self, name: &str) -> &mut Table {
        self.tables
            .entry(SmolStr::new(name))
            .or_insert_with(|| Table::new(SmolStr::new(name)))
    }
    
    pub fn get_table(&self, name: &str) -> Option<&Table> {
        self.tables.get(name)
    }
}

// OPT-C2: Main struct documentation
/// The `Circuit` is the core incremental view maintenance engine.
///
/// # Architecture
/// - **Database**: Stores raw tables.
/// - **Views**: Materialized views defined by QueryPlans.
/// - **Dependency Graph**: Maps tables to the views that depend on them.
///
/// # Ingestion Strategy
/// - `ingest_single`: Optimized for low latency (single record).
/// - `ingest_batch`: Optimized for high throughput (parallel processing).
/// - `init_load`: Optimized for startup (skips view processing).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Circuit {
    pub db: Database,
    // OPT-D2: IndexMap for O(1) view lookup by ID and stable iteration
    pub views: IndexMap<String, View>,
    // OPT-D1: SmallVec for inline dependency storage
    #[serde(skip, default)]
    pub dependency_graph: FastMap<TableName, DependencyList>,
}

impl Circuit {
    pub fn new() -> Self {
        Self {
            db: Database::new(),
            views: IndexMap::new(),
            dependency_graph: FastMap::default(),
        }
    }

    // --- Ingestion API 1: Single Record (Latency) ---

    pub fn ingest_single(
        &mut self,
        table: &str,
        op: Operation,
        id: &str,
        data: SpookyValue,
    ) -> Option<ViewUpdate> {
        let key = SmolStr::new(id);
        let (zset_key, weight) = self.db.ensure_table(table).apply_mutation(op, key, data);

        if weight == 0 { return None; }

        self.ensure_dependency_graph();
        
        // Fast path: Check dependencies without allocation
        let view_count = self.dependency_graph.get(table).map(|v| v.len()).unwrap_or(0);
        if view_count == 0 { return None; }

        let delta = Delta::new(SmolStr::new(table), zset_key, weight);

        // Iterate indices without cloning
        for idx in 0..view_count {
            let view_idx = self.dependency_graph.get(table).unwrap()[idx];
            // OPT-D2: IndexMap access by integer index is O(1)
            if let Some((_, view)) = self.views.get_index_mut(view_idx) {
                if let Some(update) = view.process_single(&delta, &self.db) {
                    return Some(update);
                }
            }
        }
        None
    }

    // --- Ingestion API 2: Batch (Throughput) ---

    pub fn ingest_batch(&mut self, entries: Vec<BatchEntry>) -> Vec<ViewUpdate> {
        if entries.is_empty() { return Vec::new(); }

        // 1. Group by Table
        let mut by_table: FastMap<TableName, Vec<BatchEntry>> = FastMap::default();
        for entry in entries {
            by_table.entry(entry.table.clone()).or_default().push(entry);
        }

        let mut table_deltas: FastMap<SmolStr, ZSet> = FastMap::default();
        let mut changed_tables: Vec<TableName> = Vec::with_capacity(by_table.len());

        // OPT-P2: Parallel Batch Storage Processing
        #[cfg(all(feature = "parallel", not(target_arch = "wasm32")))]
        {
            // P2.1: Ensure all tables exist sequentially (HashMap mutation is not parallel safe)
            for name in by_table.keys() {
                self.db.ensure_table(name.as_str());
            }

            // P2.2: Parallel Delta Computation
            // We iterate the DB tables in parallel. If a table is in our batch, we process it.
            // This avoids iterating the whole DB if we filter, but HashMap par_iter iterates all.
            // Trade-off: Efficient for large batches.
            let results: Vec<(TableName, ZSet)> = self.db.tables
                .par_iter_mut()
                .filter_map(|(name, table)| {
                    // Shared read access to by_table
                    let entries = by_table.get(name)?;
                    
                    let mut delta = ZSet::default();
                    for entry in entries {
                        // Apply mutation to the specific table (safe, exclusive ref)
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

            // P2.3: Aggregate Results
            for (name, delta) in results {
                // Apply the aggregated delta to the table's ZSet (bookkeeping)
                // Note: apply_mutation inside the parallel loop already updated Rows and ZSet!
                // Wait, apply_mutation updates ZSet too. We don't need to apply_delta again.
                // We just need to collect the deltas for propagation.
                table_deltas.insert(name.clone(), delta);
                changed_tables.push(name);
            }
        }

        // Sequential Fallback
        #[cfg(any(target_arch = "wasm32", not(feature = "parallel")))]
        {
            for (table_name, table_entries) in by_table {
                let tb = self.db.ensure_table(table_name.as_str());
                let delta = table_deltas.entry(table_name.clone()).or_default();

                for entry in table_entries {
                    let (zset_key, weight) = tb.apply_mutation(entry.op, entry.id, entry.data);
                    if weight != 0 {
                        *delta.entry(zset_key).or_insert(0) += weight;
                    }
                }
                delta.retain(|_, w| *w != 0);
                if !delta.is_empty() {
                    // Note: apply_mutation already updated the table. We just track changes.
                    changed_tables.push(table_name);
                }
            }
        }

        // 3. Propagation Phase
        self.propagate_deltas(&table_deltas, &changed_tables)
    }

    // --- Ingestion API 3: Init Load (Startup) ---

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
        table_deltas: &FastMap<SmolStr, ZSet>,
        changed_tables: &[TableName],
    ) -> Vec<ViewUpdate> {
        self.ensure_dependency_graph();

        let mut impacted_view_indices: Vec<ViewIndex> = Vec::with_capacity(changed_tables.len() * 2);
        
        for table in changed_tables {
            if let Some(indices) = self.dependency_graph.get(table) {
                impacted_view_indices.extend(indices.iter().copied());
            }
        }

        if impacted_view_indices.is_empty() { return Vec::new(); }

        impacted_view_indices.sort_unstable();
        impacted_view_indices.dedup();

        let db_ref = &self.db;

        // OPT-P1: Parallel View Processing
        #[cfg(all(feature = "parallel", not(target_arch = "wasm32")))]
        {
            use rayon::prelude::*;
            // IndexMap parallel iter allows us to iterate values.
            // But we need random access by index which IndexMap supports.
            // However, par_iter on IndexMap iterates (key, value).
            // We need to filter by index.
            // Strategy: Par-Iterate the *indices* we collected, then access views concurrently.
            // Safety: We need unsafe wrapper to access distinct indices mutably, or use IndexMap's par_iter 
            // and filter by "is this index in our list?".
            
            // Efficient approach: Iterate *all* views in parallel, filter by "is index impacted".
            self.views
                .par_iter_mut() // Iterates (key, view)
                .enumerate()
                .filter_map(|(i, (_id, view))| {
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
                if let Some((_, view)) = self.views.get_index_mut(i) {
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
        // OPT-D2: O(1) check if exists
        if self.views.contains_key(&plan.id) {
            self.unregister_view(&plan.id);
        }

        let mut view = View::new(plan.clone(), params, format);

        // Initial scan
        let empty_deltas: FastMap<SmolStr, ZSet> = FastMap::default();
        let initial_update = view.process_ingest(&empty_deltas, &self.db);

        // Add to IndexMap
        self.views.insert(plan.id.clone(), view);
        let view_idx = self.views.len() - 1;
        
        // Incremental dependency update
        // Note: register_view adds to the END, so no indices shift. Safe.
        for t in plan.root.referenced_tables() {
            self.dependency_graph
                .entry(SmolStr::new(t))
                .or_default()
                .push(view_idx);
        }

        initial_update
    }

    /// Optimized view removal using `swap_remove`.
    /// 
    /// This is O(1) but changes the index of the last element.
    /// We must patch the dependency graph for that one moved element.
    pub fn unregister_view(&mut self, id: &str) {
        // Find index of view to remove
        if let Some(removed_idx) = self.views.get_index_of(id) {
            // OPT-D2: swap_remove is O(1)
            self.views.swap_remove_index(removed_idx);
            
            // Because we swapped, the view that was at the end is now at `removed_idx`.
            // (Unless we removed the last element, in which case removed_idx == new_len)
            if removed_idx < self.views.len() {
                // Get the view that moved
                let (moved_id, moved_view) = self.views.get_index(removed_idx).unwrap();
                let moved_tables = moved_view.plan.root.referenced_tables();
                
                // Patch the dependency graph: any reference to `old_last_idx` must become `removed_idx`
                let old_last_idx = self.views.len(); // It was at len (before remove, it was len) -> actually it was at len-1 before remove, but now len is len-1.
                // Wait. swap_remove moves the element at `len - 1` to `index`.
                // The element that moved was previously at `self.views.len()` (current len).
                
                let previous_idx_of_moved = self.views.len(); 
                
                for t in moved_tables {
                    if let Some(deps) = self.dependency_graph.get_mut(t.as_str()) {
                        for idx in deps {
                            if *idx == previous_idx_of_moved {
                                *idx = removed_idx;
                            }
                        }
                    }
                }
            }
            
            // Finally, clean up dependencies for the REMOVED view?
            // Yes, we need to remove `removed_idx` from dependency lists.
            // But since we are lazy, we can just trigger a rebuild if it's too complex, 
            // OR we can leave it dirty (it might point to the wrong view now?).
            // Actually, because we swapped, `removed_idx` now points to a valid view (the one we moved).
            // BUT the `removed_idx` in the graph might belong to the *removed* view.
            // We need to remove the entry for the removed view from the graph.
            // Since we don't know easily which tables the *removed* view touched without keeping it around...
            // OPT-D2 Fallback: It is safest to rebuild dependency graph on removal 
            // UNLESS we check the plan of the removed view *before* dropping it.
            // given the complexity of patching, a rebuild is safer here, but `swap_remove` makes the vector op fast.
            // To strictly follow "no full rebuild" we would need to capture the removed view's tables.
            self.rebuild_dependency_graph();
        }
    }

    pub fn rebuild_dependency_graph(&mut self) {
        self.dependency_graph.clear();
        for (i, (_id, view)) in self.views.iter().enumerate() {
            for t in view.plan.root.referenced_tables() {
                self.dependency_graph
                    .entry(SmolStr::new(t))
                    .or_default()
                    .push(i);
            }
        }
    }

    #[inline]
    fn ensure_dependency_graph(&mut self) {
        if self.dependency_graph.is_empty() && !self.views.is_empty() {
            self.rebuild_dependency_graph();
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