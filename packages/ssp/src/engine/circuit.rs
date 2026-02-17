use super::types::{BatchDeltas, Delta, FastMap, FastHashSet, Operation, SpookyValue};
use super::update::{ViewResultFormat, ViewUpdate};
use super::view::{QueryPlan, View};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use smallvec::SmallVec;
use smol_str::SmolStr;

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
        pub fn new(
            table: impl Into<TableName>,
            op: Operation,
            id: impl Into<SmolStr>,
            data: SpookyValue,
        ) -> Self {
            Self {
                table: table.into(),
                op,
                id: id.into(),
                data,
            }
        }

        pub fn create(
            table: impl Into<TableName>,
            id: impl Into<SmolStr>,
            data: SpookyValue,
        ) -> Self {
            Self::new(table, Operation::Create, id, data)
        }

        pub fn update(
            table: impl Into<TableName>,
            id: impl Into<SmolStr>,
            data: SpookyValue,
        ) -> Self {
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
            Self {
                table: table.into(),
                id: id.into(),
                data,
            }
        }
    }
}

// --- Core Implementation ---

use self::dto::*;
use self::types::*;

use crate::db_mod::db::Database;

// ... (keep modules)

// --- Core Implementation ---

// use self::dto::*; // unused
// use self::types::*; // unused

pub fn default_db() -> Database {
    // Default to ./spooky.db for now
    Database::new("spooky.db").expect("Failed to open default database")
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Circuit {
    #[serde(skip, default = "default_db")]
    pub db: Database,
    // Using Vec<View> + manual swap_remove for O(1) removal without external crate deps
    pub views: Vec<View>,
    #[serde(skip)]
    pub dependency_list: FastMap<TableName, DependencyList>,
    #[serde(skip)]
    pub _temp_dir: Option<std::sync::Arc<tempfile::TempDir>>,
}

impl Circuit {
    pub fn new() -> Self {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let db_path = temp_dir.path().join("spooky.db");
        let db = Database::new(&db_path).expect("failed to create temp db");
        
        Self {
            db,
            views: Vec::new(),
            dependency_list: FastMap::default(),
            _temp_dir: Some(std::sync::Arc::new(temp_dir)),
        }
    }

    /// Load circuit state from JSON string and initialize all views
    pub fn load_from_json(json: &str) -> anyhow::Result<Self> {
        let mut circuit: Circuit = serde_json::from_str(json)?;

        // CRITICAL: Initialize cached flags for all views
        for view in &mut circuit.views {
            view.initialize_after_deserialize();
        }

        tracing::debug!(
            target: "ssp::circuit::load",
            views_count = circuit.views.len(),
            "Loaded and initialized circuit from JSON"
        );

        Ok(circuit)
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

    pub fn ingest_single(&mut self, entrie: BatchEntry) -> ViewUpdateList {
        let op = entrie.op;
        let key = SmolStr::new(entrie.id);
        let (zset_key, _weight) = self.db.table(entrie.table.as_str()).apply_mutation(
            op,
            key.clone(),
            entrie.data,
        );

        self.ensure_dependency_list();

        let table_key = SmolStr::new(entrie.table);

        // Clone indices to avoid borrow conflict with self.views
        let view_indices: SmallVec<[ViewIndex; 4]> = self
            .dependency_list
            .get(&table_key)
            .map(|v| v.iter().copied().collect())
            .unwrap_or_default();

        tracing::debug!(
            target: "ssp::circuit::ingest",
            table = %table_key,
            record_id = %key,
            op = ?op,
            views_to_notify = view_indices.len(),
            total_views = self.views.len(),
            "Ingesting record"
        );

        if view_indices.is_empty() {
            tracing::debug!(
                target: "ssp::circuit::ingest",
                table = %table_key,
                "No views depend on this table"
            );
            return SmallVec::new();
        }

        // Use Delta::from_operation to include content_changed flag
        let delta = Delta::from_operation(table_key, zset_key, op);
        let mut updates: ViewUpdateList = SmallVec::new();

        for view_idx in view_indices {
            if let Some(view) = self.views.get_mut(view_idx) {
                tracing::info!(
                    target: "ssp::circuit::ingest",
                    view_idx = view_idx,
                    view_id = %view.plan.id,
                    cache_size = view.cache.len(),
                    last_hash_empty = view.last_hash.is_empty(),
                    "Processing delta for view"
                );

                if let Some(update) = view.process_delta(&delta, &self.db) {
                    updates.push(update);
                }
            }
        }

        updates
    }

    // --- Ingestion API 2: Batch ---

    pub fn ingest_batch(&mut self, entries: Vec<BatchEntry>) -> Vec<ViewUpdate> {
        if entries.is_empty() {
            return Vec::new();
        }

        // Prepare batch for DB
        let db_ops: Vec<_> = entries
            .into_iter()
            .map(|e| (e.table, e.op, SmolStr::new(e.id), e.data))
            .collect();

        // Apply batch
        let results = self
            .db
            .apply_batch(db_ops)
            .expect("batch application failed");

        // Process results to build deltas
        let mut batch_deltas = BatchDeltas::new();
        let mut changed_tables_set = FastHashSet::default();

        for (table, zset_key, weight, content_changed) in results {
            let table_str = table.to_string();
            if weight != 0 {
                *batch_deltas
                    .membership
                    .entry(table_str.clone())
                    .or_default()
                    .entry(zset_key.clone())
                    .or_insert(0) += weight;
            }
            if content_changed {
                batch_deltas
                    .content_updates
                    .entry(table_str.clone())
                    .or_default()
                    .insert(zset_key);
            }
            changed_tables_set.insert(table);
        }

        // Clean up empty deltas
        batch_deltas.membership.retain(|_, delta| {
            delta.retain(|_, w| *w != 0);
            !delta.is_empty()
        });
        // Also clean up empty content_updates if any (FastHashSet doesn't need pruning values, just check empty)
         batch_deltas.content_updates.retain(|_, updates| !updates.is_empty());


        let changed_tables: Vec<TableName> = changed_tables_set.into_iter().collect();

        self.propagate_deltas(&batch_deltas, &changed_tables)
    }

    // --- Ingestion API 3: Init Load ---

    pub fn init_load(&mut self, records: impl IntoIterator<Item = LoadRecord>) {
        let db_ops: Vec<_> = records
            .into_iter()
            .map(|r| (r.table, Operation::Create, r.id, r.data))
            .collect();

        if !db_ops.is_empty() {
            tracing::debug!(target: "ssp::circuit::init_load", count = db_ops.len(), "Loading records");
            self.db.apply_batch(db_ops).expect("init_load failed");
        }
    }

    pub fn init_load_grouped(
        &mut self,
        by_table: impl IntoIterator<Item = (TableName, Vec<(SmolStr, SpookyValue)>)>,
    ) {
        let mut db_ops = Vec::new();
        for (table_name, records) in by_table {
             // We can reserve here if we want, but apply_batch handles it.
             // redb doesn't need reserve.
            for (id, data) in records {
                 db_ops.push((table_name.clone(), Operation::Create, id, data));
            }
        }
        
        if !db_ops.is_empty() {
             tracing::debug!(target: "ssp::circuit::init_load_grouped", count = db_ops.len(), "Loading grouped records");
             self.db.apply_batch(db_ops).expect("init_load_grouped failed");
        }
    }

    // --- Propagation Logic ---

    fn propagate_deltas(
        &mut self,
        batch_deltas: &BatchDeltas,
        changed_tables: &[TableName],
    ) -> Vec<ViewUpdate> {
        self.ensure_dependency_list();

        let mut impacted_view_indices: Vec<ViewIndex> =
            Vec::with_capacity(changed_tables.len() * 2);

        for table in changed_tables {
            if let Some(indices) = self.dependency_list.get(table) {
                impacted_view_indices.extend(indices.iter().copied());
            }
        }

        if impacted_view_indices.is_empty() {
            return Vec::new();
        }

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
                        view.process_batch(batch_deltas, db_ref)
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
                    if let Some(update) = view.process_batch(batch_deltas, db_ref) {
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
            tracing::warn!(
                target: "ssp::circuit::register",
                view_id = %plan.id,
                old_cache_size = self.views[pos].cache.len(),
                "Re-registering existing view - old cache will be lost!"
            );
            self.unregister_view_by_index(pos);
        }

        let referenced_tables = plan.root.referenced_tables();

        tracing::info!(
            target: "ssp::circuit::register",
            view_id = %plan.id,
            referenced_tables = ?referenced_tables,
            "Registering new view"
        );

        let mut view = View::new(plan.clone(), params, format);

        let empty_deltas = BatchDeltas::new();
        let initial_update = view.process_batch(&empty_deltas, &self.db);

        tracing::info!(
            target: "ssp::circuit::register",
            view_id = %plan.id,
            cache_size_after_init = view.cache.len(),
            last_hash = %view.last_hash,
            "View initialized"
        );

        self.views.push(view);
        let view_idx = self.views.len() - 1;

        for t in referenced_tables {
            self.dependency_list
                .entry(SmolStr::new(t))
                .or_default()
                .push(view_idx);
        }

        tracing::debug!(
            target: "ssp::circuit::register",
            view_id = %plan.id,
            view_idx = view_idx,
            total_views = self.views.len(),
            "View added to circuit"
        );

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

        #[cfg(debug_assertions)]
        {
            for (table, indices) in &self.dependency_list {
                let unique: std::collections::HashSet<_> = indices.iter().collect();
                if unique.len() != indices.len() {
                    tracing::error!(
                        target: "ssp::circuit::unreister_view_index",
                        table = %table,
                        indices = ?indices,
                        "Duplicate view indices in dependency_list!"
                    );
                }
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
