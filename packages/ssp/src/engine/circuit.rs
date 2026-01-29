use super::types::{
    make_zset_key, BatchDeltas, Delta, FastMap, Operation, RowKey, SpookyValue, ZSet,
};
use super::update::{ViewResultFormat, ViewUpdate};
use super::view::{QueryPlan, View};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use smallvec::SmallVec;
use smol_str::SmolStr;

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

    //quick fix for version look up
    pub fn get_record_version(&self, id: &str) -> Option<i64> {
        let sv = self.rows.get(id)?;
        let version = sv.get("_spooky_version")?.as_f64()?;
        Some(version as i64)
    }

    pub fn reserve(&mut self, additional: usize) {
        self.rows.reserve(additional);
        self.zset.reserve(additional);
    }

    pub fn apply_mutation(
        &mut self,
        op: Operation,
        key: SmolStr,
        data: SpookyValue,
    ) -> (SmolStr, i64) {
        let weight = op.weight();
        match op {
            Operation::Create | Operation::Update => {
                self.rows.insert(key.clone(), data);
            }
            Operation::Delete => {
                self.rows.remove(&key);
            }
        }

        let zset_key = make_zset_key(&self.name, &key);
        if weight != 0 {
            let entry = self.zset.entry(zset_key.clone()).or_insert(0);
            *entry += weight;
            if *entry == 0 {
                self.zset.remove(&zset_key);
            }
        }
        (zset_key, weight)
    }

    pub fn apply_delta(&mut self, delta: &ZSet) {
        for (key, weight) in delta {
            tracing::debug!(target: "ssp::circuit::apply_delta", "key: {}", key);
            let entry = self.zset.entry(key.clone()).or_insert(0);
            *entry += weight;
            if *entry == 0 {
                self.zset.remove(key);
            }
        }
    }
}

//cargo test -p ssp --lib engine::circuit::table::apply_mutation
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn create_record(tb_name: &str, record_id: &str) -> LoadRecord {
        LoadRecord::new(
            tb_name,
            record_id,
            json!({ "status": "spooky", "level": 10, "_spooky_version": 3 }).into(),
        )
    }

    fn common() -> (SmolStr, i64, Table) {
        let record_user1 = create_record("user", "user:23lk4j233jd");
        let mut tb_user = Table::new(SmolStr::from("user"));
        let (zset_key, weight) = tb_user.apply_mutation(
            Operation::Create,
            SmolStr::from("user:23lk4j233jd"),
            record_user1.data,
        );
        return (zset_key, weight, tb_user);
    }

    #[test]
    fn apply_mutation_check() {
        let (zset_key, weight, _) = common();

        assert_eq!(zset_key, SmolStr::from("user:23lk4j233jd"));
        assert_eq!(weight, 1 as i64);
    }

    #[test]
    fn version_check() {
        let (_, _, tb) = common();
        let version = tb.get_record_version("user:23lk4j233jd");
        assert_eq!(version, Some(3));
    }

    #[test]
    fn apply_delta_check() {
        let mut tb = Table::new(SmolStr::new("user"));
        let mut zset: ZSet = FastMap::default();
        zset.entry(SmolStr::new("user:23lk4j233jd")).or_insert(1);
        zset.entry(SmolStr::new("user:ssdf8sdf")).or_insert(1);
        tb.apply_delta(&zset);

        let zset_value = tb.zset.get("user:23lk4j233jd").unwrap();
        assert_eq!(*zset_value, 1 as i64);
        let zset_value = tb.zset.get("user:ssdf8sdf").unwrap();
        assert_eq!(*zset_value, 1 as i64);
        let mut delta_zset: ZSet = FastMap::default();
        delta_zset.insert(SmolStr::new("user:ssdf8sdf"), 1);
        tb.apply_delta(&delta_zset);
        let zset_value = tb.zset.get("user:ssdf8sdf").unwrap();
        assert_eq!(*zset_value, 2 as i64);
        let mut delta_zset: ZSet = FastMap::default();
        delta_zset.insert(SmolStr::new("user:ssdf8sdf"), -2);
        tb.apply_delta(&delta_zset);

        let zset_value = tb.zset.get("user:ssdf8sdf");
        assert!(zset_value.is_none());
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
        Self {
            tables: FastMap::default(),
        }
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
        let (zset_key, _weight) = self.db.ensure_table(entrie.table.as_str()).apply_mutation(
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

        let mut by_table: FastMap<TableName, Vec<BatchEntry>> = FastMap::default();
        for entry in entries {
            by_table.entry(entry.table.clone()).or_default().push(entry);
        }

        let mut batch_deltas = BatchDeltas::new();
        let mut changed_tables: Vec<TableName> = Vec::with_capacity(by_table.len());

        // Parallel Storage Phase
        #[cfg(all(feature = "parallel", not(target_arch = "wasm32")))]
        {
            // P2.1: Ensure tables exist sequentially
            for name in by_table.keys() {
                self.db.ensure_table(name.as_str());
            }

            // P2.2: Parallel Delta Computation
            let results: Vec<(String, ZSet, Vec<SmolStr>)> = self
                .db
                .tables
                .par_iter_mut()
                .filter_map(|(name, table)| {
                    let name_smol = SmolStr::new(name);
                    let entries = by_table.get(&name_smol)?;

                    let mut delta = ZSet::default();
                    let mut content_updates = Vec::new();

                    for entry in entries {
                        let (zset_key, weight) =
                            table.apply_mutation(entry.op, entry.id.clone(), entry.data.clone());
                        tracing::debug!(target: "ssp::circut::ingest_batch", "entry id: {}, zset: {}", entry.id, zset_key);
                        if weight != 0 {
                            *delta.entry(zset_key.clone()).or_insert(0) += weight;
                        }
                        if entry.op.changes_content() {
                            content_updates.push(zset_key);
                        }
                    }
                    delta.retain(|_, w| *w != 0);

                    if !delta.is_empty() || !content_updates.is_empty() {
                        Some((name.clone(), delta, content_updates))
                    } else {
                        None
                    }
                })
                .collect();

            for (name_str, delta, content_updates) in results {
                let smol_name = SmolStr::new(&name_str);
                if !delta.is_empty() {
                    batch_deltas.membership.insert(name_str.clone(), delta);
                }
                if !content_updates.is_empty() {
                    batch_deltas
                        .content_updates
                        .insert(name_str, content_updates);
                }
                changed_tables.push(smol_name);
            }
        }

        // Sequential Fallback
        #[cfg(any(target_arch = "wasm32", not(feature = "parallel")))]
        {
            for (table_name, table_entries) in by_table {
                let tb = self.db.ensure_table(table_name.as_str());

                let mut has_changes = false;
                for entry in table_entries {
                    let (zset_key, weight) = tb.apply_mutation(entry.op, entry.id, entry.data);

                    if weight != 0 {
                        let delta = batch_deltas
                            .membership
                            .entry(table_name.to_string())
                            .or_default();
                        *delta.entry(zset_key.clone()).or_insert(0) += weight;
                        has_changes = true;
                    }

                    if entry.op.changes_content() {
                        batch_deltas
                            .content_updates
                            .entry(table_name.to_string())
                            .or_default()
                            .push(zset_key);
                        has_changes = true;
                    }
                }

                if has_changes {
                    changed_tables.push(table_name);
                }
            }

            // Clean up empty deltas
            batch_deltas.membership.retain(|_, delta| !delta.is_empty());
        }

        self.propagate_deltas(&batch_deltas, &changed_tables)
    }

    // --- Ingestion API 3: Init Load ---

    pub fn init_load(&mut self, records: impl IntoIterator<Item = LoadRecord>) {
        for record in records {
            let tb = self.db.ensure_table(record.table.as_str());
            tracing::debug!(target: "ssp::circuit::init_load", "table: {}, id: {}", record.table, record.id);
            let zset_key = make_zset_key(&record.table, &record.id);
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
                tracing::debug!(target: "ssp::circuit::init_load_grouped", "table: {}, id: {}", table_name, id);
                let zset_key = make_zset_key(&table_name, &id);
                tb.rows.insert(id, data);
                tb.zset.insert(zset_key, 1);
            }
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

        #[cfg(debug_assertions)]
        {
            let unique: std::collections::HashSet<_> = referenced_tables.iter().collect();
            if unique.len() != referenced_tables.len() {
                tracing::warn!(
                    target: "ssp::circuit::register",
                    view_id = %plan.id,
                    referenced = ?referenced_tables,
                    "referenced_tables() returned duplicates - this should be fixed in Operator"
                );
            }
        }

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
                        target: "ssp::circuit",
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
