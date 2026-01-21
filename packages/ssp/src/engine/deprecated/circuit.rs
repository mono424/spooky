use crate::engine::circuit::{BatchEntry, Circuit, Operation};
use crate::engine::metadata::{BatchMeta, VersionStrategy};
use crate::engine::types::{FastMap, SpookyValue, ZSet};
use crate::engine::update::ViewUpdate;
use serde_json::Value;
use smol_str::SmolStr;

impl Circuit {
    #[deprecated(note = "Use ingest() instead")]
    pub fn ingest_entries(
        &mut self,
        entries: Vec<BatchEntry>,
        is_optimistic: bool,
    ) -> Vec<ViewUpdate> {
        self.ingest_entries_internal(entries, None, is_optimistic)
    }

    #[deprecated(note = "Use ingest() instead")]
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
            vec![BatchEntry::new(
                table,
                op,
                id,
                SpookyValue::from(record),
                hash.to_string(),
            )],
            is_optimistic,
        )
    }

    #[deprecated(note = "Use ingest() instead")]
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

    #[deprecated(note = "Use ingest() instead")]
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

    #[deprecated(note = "Use ingest() instead")]
    pub fn ingest_batch_with_meta(
        &mut self,
        batch: Vec<(SmolStr, SmolStr, SmolStr, SpookyValue, String)>,
        batch_meta: Option<&BatchMeta>,
        is_optimistic: bool,
    ) -> Vec<ViewUpdate> {
        let entries: Vec<BatchEntry> = batch
            .into_iter()
            .filter_map(|(t, o, i, r, h)| {
                let op = Operation::from_str(&o)?;
                let mut entry = BatchEntry::new(t, op, i.clone(), r, h);
                if let Some(meta) = batch_meta {
                    if let Some(record_meta) = meta.get(i.as_str()) {
                        entry = entry.with_meta(record_meta.clone());
                    }
                }
                Some(entry)
            })
            .collect();

        let strategy = batch_meta.map(|m| m.default_strategy.clone());
        self.ingest_entries_internal(entries, strategy, is_optimistic)
    }

    // SINGLE internal implementation
    #[deprecated(note = "Use ingest() instead")]
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

    #[deprecated(note = "use ingest() instead")]
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
}