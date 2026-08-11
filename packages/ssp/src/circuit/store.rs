use crate::algebra::{Weight, ZSet};
use crate::types::{make_key, raw_id, Sp00kyValue};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A base collection (table) in the store.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Collection {
    pub name: String,
    /// Z-set tracking record membership and weights.
    pub zset: ZSet,
    /// Actual record data, keyed by raw record ID (without table prefix).
    pub rows: HashMap<String, Sp00kyValue>,
    /// Incremental XOR set-hash of `rows`, maintained in lockstep with every
    /// mutation (see `apply_mutation`) so it can never drift from the actual
    /// rows. The scheduler reconstructs the same hash at the catch-up cut to
    /// verify a rejoining SSP. Not serialized — re-seeded after a bulk load via
    /// [`reseed_catchup_xor`].
    #[serde(skip)]
    pub catchup_xor: [u8; 32],
    /// Reusable canonicalization buffer for digest computation. Kept on the
    /// collection so a bulk re-seed (or a steady stream of mutations) reuses
    /// one allocation instead of one per row. Never serialized; carries no
    /// state between calls beyond its capacity.
    #[serde(skip)]
    scratch: Vec<u8>,
}

impl Collection {
    pub fn new(name: String) -> Self {
        Self {
            name,
            zset: HashMap::new(),
            rows: HashMap::new(),
            catchup_xor: ssp_protocol::snapshot_hash::xor_empty(),
            scratch: Vec::new(),
        }
    }

    /// Recompute the incremental XOR accumulator from the current rows. Call
    /// after a bulk load (`Circuit::load`) or a deserialize, where the per-row
    /// `apply_mutation` maintenance didn't run.
    pub fn reseed_catchup_xor(&mut self) {
        let mut acc = ssp_protocol::snapshot_hash::xor_empty();
        let mut scratch = std::mem::take(&mut self.scratch);
        for (id, val) in &self.rows {
            let digest = val.record_digest_into(id, &mut scratch);
            ssp_protocol::snapshot_hash::xor_digest(&mut acc, &digest);
        }
        self.scratch = scratch;
        self.catchup_xor = acc;
    }

    /// Digest of the row currently stored under `id`, if any.
    ///
    /// Takes `&mut self` only to borrow `scratch`; the rows are not modified.
    fn stored_digest(&mut self, id: &str) -> Option<[u8; 32]> {
        let mut scratch = std::mem::take(&mut self.scratch);
        let digest = self
            .rows
            .get(id)
            .map(|row| row.record_digest_into(id, &mut scratch));
        self.scratch = scratch;
        digest
    }

    /// Fold a row into the catch-up accumulator without storing it. For bulk
    /// loads, where there is never a prior value to XOR out.
    pub fn xor_in_row(&mut self, id: &str, value: &Sp00kyValue) {
        let digest = self.incoming_digest(id, value);
        ssp_protocol::snapshot_hash::xor_digest(&mut self.catchup_xor, &digest);
    }

    /// Digest of a value that is about to be stored under `id`.
    fn incoming_digest(&mut self, id: &str, value: &Sp00kyValue) -> [u8; 32] {
        let mut scratch = std::mem::take(&mut self.scratch);
        let digest = value.record_digest_into(id, &mut scratch);
        self.scratch = scratch;
        digest
    }

    /// Apply a mutation to this collection. Returns (zset_key, weight).
    pub fn apply_mutation(
        &mut self,
        op: Operation,
        id: &str,
        data: Sp00kyValue,
    ) -> (String, Weight) {
        let weight = op.weight();
        let normalized = raw_id(id);
        // Maintain the incremental XOR set-hash atomically with the row change:
        // XOR out the prior value (if any), XOR in the new value. Done here, the
        // single row-mutation chokepoint, so the accumulator cannot miss an op.
        //
        // Both digests are taken straight off the values via the canonical
        // writer. The previous shape deep-cloned the stored row, then built a
        // throwaway `serde_json::Value` tree from each of the prior and the
        // incoming value (and `record_digest` cloned again internally) — five
        // full copies of a row body to update 32 bytes of accumulator, on
        // every single mutation.
        let prior_digest = self.stored_digest(normalized);
        if let Some(d) = prior_digest {
            ssp_protocol::snapshot_hash::xor_digest(&mut self.catchup_xor, &d);
        }
        match op {
            Operation::Create | Operation::Update => {
                let d = self.incoming_digest(normalized, &data);
                ssp_protocol::snapshot_hash::xor_digest(&mut self.catchup_xor, &d);
                self.rows.insert(normalized.to_string(), data);
            }
            Operation::Delete => {
                self.rows.remove(normalized);
            }
        }

        let key = make_key(&self.name, id);
        if weight != 0 {
            let entry = self.zset.entry(key.clone()).or_insert(0);
            *entry += weight;
            if *entry == 0 {
                self.zset.remove(&key);
            }
        }
        (key, weight)
    }

    /// Look up a row by its raw ID.
    pub fn get_row(&self, id: &str) -> Option<&Sp00kyValue> {
        self.rows.get(raw_id(id))
    }

    /// Approximate heap bytes held by `rows`: the bucket array, every raw-id
    /// key, and every row body.
    pub fn rows_bytes(&self) -> usize {
        crate::size::map_table_bytes::<String, Sp00kyValue>(self.rows.capacity())
            + self
                .rows
                .iter()
                .map(|(id, row)| id.capacity() + row.heap_bytes())
                .sum::<usize>()
    }

    /// Approximate heap bytes held by `zset`. Reported apart from
    /// [`rows_bytes`] because its keys are a second, independently allocated
    /// `"table:id"` string per row on top of the raw id already in `rows`.
    pub fn zset_bytes(&self) -> usize {
        crate::size::zset_bytes(&self.zset)
    }

    /// Get the version of a record from its `_00_rv` field.
    pub fn get_record_version(&self, id: &str) -> Option<i64> {
        self.rows
            .get(raw_id(id))?
            .get("_00_rv")?
            .as_f64()
            .map(|n| n as i64)
    }
}

/// The store holds all base collections (tables).
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Store {
    pub collections: HashMap<String, Collection>,
    /// Transient, per-step overlay of rows being deleted in the CURRENT step,
    /// keyed by full `table:id`. A `Delete` removes the row from `collections`
    /// immediately (the subquery/join path relies on it being gone), but the
    /// Filter/Scan predicate evaluation still needs the row's content to decide
    /// whether the retraction (`-1`) belongs to the view. Predicate evaluation
    /// consults this overlay as a fallback; nothing else does. Populated before
    /// `apply_change` and cleared after the step. Never serialized.
    #[serde(skip)]
    pub pending_deleted_rows: HashMap<String, Sp00kyValue>,
}

impl Store {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn ensure_collection(&mut self, name: &str) -> &mut Collection {
        self.collections
            .entry(name.to_string())
            .or_insert_with(|| Collection::new(name.to_string()))
    }

    pub fn get_collection(&self, name: &str) -> Option<&Collection> {
        self.collections.get(name)
    }

    /// Apply a mutation whose row body is already owned. Returns
    /// (zset_key, weight).
    ///
    /// The hot ingest path uses this so the incoming body moves into the
    /// collection; [`apply_change`] clones because it only has a borrow.
    pub fn apply_owned(
        &mut self,
        table: &str,
        op: Operation,
        id: &str,
        data: Sp00kyValue,
    ) -> (String, Weight) {
        self.ensure_collection(table).apply_mutation(op, id, data)
    }

    /// Apply a borrowed Change to the store. Returns (zset_key, weight).
    ///
    /// Convenience for callers holding a `&Change` (tests, mostly); it has to
    /// clone the body. Prefer [`apply_owned`] where the body is owned.
    pub fn apply_change(&mut self, change: &Change) -> (String, Weight) {
        self.apply_owned(&change.table, change.op, &change.id, change.data.clone())
    }

    /// Get row data by zset key (format "table:id").
    pub fn get_row_by_key(&self, key: &str) -> Option<&Sp00kyValue> {
        let (table, id) = crate::types::parse_key(key)?;
        let coll = self.collections.get(table)?;
        // Try raw ID first, then with table prefix
        coll.rows.get(id).or_else(|| coll.rows.get(key))
    }

    /// Like [`get_row_by_key`], but falls back to a row staged for deletion in
    /// the current step (see [`pending_deleted_rows`]). Used ONLY by predicate
    /// evaluation so a delete's `-1` retraction can be tested against the WHERE
    /// clause even though the row is already gone from `collections`.
    pub fn get_row_by_key_or_deleted(&self, key: &str) -> Option<&Sp00kyValue> {
        self.get_row_by_key(key)
            .or_else(|| self.pending_deleted_rows.get(key))
    }

    /// Stage a row's content before a `Delete` removes it, so predicate
    /// evaluation in this step can still read it. No-op if the row is absent.
    pub fn stage_deleted_row(&mut self, table: &str, id: &str) {
        let key = make_key(table, id);
        if let Some(row) = self.get_row_by_key(&key).cloned() {
            self.pending_deleted_rows.insert(key, row);
        }
    }

    /// Clear the per-step deleted-row overlay (call after stepping).
    pub fn clear_pending_deleted_rows(&mut self) {
        self.pending_deleted_rows.clear();
    }

    /// Get the version of a record by its zset key (format "table:id").
    pub fn get_record_version_by_key(&self, key: &str) -> Option<i64> {
        let (table, id) = crate::types::parse_key(key)?;
        self.collections.get(table)?.get_record_version(id)
    }
}

/// A single record mutation.
#[derive(Debug, Clone)]
pub struct Change {
    pub table: String,
    pub op: Operation,
    pub id: String,
    pub data: Sp00kyValue,
}

impl Change {
    pub fn create(table: &str, id: &str, data: impl Into<Sp00kyValue>) -> Self {
        Self {
            table: table.to_string(),
            op: Operation::Create,
            id: id.to_string(),
            data: data.into(),
        }
    }

    pub fn update(table: &str, id: &str, data: impl Into<Sp00kyValue>) -> Self {
        Self {
            table: table.to_string(),
            op: Operation::Update,
            id: id.to_string(),
            data: data.into(),
        }
    }

    pub fn delete(table: &str, id: &str) -> Self {
        Self {
            table: table.to_string(),
            op: Operation::Delete,
            id: id.to_string(),
            data: Sp00kyValue::Null,
        }
    }
}

/// A batch of changes to apply in a single step.
#[derive(Debug, Clone, Default)]
pub struct ChangeSet {
    pub changes: Vec<Change>,
}

/// A record for initial bulk loading.
#[derive(Debug, Clone)]
pub struct Record {
    pub table: String,
    pub id: String,
    pub data: Sp00kyValue,
}

impl Record {
    pub fn new(table: &str, id: &str, data: impl Into<Sp00kyValue>) -> Self {
        Self {
            table: table.to_string(),
            id: id.to_string(),
            data: data.into(),
        }
    }
}

/// Mutation type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Create,
    Update,
    Delete,
}

impl Operation {
    pub fn weight(&self) -> Weight {
        match self {
            Operation::Create => 1,
            Operation::Update => 0,
            Operation::Delete => -1,
        }
    }

    pub fn changes_content(&self) -> bool {
        matches!(self, Operation::Create | Operation::Update)
    }

    /// Parse an operation from a string (case-insensitive).
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_uppercase().as_str() {
            "CREATE" => Some(Operation::Create),
            "UPDATE" => Some(Operation::Update),
            "DELETE" => Some(Operation::Delete),
            _ => None,
        }
    }
}
