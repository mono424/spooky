use super::view::{
    FastMap, QueryPlan, RowKey, SpookyValue, View, ViewUpdate, ZSet, Weight
};
use super::update::ViewResultFormat;
use super::metadata::{BatchMeta, IngestStrategy, RecordMeta};
// use rustc_hash::{FxHashMap, FxHasher}; // Unused in this file (used via FastMap)
use serde::{Deserialize, Serialize};
use serde_json::Value;
use smol_str::SmolStr;  

use tracing::{instrument, info, debug};

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
    pub fn weight(&self) -> Weight {
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
    pub meta: Option<RecordMeta>,
}

impl BatchEntry {
    #[inline]
    pub fn new(
        table: impl Into<SmolStr>,
        op: Operation,
        id: impl Into<SmolStr>,
        record: SpookyValue,
    ) -> Self {
        Self {
            table: table.into(),
            op,
            id: id.into(),
            record,
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

    pub fn from_tuple(tuple: (String, String, String, Value)) -> Option<Self> {
        let (table, op_str, id, record) = tuple;
        let op = Operation::from_str(&op_str)?;
        Some(Self {
            table: SmolStr::from(table),
            op,
            id: SmolStr::from(id),
            record: SpookyValue::from(record),
            meta: None,
        })
    }
}

#[derive(Clone, Debug)]
pub struct IngestBatch {
    entries: Vec<BatchEntry>,
    default_strategy: Option<IngestStrategy>,
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
    pub fn with_strategy(mut self, strategy: IngestStrategy) -> Self {
        self.default_strategy = Some(strategy);
        self
    }

    #[inline]
    pub fn create(mut self, table: &str, id: &str, record: SpookyValue) -> Self {
        self.entries.push(BatchEntry::new(table, Operation::Create, id, record));
        self
    }

    #[inline]
    pub fn update(mut self, table: &str, id: &str, record: SpookyValue) -> Self {
        self.entries.push(BatchEntry::new(table, Operation::Update, id, record));
        self
    }

    #[inline]
    pub fn delete(mut self, table: &str, id: &str) -> Self {
        self.entries.push(BatchEntry::new(table, Operation::Delete, id, SpookyValue::Null));
        self
    }

    #[inline]
    pub fn create_with_version(mut self, table: &str, id: &str, record: SpookyValue, version: u64) -> Self {
        self.entries.push(BatchEntry::new(table, Operation::Create, id, record).with_version(version));
        self
    }

    #[inline]
    pub fn update_with_version(mut self, table: &str, id: &str, record: SpookyValue, version: u64) -> Self {
        self.entries.push(BatchEntry::new(table, Operation::Update, id, record).with_version(version));
        self
    }

    #[inline]
    pub fn entry(mut self, entry: BatchEntry) -> Self {
        self.entries.push(entry);
        self
    }

    #[inline]
    pub fn build(self) -> (Vec<BatchEntry>, Option<IngestStrategy>) {
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
// opti: when make a module use redb + rkyv

#[derive(Clone, Debug, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct Table {
    pub name: String,
    pub zset: ZSet,                         // This is the fast FxHashMap
    pub rows: FastMap<RowKey, SpookyValue>, // Using SpookyValue
}

impl Table {
    #[inline]
    pub fn new(name: String) -> Self {
        Self {
            name,
            zset: FastMap::default(),
            rows: FastMap::default(),
        }
    }

    // Changing signature to use SmolStr is implied by RowKey definition change
    #[inline]
    pub fn update_row(&mut self, key: SmolStr, data: SpookyValue) {
        self.rows.insert(key, data);
    }

    #[inline]
    pub fn delete_row(&mut self, key: &SmolStr) {
        self.rows.remove(key);
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
            /*
            dependency_graph = {
                "users"    → [0, 2],     // Views 0 und 2 hängen von "users" ab
                "orders"   → [1, 2],     // Views 1 und 2 hängen von "orders" ab
                "products" → [1]         // View 1 hängt von "products" ab
            }
             */
            dependency_graph: FastMap::default(),
        }
    }

    // Unified Ingestion API
    // Optimized single record path (no meta, no version overhead)
    #[instrument(target = "ssp_module", level = "debug", skip(self, record))]
    pub fn ingest_single(
        &mut self,
        table: &str,
        op: Operation,
        id: &str,
        record: SpookyValue,
    ) -> Vec<ViewUpdate> {
        let tb = self.db.ensure_table(table);
        let id_smol = SmolStr::from(id);

        if op.is_additive() {
            tb.update_row(id_smol.clone(), record);
        } else {
            tb.delete_row(&id_smol);
        }

        // Optimized: minimal allocations for single record delta
        let mut delta = ZSet::default();
        delta.insert(id_smol, op.weight());

        let mut table_deltas = FastMap::default();
        table_deltas.insert(table.to_string(), delta);

        self.propagate_deltas(table_deltas, None)
    }

    #[instrument(target = "ssp_module", level = "debug", skip(self, batch))]
    pub fn ingest_batch(&mut self, batch: IngestBatch) -> Vec<ViewUpdate> {
        let (entries, strategy) = batch.build();
        if entries.is_empty() {
            return Vec::new();
        }

        // 1. Metadata vorbereiten
        let batch_meta = {
            let _span = tracing::debug_span!(target: "ssp_module", "build_metadata").entered();
            self.build_batch_meta(&entries, strategy)
        };

        // 2. Gruppieren nach Tabellen für effiziente DB-Updates
        let by_table: FastMap<SmolStr, Vec<BatchEntry>> = {
            let _span = tracing::debug_span!(target: "ssp_module", "group_by_table").entered();
            let mut by_table: FastMap<SmolStr, Vec<BatchEntry>> = FastMap::default();
            for entry in entries {
                by_table.entry(entry.table.clone()).or_default().push(entry);
            }
            by_table
        };

        let mut table_deltas: FastMap<String, ZSet> = FastMap::default();

        {
            let _span = tracing::debug_span!(target: "ssp_module", "apply_table_deltas", tables = by_table.len()).entered();
            for (table, table_entries) in by_table {
                //opti: not convert to string make a total SmolStr chain (&table) (2+ Alloc) -> 0 Alloc if < 23 bytes)
                let tb = self.db.ensure_table(table.as_str());
                let delta = table_deltas.entry(table.to_string()).or_default();

                for entry in table_entries {
                    let weight = entry.op.weight();
                    if entry.op.is_additive() {
                        tb.update_row(entry.id.clone(), entry.record);
                    } else {
                        tb.delete_row(&entry.id);
                    }
                    *delta.entry(entry.id).or_insert(0) += weight;
                }
            }
        }
        
        // 3. Änderungen an die Views propagieren
        let updates = self.propagate_deltas(table_deltas, batch_meta.as_ref());
        
        // 4. Enrich updates with metadata
        self.enrich_updates_with_meta(updates, batch_meta.as_ref())
    }


    //just for now public because of deprecated methods
    #[instrument(target = "ssp_module", level = "debug", skip(self, entries))]
    pub fn build_batch_meta(&self, entries: &[BatchEntry], default_strategy: Option<IngestStrategy>) -> Option<BatchMeta> {
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

    //just for now public because of deprecated methods
    #[instrument(target = "ssp_module", level = "debug", skip(self, table_deltas, batch_meta))]
    pub fn propagate_deltas(
        &mut self, 
        mut table_deltas: FastMap<String, ZSet>, 
        batch_meta: Option<&BatchMeta>,
    ) -> Vec<ViewUpdate> {
        // Apply Deltas to DB ZSets
        let changed_tables = {
            let _span = tracing::debug_span!(target: "ssp_module", "apply_db_deltas", table_count = table_deltas.len()).entered();
            let mut changed_tables = Vec::with_capacity(table_deltas.len());
            for (table, delta) in &mut table_deltas {

                let delta: &mut ZSet = delta;

                //remove all <tablename, ZSet> where zset weight is 0
                delta.retain(|_, w| *w != 0);
                if !delta.is_empty() {
                    //opti: not convert to string make a total SmolStr chain (&table) (2+ Alloc) -> 0 Alloc if < 23 bytes)
                    //q: why do i have to do it again?
                    let tb = self.db.ensure_table(table.as_str());
                    //add update weights of Table ZSet or remove ZSet when weight = 0
                    tb.apply_delta(delta);
                    //opti: SmolStr and resize changed_tables size delta.len()?
                    changed_tables.push(table.to_string());
                }
            }
            info!(target: "ssp_module", "Changed tables: {:?}", changed_tables);
            changed_tables
        };

        // Optimized Lazy Rebuild Check (once per batch)
        {
            let _span = tracing::debug_span!(target: "ssp_module", "check_dependency_graph").entered();
            debug!(target: "ssp_module", "Dependency graph: {:?}, views: {}", self.dependency_graph, self.views.len());
            if self.dependency_graph.is_empty() && !self.views.is_empty() {
                self.rebuild_dependency_graph();
                info!(target: "ssp_module", "Dependency graph rebuilt");
            }
        }

        // Identify ALL affected views from ALL changed tables
        let impacted_view_indices = {
            let _span = tracing::debug_span!(target: "ssp_module", "find_impacted_views", changed_tables = changed_tables.len()).entered();
            let mut impacted_view_indices: Vec<usize> = Vec::with_capacity(self.views.len());
            
            info!(target: "ssp_module", "Changed tables: {:?}", changed_tables);

            for table in changed_tables {
                if let Some(indices) = self.dependency_graph.get(&table) {
                    impacted_view_indices.extend(indices.iter().copied());
                } else {
                    info!(target: "ssp_module", "Table {} changed, but no views depend on it", table);
                }
            }

            // Deduplicate View Indices (Sort + Dedup)
            // This ensures each view is processed EXACTLY ONCE, even if multiple input tables changed
            // opti: remove sort and dedup use HashSet its faster when many views
            impacted_view_indices.sort_unstable();
            impacted_view_indices.dedup();
            impacted_view_indices
        };

        // 3. Execution Phase - Group all view processing under one span
        {
            let _span = tracing::debug_span!(target: "ssp_module", "process_views", view_count = impacted_view_indices.len()).entered();
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
                            view.process_ingest_with_meta(deltas_ref, db_ref, batch_meta)
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
                        if let Some(update) = view.process_ingest_with_meta(deltas_ref, db_ref, batch_meta) {
                            ups.push(update);
                        }
                    }
                }
                ups
            }
        }
    }

    /// Helper to enrich ViewUpdates with metadata from the batch
    fn enrich_updates_with_meta(
        &self,
        updates: Vec<ViewUpdate>,
        batch_meta: Option<&BatchMeta>
    ) -> Vec<ViewUpdate> {
        // If no batch meta, nothing to enrich
        let meta = match batch_meta {
            Some(m) if !m.records.is_empty() => m,
            _ => return updates,
        };

        updates.into_iter().map(|mut update| {
            match &mut update {
                ViewUpdate::Streaming(streaming) => {
                    for record in &mut streaming.records {
                        // Only enrich if metadata is missing (it's initialized to None in View)
                        if record.metadata.is_none() {
                            if let Some(record_meta) = meta.records.get(record.id.as_str()) {
                                record.metadata = Some(record_meta.clone());
                            }
                        }
                    }
                },
                ViewUpdate::Flat(mat) | ViewUpdate::Tree(mat) => {
                    for (id, _, metadata) in &mut mat.result_data {
                        if metadata.is_none() {
                            if let Some(record_meta) = meta.records.get(id.as_str()) {
                                *metadata = Some(record_meta.clone());
                            }
                        }
                    }
                    // Recompute hash because metadata changed?
                    // The hash function now includes metadata.
                    // But we modified the vector in place. The hash in struct is stale.
                    mat.result_hash = super::update::compute_flat_hash(&mat.result_data);
                }
            }
            update
        }).collect()
    }

    // Must be called after Deserialization to rebuild the Cache!
    // opti: q: dont remove cache for performance so i dont have to rebuild it?
    // it just have to be rebuild when i Deserializat circuit JSON -> Struct
    #[instrument(target = "ssp_module", level = "debug", ret(level = "debug"))]
    pub fn rebuild_dependency_graph(&mut self) {
        self.dependency_graph.clear();

        info!(target: "ssp_module", "Rebuilding dependency graph for {} views", self.views.len());

        for (i, view) in self.views.iter().enumerate() {
            let tables = view.plan.root.referenced_tables();

            for t in tables {
                // q: what exectly is the value i?
                self.dependency_graph.entry(t).or_default().push(i);
            }
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
    #[instrument(target = "ssp_module", level = "debug", skip(self, plan, params))]
    pub fn register_view_with_strategy(
        &mut self,
        plan: QueryPlan,
        params: Option<Value>,
        format: Option<ViewResultFormat>,
        strategy: Option<IngestStrategy>,
    ) -> Option<ViewUpdate> {
        // Remove existing view if present
        {
            let _span = tracing::debug_span!(target: "ssp_module", "remove_existing_view").entered();
            if let Some(pos) = self.views.iter().position(|v| v.plan.id == plan.id) {
                self.views.remove(pos);
                self.rebuild_dependency_graph();
            }
        }

        // Create new view
        let mut view = {
            let _span = tracing::debug_span!(target: "ssp_module", "create_view").entered();
            View::new_with_strategy(
                plan, 
                params, 
                format.clone(), 
                strategy.unwrap_or_else(|| match format {
                    Some(ViewResultFormat::Tree) => IngestStrategy::HashBased,
                    _ => IngestStrategy::Optimistic,
                })
            )
        };

        // Process initial snapshot
        let initial_update = {
            let _span = tracing::debug_span!(target: "ssp_module", "initial_snapshot").entered();
            let empty_deltas: FastMap<String, ZSet> = FastMap::default();
            view.process_ingest(&empty_deltas, &self.db)
        };

        // Add view and update dependency graph
        {
            let _span = tracing::debug_span!(target: "ssp_module", "update_dependencies").entered();
            let view_idx = self.views.len();
            self.views.push(view);

            if let Some(v) = self.views.last() {
                let tables = v.plan.root.referenced_tables();
                for t in tables {
                    self.dependency_graph.entry(t).or_default().push(view_idx);
                }
            }
        }

        initial_update
    }

    #[allow(dead_code)]
    pub fn unregister_view(&mut self, id: &str) {
        self.views.retain(|v| v.plan.id != id);
        self.rebuild_dependency_graph();
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
