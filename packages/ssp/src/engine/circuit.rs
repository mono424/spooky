use crate::debug_log;
use super::view::{
    FastMap, QueryPlan, RowKey, SpookyValue, View, ViewUpdate, ZSet,
};
use super::update::ViewResultFormat;
use super::metadata::{BatchMeta, VersionStrategy, RecordMeta};
// use rustc_hash::{FxHashMap, FxHasher}; // Unused in this file (used via FastMap)
use serde::{Deserialize, Serialize};
use serde_json::Value;
use smol_str::SmolStr;

// --- Types ---

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Operation {
    Create,
    Update,
    Delete,
}

impl Operation {
    #[inline]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_ascii_uppercase().as_str() {
            "CREATE" => Some(Operation::Create),
            "UPDATE" => Some(Operation::Update),
            "DELETE" => Some(Operation::Delete),
            _ => None,
        }
    }

    #[inline]
    pub fn weight(&self) -> i64 {
        match self {
            Operation::Create | Operation::Update => 1,
            Operation::Delete => -1,
        }
    }

    #[inline]
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
    pub meta: Option<RecordMeta>,
}

impl BatchEntry {
    #[inline]
    pub fn new(
        table: impl Into<SmolStr>,
        op: Operation,
        id: impl Into<SmolStr>,
        record: SpookyValue,
        hash: String,
    ) -> Self {
        Self {
            table: table.into(),
            op,
            id: id.into(),
            record,
            hash,
            meta: None,
        }
    }

    #[inline]
    pub fn with_meta(mut self, meta: RecordMeta) -> Self {
        self.meta = Some(meta);
        self
    }

    #[inline]
    pub fn with_version(mut self, version: u64) -> Self {
        self.meta = Some(RecordMeta::new().with_version(version));
        self
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
            meta: None,
        })
    }
}

pub struct IngestBatch {
    entries: Vec<BatchEntry>,
    default_strategy: Option<VersionStrategy>,
}

impl IngestBatch {
    #[inline]
    pub fn new() -> Self {
        Self { entries: Vec::new(), default_strategy: None }
    }

    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        Self { entries: Vec::with_capacity(capacity), default_strategy: None }
    }

    #[inline]
    pub fn with_strategy(mut self, strategy: VersionStrategy) -> Self {
        self.default_strategy = Some(strategy);
        self
    }

    #[inline]
    pub fn create(mut self, table: &str, id: &str, record: SpookyValue, hash: String) -> Self {
        self.entries.push(BatchEntry::new(table, Operation::Create, id, record, hash));
        self
    }

    #[inline]
    pub fn update(mut self, table: &str, id: &str, record: SpookyValue, hash: String) -> Self {
        self.entries.push(BatchEntry::new(table, Operation::Update, id, record, hash));
        self
    }

    #[inline]
    pub fn delete(mut self, table: &str, id: &str) -> Self {
        self.entries.push(BatchEntry::new(table, Operation::Delete, id, SpookyValue::Null, String::new()));
        self
    }

    #[inline]
    pub fn create_with_version(mut self, table: &str, id: &str, record: SpookyValue, hash: String, version: u64) -> Self {
        self.entries.push(BatchEntry::new(table, Operation::Create, id, record, hash).with_version(version));
        self
    }

    #[inline]
    pub fn update_with_version(mut self, table: &str, id: &str, record: SpookyValue, hash: String, version: u64) -> Self {
        self.entries.push(BatchEntry::new(table, Operation::Update, id, record, hash).with_version(version));
        self
    }

    #[inline]
    pub fn entry(mut self, entry: BatchEntry) -> Self {
        self.entries.push(entry);
        self
    }

    #[inline]
    pub fn build(self) -> (Vec<BatchEntry>, Option<VersionStrategy>) {
        (self.entries, self.default_strategy)
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for IngestBatch {
    fn default() -> Self {
        Self::new()
    }
}

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
    #[inline]
    pub fn new(name: String) -> Self {
        Self {
            name,
            zset: FastMap::default(),
            rows: FastMap::default(),
            hashes: FastMap::default(),
        }
    }

    // Changing signature to use SmolStr is implied by RowKey definition change
    #[inline]
    pub fn update_row(&mut self, key: SmolStr, data: SpookyValue, hash: String) {
        self.rows.insert(key.clone(), data);
        self.hashes.insert(key, hash);
    }

    #[inline]
    pub fn delete_row(&mut self, key: &SmolStr) {
        self.rows.remove(key);
        self.hashes.remove(key);
    }

    #[inline]
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

impl Default for Database {
    fn default() -> Self {
        Self::new()
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

    // Unified Ingestion API
    pub fn ingest(&mut self, batch: IngestBatch, is_optimistic: bool) -> Vec<ViewUpdate> {
        let (entries, strategy) = batch.build();
        self.ingest_entries_internal(entries, strategy, is_optimistic)
    }

    pub fn ingest_entries(&mut self, entries: Vec<BatchEntry>, is_optimistic: bool) -> Vec<ViewUpdate> {
        self.ingest_entries_internal(entries, None, is_optimistic)
    }

    // Backward compatibility wrappers
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
            vec![BatchEntry::new(table, op, id, SpookyValue::from(record), hash.to_string())],
            is_optimistic,
        )
    }

    pub fn ingest_batch(
        &mut self,
        batch: Vec<(String, String, String, Value, String)>,
        is_optimistic: bool,
    ) -> Vec<ViewUpdate> {
        let entries: Vec<BatchEntry> = batch.into_iter().filter_map(BatchEntry::from_tuple).collect();
        self.ingest_entries(entries, is_optimistic)
    }

    // Support existing ingest_with_meta by converting to unified format
    pub fn ingest_with_meta(
        &mut self,
        table: &str,
        op: &str,
        id: &str,
        record: Value,
        hash: &str,
        batch_meta: Option<&BatchMeta>,
        is_optimistic: bool,
    ) -> Vec<ViewUpdate> {
        let op_enum = match Operation::from_str(op) {
            Some(o) => o,
            None => return Vec::new(),
        };
        
        let mut entry = BatchEntry::new(
            table,
            op_enum,
            id,
            SpookyValue::from(record),
            hash.to_string(),
        );

        // Attach metadata if present
        if let Some(meta) = batch_meta {
            if let Some(record_meta) = meta.get(id) {
                entry = entry.with_meta(record_meta.clone());
            }
        }

        // We can pass the strategy from batch_meta if we extract it, 
        // but since we are attaching per-record meta, strictly speaking we might lose the 'default strategy' 
        // if we don't pass it. 
        // However, for single record ingestion, attaching meta is sufficient.
        let strategy = batch_meta.map(|m| m.default_strategy.clone());
        
        self.ingest_entries_internal(vec![entry], strategy, is_optimistic)
    }

    // Re-impl of ingest_batch_with_meta using unified logic
    pub fn ingest_batch_with_meta(
        &mut self,
        batch: Vec<(SmolStr, SmolStr, SmolStr, SpookyValue, String)>,
        batch_meta: Option<&BatchMeta>,
        is_optimistic: bool,
    ) -> Vec<ViewUpdate> {
        let entries: Vec<BatchEntry> = batch.into_iter().filter_map(|(t, o, i, r, h)| {
            let op = Operation::from_str(&o)?;
            let mut entry = BatchEntry::new(t, op, i.clone(), r, h);
            if let Some(meta) = batch_meta {
                if let Some(record_meta) = meta.get(i.as_str()) {
                    entry = entry.with_meta(record_meta.clone());
                }
            }
            Some(entry)
        }).collect();

        let strategy = batch_meta.map(|m| m.default_strategy.clone());
        self.ingest_entries_internal(entries, strategy, is_optimistic)
    }

    // SINGLE internal implementation
    fn ingest_entries_internal(
        &mut self,
        entries: Vec<BatchEntry>,
        default_strategy: Option<VersionStrategy>,
        is_optimistic: bool,
    ) -> Vec<ViewUpdate> {
        if entries.is_empty() {
            return Vec::new();
        }

        // Build per-record metadata map from entries that have explicit meta
        let batch_meta = self.build_batch_meta(&entries, default_strategy);

        // Group by table for cache-friendly processing
        let mut by_table: FastMap<SmolStr, Vec<BatchEntry>> = FastMap::default();
        for entry in entries {
            by_table.entry(entry.table.clone()).or_default().push(entry);
        }

        let mut table_deltas: FastMap<String, ZSet> = FastMap::default();

        // Process each table's entries together
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

        self.propagate_deltas(table_deltas, batch_meta.as_ref(), is_optimistic)
    }

    fn build_batch_meta(&self, entries: &[BatchEntry], default_strategy: Option<VersionStrategy>) -> Option<BatchMeta> {
        let has_any_meta = entries.iter().any(|e| e.meta.is_some()) || default_strategy.is_some();
        if !has_any_meta {
            return None;
        }

        let mut batch_meta = BatchMeta::new();
        if let Some(strategy) = default_strategy {
            batch_meta.default_strategy = strategy;
        }
        for entry in entries {
            if let Some(ref meta) = entry.meta {
                batch_meta.records.insert(entry.id.clone(), meta.clone());
            }
        }
        Some(batch_meta)
    }

    fn propagate_deltas(
        &mut self, 
        mut table_deltas: FastMap<String, ZSet>, 
        batch_meta: Option<&BatchMeta>,
        is_optimistic: bool
    ) -> Vec<ViewUpdate> {
        // Apply Deltas to DB ZSets
        let mut changed_tables = Vec::with_capacity(table_deltas.len());
        for (table, delta) in &mut table_deltas {
            delta.retain(|_, w| *w != 0);
            if !delta.is_empty() {
                let tb = self.db.ensure_table(table.as_str());
                tb.apply_delta(delta);
                changed_tables.push(table.to_string());
            }
        }

        // Optimized Lazy Rebuild Check (once per batch)
        if self.dependency_graph.is_empty() && !self.views.is_empty() {
            self.rebuild_dependency_graph();
        }

        // Identify ALL affected views from ALL changed tables
        let mut impacted_view_indices: Vec<usize> = Vec::with_capacity(self.views.len());
        debug_log!("DEBUG: Changed tables: {:?}", changed_tables);
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

        // 3. Execution Phase
        let db_ref = &self.db;
        let deltas_ref = &table_deltas;

        #[cfg(all(feature = "parallel", not(target_arch = "wasm32")))]
        {
            use rayon::prelude::*;
            self.views
                .par_iter_mut()
                .enumerate()
                .filter_map(|(i, view)| {
                    // Check if this view needs update.
                    // impacted_view_indices is sorted, so binary_search is efficient.
                    if impacted_view_indices.binary_search(&i).is_ok() {
                        view.process_ingest_with_meta(deltas_ref, db_ref, is_optimistic, batch_meta)
                    } else {
                        None
                    }
                })
                .collect()
        }

        #[cfg(any(target_arch = "wasm32", not(feature = "parallel")))]
        {
            let mut ups = Vec::new();
            for i in impacted_view_indices {
                if i < self.views.len() {
                    let view: &mut View = &mut self.views[i];
                    if let Some(update) = view.process_ingest_with_meta(deltas_ref, db_ref, is_optimistic, batch_meta) {
                        ups.push(update);
                    }
                }
            }
            ups
        }
    }

    /// Register a view (backward compatible)
    pub fn register_view(
        &mut self,
        plan: QueryPlan,
        params: Option<Value>,
        format: Option<ViewResultFormat>,
    ) -> Option<ViewUpdate> {
        self.register_view_with_strategy(plan, params, format, None)
    }

    /// Register a view with explicit version strategy
    pub fn register_view_with_strategy(
        &mut self,
        plan: QueryPlan,
        params: Option<Value>,
        format: Option<ViewResultFormat>,
        strategy: Option<VersionStrategy>,
    ) -> Option<ViewUpdate> {
        if let Some(pos) = self.views.iter().position(|v| v.plan.id == plan.id) {
            self.views.remove(pos);
            self.rebuild_dependency_graph();
        }

        let mut view = View::new_with_strategy(
            plan, 
            params, 
            format.clone(), 
            strategy.unwrap_or_else(|| match format {
                Some(ViewResultFormat::Tree) => VersionStrategy::HashBased,
                _ => VersionStrategy::Optimistic,
            })
        );

        let empty_deltas: FastMap<String, ZSet> = FastMap::default();
        let initial_update = view.process_ingest(&empty_deltas, &self.db, true);

        let view_idx = self.views.len();
        self.views.push(view);

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

impl Default for Circuit {
    fn default() -> Self {
        Self::new()
    }
}
