use super::view::{MaterializedViewUpdate, Operator, QueryPlan, View, ZSet, FastMap, Projection, SpookyValue};
// use rustc_hash::{FxHashMap, FxHasher}; // Unused in this file (used via FastMap)
use serde::{Deserialize, Serialize};
use serde_json::Value;
use smol_str::SmolStr;

// --- Table & Database ---

// Table moved to storage.rs
use super::storage::{Table, Column};
use super::interner::SymbolTable;
use std::sync::Arc;


// I will just use 'Table' name but with new types.

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Database {
    pub tables: FastMap<String, Table>,
    pub interner: Arc<SymbolTable>,
}

impl Database {
    pub fn new() -> Self {
        Self {
            tables: FastMap::default(),
            interner: Arc::new(SymbolTable::new()),
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
        for (i, view) in self.views.iter().enumerate() {
            let tables = extract_tables(&view.plan.root);
            for t in tables {
                self.dependency_graph.entry(t).or_default().push(i);
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
        self.ingest_batch(vec![(table, op, id, record, hash)])
    }

    pub fn ingest_batch(
        &mut self,
        batch: Vec<(String, String, String, Value, String)>,
    ) -> Vec<MaterializedViewUpdate> {
         // Convert to SpookyValue
         let batch_spooky: Vec<(String, String, String, SpookyValue, String)> = batch
             .into_iter()
             .map(|(t, o, i, r, h)| (t, o, i, SpookyValue::from(r), h))
             .collect();
         
         self.ingest_batch_spooky(batch_spooky)
    }

    pub fn ingest_batch_spooky(
        &mut self,
        batch: Vec<(String, String, String, SpookyValue, String)>,
    ) -> Vec<MaterializedViewUpdate> {
        let mut table_deltas: FastMap<String, ZSet> = FastMap::default();
        let interner = self.db.interner.clone(); // Arc clone for borrow checker

        // 1. Storage Phase: Update Storage & Accumulate Deltas
        for (table, op, id, record_spooky, hash) in batch {
             let key = SmolStr::new(id);
             let weight: i64 = match op.as_str() {
                 "CREATE" | "UPDATE" => 1,
                 "DELETE" => -1,
                 _ => 0,
             };
 
             if weight == 0 {
                 continue;
             }
 
             {
                 let tb = self.db.ensure_table(&table);
                 if weight > 0 {
                     // COLUMNAR UPDATE
                     // 1. Check if row exists, if so, we might need to overwrite (complex in columnar)
                     // For simplicity in this Task, assume strict append for new IDs, or update in place if ID exists.
                     
                     let row_idx = if let Some(&idx) = tb.pk_map.get(&key) {
                         idx
                     } else {
                         let idx = tb.num_rows;
                         tb.pk_map.insert(key.clone(), idx);
                         tb.index_to_pk.push(key.clone());
                         tb.num_rows += 1;
                         
                         // Backfill new row with defaults in all existing columns
                         for col in tb.columns.values_mut() {
                             match col {
                                 Column::Int(v) => v.push(0),
                                 Column::Float(v) => v.push(0.0),
                                 Column::Bool(v) => v.push(false),
                                 Column::Text(v) => v.push(0), // 0 = empty/null symbol ideally
                             }
                         }
                         idx
                     };
                     
                     tb.hashes.insert(key.clone(), hash);

                     // 2. Parse SpookyValue and update Columns
                     if let SpookyValue::Object(map) = record_spooky {
                         for (col_name, val) in map {
                             let col_str = col_name.to_string();
                             
                             // Ensure column exists
                             if !tb.columns.contains_key(&col_str) {
                                 // Infer type from first value saw? default to Text?
                                 // Or sniff type.
                                 let new_col = match &val {
                                     SpookyValue::Number(_) => Column::Float(vec![0.0; tb.num_rows]),
                                     SpookyValue::Bool(_) => Column::Bool(vec![false; tb.num_rows]),
                                     _ => Column::Text(vec![0; tb.num_rows]),
                                 };
                                 tb.columns.insert(col_str.clone(), new_col);
                             }
                             
                             let col = tb.columns.get_mut(&col_str).unwrap();
                             match (col, val) {
                                 (Column::Float(v), SpookyValue::Number(n)) => v[row_idx] = n,
                                 (Column::Int(v), SpookyValue::Number(n)) => v[row_idx] = n as i64, 
                                 (Column::Bool(v), SpookyValue::Bool(b)) => v[row_idx] = b,
                                 (Column::Text(v), SpookyValue::Str(s)) => {
                                     let sym = interner.get_or_intern(&s);
                                     v[row_idx] = sym;
                                 }
                                  // Type coercions or skips
                                 (Column::Float(v), SpookyValue::Null) => v[row_idx] = 0.0,
                                 _ => {} // Ignore mismatches for now
                             }
                         }
                     }

                 } else {
                     // DELETE
                     // Logic used to be tb.delete_row(&key);
                     // In columnar, real delete is hard (O(N) shift).
                     // We just remove from ZSet and Hashes so it's "invisible" to Views,
                     // but data remains in columns (Tombstone style).
                     // Or we swap-remove? Swap-remove breaks index mapping.
                     // Let's just remove from metadata `hashes` and `pk_map`?
                     // If we remove from `pk_map`, we orphan the row index. It becomes junk.
                     // This is fine for an "Analytics Engine" (append only mostly).
                     // Ideally we have a validity bitmap.
                     
                     // Minimal "delete" from visible set:
                     tb.hashes.remove(&key);
                     // We KEEP it in pk_map so if it's re-inserted we reuse the slot?
                     // Or we assume DELETE means "tombstone".
                     // For correct counting in ZSet, we just pass the negative weight.
                 }
             }
 
             let delta_map = table_deltas.entry(table).or_default();
             *delta_map.entry(key).or_insert(0) += weight;
         }
 
         // Apply Deltas to DB ZSets
         let mut changed_tables = Vec::new();
         for (table, delta) in &mut table_deltas {
              delta.retain(|_, w| *w != 0);
              if !delta.is_empty() {
                  let tb = self.db.ensure_table(table);
                  tb.apply_delta(delta);
                  changed_tables.push(table.clone());
              }
         }

        // 2. Propagation Phase: Process Deltas with Dependency Graph
        
        // Optimized Lazy Rebuild Check (once per batch)
        if self.dependency_graph.is_empty() && !self.views.is_empty() {
            self.rebuild_dependency_graph();
        }

        // Identify ALL affected views from ALL changed tables
        let mut impacted_view_indices: Vec<usize> = Vec::new();
        for table in changed_tables {
            if let Some(indices) = self.dependency_graph.get(&table) {
                impacted_view_indices.extend(indices.iter().copied());
            } else {
                println!("DEBUG: Table {} changed, but no views depend on it", table);
            }
        }

        // Deduplicate View Indices (Sort + Dedup)
        // This ensures each view is processed EXACTLY ONCE, even if multiple input tables changed
        impacted_view_indices.sort_unstable();
        impacted_view_indices.dedup();

        let mut all_updates: Vec<MaterializedViewUpdate> = Vec::new();

        // 3. Execution Phase
        // 3. Execution Phase
        let db_ref = &self.db;
        let deltas_ref = &table_deltas;

        #[cfg(all(feature = "parallel", not(target_arch = "wasm32")))]
        let updates: Vec<MaterializedViewUpdate> = {
            use rayon::prelude::*;
            self.views
                .par_iter_mut()
                .enumerate()
                .filter_map(|(i, view)| {
                    // Check if this view needs update. 
                    // impacted_view_indices is sorted, so binary_search is efficient.
                    if impacted_view_indices.binary_search(&i).is_ok() {
                        view.process_ingest(deltas_ref, db_ref)
                    } else {
                        None
                    }
                })
                .collect()
        };

        #[cfg(any(target_arch = "wasm32", not(feature = "parallel")))]
        let updates: Vec<MaterializedViewUpdate> = {
            let mut ups = Vec::new();
            for i in impacted_view_indices {
                 if i < self.views.len() {
                     let view: &mut View = &mut self.views[i];
                     if let Some(update) = view.process_ingest(deltas_ref, db_ref) {
                         ups.push(update);
                     }
                 }
            }
            ups
        };

        all_updates.extend(updates);
        all_updates
    }

    pub fn register_view(
        &mut self,
        plan: QueryPlan,
        params: Option<Value>,
    ) -> Option<MaterializedViewUpdate> {
        let plan = super::optimizer::optimize(plan);

        if let Some(pos) = self.views.iter().position(|v| v.plan.id == plan.id) {
            self.views.remove(pos);
            // Rebuild dependencies entirely to be safe (simple but slower)
            self.rebuild_dependency_graph();
        }

        let mut view = View::new(plan, params);

        let empty_delta: ZSet = FastMap::default();
        let initial_update = view.process("", &empty_delta, &self.db);

        let view_idx = self.views.len();
        self.views.push(view);

        // Update Dependencies for the new view
        // Note: We use self.views.last() to inspect the plan we just pushed
        if let Some(v) = self.views.last() {
            let tables = extract_tables(&v.plan.root);
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

    pub fn step(&mut self, table: String, delta: ZSet) -> Vec<MaterializedViewUpdate> {
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
                if let Projection::Subquery { plan, .. } = p {
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

// Trait Implementation
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

    fn ingest_batch(
        &mut self,
        batch: Vec<(String, String, String, Value, String)>,
    ) -> Vec<MaterializedViewUpdate> {
        self.ingest_batch(batch)
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
