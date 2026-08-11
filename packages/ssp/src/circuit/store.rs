use crate::algebra::{Weight, ZSet};
use crate::circuit::row_table::RowTable;
use crate::eval::value_ref::ValueRef;
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
    ///
    /// Flat-encoded behind an index rather than a map of parsed values — see
    /// [`RowTable`]. Serializes to the same shape as the `HashMap` it replaced,
    /// so existing snapshots still load.
    pub rows: RowTable,
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
        let rows = RowTable::with_arena(crate::circuit::arena::new_arena(&name));
        Self {
            name,
            zset: HashMap::new(),
            rows,
            catchup_xor: ssp_protocol::snapshot_hash::xor_empty(),
            scratch: Vec::new(),
        }
    }

    /// Recompute the incremental XOR accumulator from the current rows. Call
    /// after a bulk load (`Circuit::load`) or a deserialize, where the per-row
    /// `apply_mutation` maintenance didn't run.
    ///
    /// Reads each row's stored digest rather than re-canonicalizing it, so a
    /// re-seed is now a walk of 32-byte header reads.
    pub fn reseed_catchup_xor(&mut self) {
        let mut acc = ssp_protocol::snapshot_hash::xor_empty();
        let ids: Vec<&str> = self.rows.keys().collect();
        for id in ids {
            if let Some(digest) = self.rows.digest_of(id) {
                ssp_protocol::snapshot_hash::xor_digest(&mut acc, &digest);
            }
        }
        self.catchup_xor = acc;
    }

    /// Digest of a value about to be stored under `id`, via the canonical
    /// writer. Exposed so a bulk load can canonicalize once and reuse the
    /// result for both the accumulator and the record header.
    pub fn digest_for(&mut self, id: &str, value: &Sp00kyValue) -> [u8; 32] {
        self.incoming_digest(id, value)
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
        // The outgoing digest is read from the stored row's header — 32 bytes
        // at a fixed offset, no decode. The incoming one is computed once by
        // the canonical writer and handed to the row store, which keeps it in
        // the record it writes. So a mutation canonicalizes exactly once and
        // copies the body exactly once.
        if let Some(d) = self.rows.digest_of(normalized) {
            ssp_protocol::snapshot_hash::xor_digest(&mut self.catchup_xor, &d);
        }
        match op {
            Operation::Create | Operation::Update => {
                let d = self.incoming_digest(normalized, &data);
                ssp_protocol::snapshot_hash::xor_digest(&mut self.catchup_xor, &d);
                self.rows.insert(normalized, &data, &d);
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
    pub fn get_row(&self, id: &str) -> ValueRef<'_> {
        self.rows.get(raw_id(id))
    }

    /// Whether a row exists under this raw ID.
    pub fn has_row(&self, id: &str) -> bool {
        self.rows.contains_key(raw_id(id))
    }

    /// Approximate heap bytes held by `rows`: the id index, the encoded row
    /// bytes, and the field-name dictionary.
    pub fn rows_bytes(&self) -> usize {
        self.rows.heap_bytes()
    }

    /// Heap held by the row index alone. Broken out because it is the part
    /// that stays O(rows) and resident no matter how the bodies are stored.
    pub fn index_bytes(&self) -> usize {
        self.rows.index_bytes()
    }

    /// Approximate heap bytes held by `zset`. Reported apart from
    /// [`rows_bytes`] because its keys are a second, independently allocated
    /// `"table:id"` string per row on top of the raw id already in `rows`.
    pub fn zset_bytes(&self) -> usize {
        crate::size::zset_bytes(&self.zset)
    }

    /// Get the version of a record from its `_00_rv` field.
    ///
    /// Read from the record header, where the encoder lifts it, rather than by
    /// resolving a field out of the body.
    pub fn get_record_version(&self, id: &str) -> Option<i64> {
        self.rows.rv_of(raw_id(id))
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
    ///
    /// Returns [`ValueRef::Missing`] for an unparseable key, an unknown table,
    /// or an absent row — the caller does not distinguish those cases.
    pub fn get_row_by_key(&self, key: &str) -> ValueRef<'_> {
        let Some((table, id)) = crate::types::parse_key(key) else {
            return ValueRef::Missing;
        };
        let Some(coll) = self.collections.get(table) else {
            return ValueRef::Missing;
        };
        // Try raw ID first, then with table prefix
        let by_raw = coll.rows.get(id);
        if by_raw.is_missing() {
            coll.rows.get(key)
        } else {
            by_raw
        }
    }

    /// Like [`get_row_by_key`], but falls back to a row staged for deletion in
    /// the current step (see [`pending_deleted_rows`]). Used ONLY by predicate
    /// evaluation so a delete's `-1` retraction can be tested against the WHERE
    /// clause even though the row is already gone from `collections`.
    pub fn get_row_by_key_or_deleted(&self, key: &str) -> ValueRef<'_> {
        let live = self.get_row_by_key(key);
        if live.is_missing() {
            ValueRef::from_opt(self.pending_deleted_rows.get(key))
        } else {
            live
        }
    }

    /// Stage a row's content before a `Delete` removes it, so predicate
    /// evaluation in this step can still read it. No-op if the row is absent.
    ///
    /// This is the one place that materializes a stored row: the overlay
    /// outlives the row's bytes within the step, so it has to own its copy.
    /// It holds at most the rows deleted in a single step.
    pub fn stage_deleted_row(&mut self, table: &str, id: &str) {
        let key = make_key(table, id);
        let row = self.get_row_by_key(&key);
        if !row.is_missing() {
            let owned = row.to_owned_value();
            self.pending_deleted_rows.insert(key, owned);
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
