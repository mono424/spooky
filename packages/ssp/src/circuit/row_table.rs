//! A table's rows, stored as flat bytes behind an index.
//!
//! Replaces the `HashMap<String, Sp00kyValue>` the store used to keep. That
//! map cost roughly six times the rows' own JSON, mostly in per-row per-field
//! `String` allocations for field names that a table only has a few dozen of.
//! Here the names live once in a [`FieldDict`], the bodies live in an
//! [`Arena`] as encoded bytes, and what stays on the heap per row is an index
//! entry: an id and a [`RowSlot`].
//!
//! # What this does not fix
//!
//! The index is still O(rows) and still resident. It is much smaller than the
//! rows were, but it does not go away, so very large tables remain bounded by
//! it. Sizing it is reported separately by [`RowTable::index_bytes`] precisely
//! so that floor stays visible rather than hiding inside a total.

use crate::circuit::arena::{Arena, HeapArena, Span};
use crate::circuit::row_codec::{self as codec, FieldDict};
use crate::eval::value_ref::{FlatRef, ValueRef};
use crate::types::Sp00kyValue;
use rustc_hash::FxHashMap;

/// Where one row's encoded bytes live.
pub type RowSlot = Span;

/// Rows of one table.
#[derive(Debug)]
pub struct RowTable {
    dict: FieldDict,
    index: FxHashMap<Box<str>, RowSlot>,
    arena: Box<dyn Arena>,
    /// Reused encode buffer, so a steady stream of writes does not allocate
    /// one per row.
    scratch: Vec<u8>,
}

impl Default for RowTable {
    fn default() -> Self {
        Self::new()
    }
}

impl RowTable {
    pub fn new() -> Self {
        Self::with_arena(Box::new(HeapArena::new()))
    }

    /// Build a table over a caller-supplied arena.
    ///
    /// The seam that lets the row bytes live somewhere other than the heap —
    /// a file mapping, on platforms that have one — without any operator or
    /// codec change.
    pub fn with_arena(arena: Box<dyn Arena>) -> Self {
        Self {
            dict: FieldDict::new(),
            index: FxHashMap::default(),
            arena,
            scratch: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.index.len()
    }

    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    pub fn contains_key(&self, id: &str) -> bool {
        self.index.contains_key(id)
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> + '_ {
        self.index.keys().map(|k| &**k)
    }

    pub fn dict(&self) -> &FieldDict {
        &self.dict
    }

    /// Borrow a row. [`ValueRef::Missing`] when absent.
    pub fn get(&self, id: &str) -> ValueRef<'_> {
        match self.record(id) {
            Some(rec) => FlatRef {
                bytes: codec::record_value(rec),
                dict: &self.dict,
            }
            .value(),
            None => ValueRef::Missing,
        }
    }

    /// The raw record (header + value) for a row.
    fn record(&self, id: &str) -> Option<&[u8]> {
        let slot = *self.index.get(id)?;
        let bytes = self.arena.get(slot);
        (!bytes.is_empty()).then_some(bytes)
    }

    /// The digest stored in a row's header.
    ///
    /// This is why writes no longer canonicalize anything: the digest was
    /// computed once, at insert, and is read back as 32 bytes.
    pub fn digest_of(&self, id: &str) -> Option<[u8; codec::DIGEST_LEN]> {
        codec::record_digest(self.record(id)?).copied()
    }

    /// A row's lifted `_00_rv`. Two loads at a fixed offset — no decode.
    pub fn rv_of(&self, id: &str) -> Option<i64> {
        codec::record_rv(self.record(id)?)
    }

    /// Highest `_00_rv` across the table, or `None` if no row carries one.
    pub fn max_rv(&self) -> Option<i64> {
        self.index
            .values()
            .filter_map(|slot| codec::record_rv(self.arena.get(*slot)))
            .max()
    }

    /// Insert or replace a row, returning the digest that was stored.
    ///
    /// `digest` comes from the caller rather than being computed here so it
    /// always originates from the canonical writer that is pinned against the
    /// `serde_json` hash pipeline.
    pub fn insert(
        &mut self,
        id: &str,
        value: &Sp00kyValue,
        digest: &[u8; codec::DIGEST_LEN],
    ) -> RowSlot {
        let mut scratch = std::mem::take(&mut self.scratch);
        codec::encode_record(value, digest, &mut self.dict, &mut scratch);
        let slot = self.arena.append(&scratch);
        self.scratch = scratch;
        if let Some(old) = self.index.insert(id.into(), slot) {
            self.arena.free(old);
        }
        slot
    }

    /// Remove a row. Returns whether it was present.
    pub fn remove(&mut self, id: &str) -> bool {
        match self.index.remove(id) {
            Some(slot) => {
                self.arena.free(slot);
                true
            }
            None => false,
        }
    }

    pub fn clear(&mut self) {
        self.index.clear();
        self.arena.clear();
        self.dict = FieldDict::new();
    }

    /// Iterate `(raw_id, row)` pairs. Order is unspecified.
    pub fn iter(&self) -> impl Iterator<Item = (&str, ValueRef<'_>)> + '_ {
        self.index.iter().map(move |(id, slot)| {
            let value = FlatRef {
                bytes: codec::record_value(self.arena.get(*slot)),
                dict: &self.dict,
            }
            .value();
            (&**id, value)
        })
    }

    /// Heap held by the index: the id strings and the bucket array. Reported
    /// apart from the arena because this is the part that does not shrink.
    pub fn index_bytes(&self) -> usize {
        crate::size::map_table_bytes::<Box<str>, RowSlot>(self.index.capacity())
            + self.index.keys().map(|k| k.len()).sum::<usize>()
    }

    /// Bytes held by the encoded rows themselves, including space orphaned by
    /// updates and deletes and not yet reclaimed.
    pub fn arena_bytes(&self) -> usize {
        self.arena.capacity_bytes() as usize
    }

    pub fn dict_bytes(&self) -> usize {
        self.dict.heap_bytes()
    }

    /// Total approximate heap for this table's rows.
    pub fn heap_bytes(&self) -> usize {
        self.index_bytes() + self.arena_bytes() + self.dict_bytes()
    }

    /// Bytes orphaned by updates and deletes — what a compaction pass would
    /// reclaim.
    pub fn dead_bytes(&self) -> u64 {
        self.arena.dead_bytes()
    }
}

impl Clone for RowTable {
    /// Rebuilds through the encoder rather than copying the arena, so a clone
    /// is compact even if the original had accumulated dead bytes.
    fn clone(&self) -> Self {
        let mut out = RowTable::new();
        for (id, slot) in &self.index {
            let record = self.arena.get(*slot);
            let Some(digest) = codec::record_digest(record).copied() else {
                continue;
            };
            if let Some(value) = codec::decode_value(codec::record_value(record), &self.dict) {
                out.insert(id, &value, &digest);
            }
        }
        out
    }
}

// --- serialization ---
//
// Serialized as `{ raw_id: <row as JSON> }`, i.e. exactly the shape the old
// `HashMap<String, Sp00kyValue>` produced. Keeping that shape means a snapshot
// written before this change still deserializes, and the flat encoding stays
// an in-memory detail rather than an on-disk format that would have to be
// versioned and migrated.
//
// Digests are NOT serialized: they are recomputed on load from the canonical
// writer, which is the single source of truth for them.

impl serde::Serialize for RowTable {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(self.index.len()))?;
        for (id, value) in self.iter() {
            map.serialize_entry(id, &value.to_owned_value())?;
        }
        map.end()
    }
}

impl<'de> serde::Deserialize<'de> for RowTable {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use std::collections::HashMap;
        let rows: HashMap<String, Sp00kyValue> = HashMap::deserialize(deserializer)?;
        let mut table = RowTable::new();
        let mut scratch = Vec::new();
        for (id, value) in rows {
            let digest = value.record_digest_into(&id, &mut scratch);
            table.insert(&id, &value, &digest);
        }
        Ok(table)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sv(j: serde_json::Value) -> Sp00kyValue {
        Sp00kyValue::from(j)
    }

    fn digest_of(id: &str, v: &Sp00kyValue) -> [u8; codec::DIGEST_LEN] {
        let mut scratch = Vec::new();
        v.record_digest_into(id, &mut scratch)
    }

    fn put(t: &mut RowTable, id: &str, j: serde_json::Value) -> Sp00kyValue {
        let v = sv(j);
        let d = digest_of(id, &v);
        t.insert(id, &v, &d);
        v
    }

    #[test]
    fn insert_then_read_back() {
        let mut t = RowTable::new();
        let v = put(&mut t, "a", json!({ "title": "x", "n": 1, "nested": { "k": [1, 2] } }));
        assert_eq!(t.len(), 1);
        assert!(t.contains_key("a"));
        assert_eq!(t.get("a").to_owned_value(), v);
        assert!(t.get("nope").is_missing());
    }

    #[test]
    fn field_lookup_matches_the_owned_value() {
        let mut t = RowTable::new();
        let v = put(&mut t, "a", json!({ "s": "str", "i": 5, "f": 1.5, "b": true, "n": null }));
        let row = t.get("a");
        for key in ["s", "i", "f", "b", "n"] {
            assert_eq!(
                row.get(key).to_owned_value(),
                *v.get(key).unwrap(),
                "field {key} mismatched"
            );
        }
        assert!(row.get("absent").is_missing());
    }

    #[test]
    fn update_replaces_and_frees_the_old_bytes() {
        let mut t = RowTable::new();
        put(&mut t, "a", json!({ "v": 1 }));
        assert_eq!(t.dead_bytes(), 0);
        let second = put(&mut t, "a", json!({ "v": 2 }));
        assert_eq!(t.len(), 1, "update must not add a row");
        assert_eq!(t.get("a").to_owned_value(), second);
        assert!(t.dead_bytes() > 0, "the superseded record must be accounted dead");
    }

    #[test]
    fn remove_drops_the_row() {
        let mut t = RowTable::new();
        put(&mut t, "a", json!({ "v": 1 }));
        assert!(t.remove("a"));
        assert!(!t.remove("a"), "removing twice reports absent");
        assert_eq!(t.len(), 0);
        assert!(t.get("a").is_missing());
    }

    #[test]
    fn digest_is_stored_and_matches_the_canonical_writer() {
        let mut t = RowTable::new();
        let v = put(&mut t, "a", json!({ "x": 1, "_00_rv": 9, "gone": null }));
        assert_eq!(t.digest_of("a"), Some(digest_of("a", &v)));
        assert_eq!(t.digest_of("missing"), None);
    }

    #[test]
    fn rv_is_readable_without_decoding() {
        let mut t = RowTable::new();
        put(&mut t, "a", json!({ "_00_rv": 7 }));
        put(&mut t, "b", json!({ "_00_rv": 12 }));
        put(&mut t, "c", json!({ "no": "rv" }));
        assert_eq!(t.rv_of("a"), Some(7));
        assert_eq!(t.rv_of("c"), None);
        assert_eq!(t.max_rv(), Some(12));

        let empty = RowTable::new();
        assert_eq!(empty.max_rv(), None);
    }

    #[test]
    fn iter_yields_every_row() {
        let mut t = RowTable::new();
        for i in 0..16 {
            put(&mut t, &format!("r{i}"), json!({ "n": i }));
        }
        let mut seen: Vec<(String, i64)> = t
            .iter()
            .map(|(id, v)| (id.to_string(), v.get("n").as_i64().unwrap()))
            .collect();
        seen.sort();
        assert_eq!(seen.len(), 16);
        assert_eq!(seen[0], ("r0".to_string(), 0));
    }

    #[test]
    fn serde_round_trip_preserves_content_and_digests() {
        let mut t = RowTable::new();
        for i in 0..8 {
            put(
                &mut t,
                &format!("r{i}"),
                json!({ "n": i, "s": format!("v{i}"), "_00_rv": i, "nil": null }),
            );
        }
        let json = serde_json::to_string(&t).unwrap();
        let back: RowTable = serde_json::from_str(&json).unwrap();

        assert_eq!(back.len(), t.len());
        for i in 0..8 {
            let id = format!("r{i}");
            assert_eq!(
                back.get(&id).to_owned_value(),
                t.get(&id).to_owned_value(),
                "row {id} changed across serde"
            );
            assert_eq!(back.digest_of(&id), t.digest_of(&id), "digest for {id}");
            assert_eq!(back.rv_of(&id), t.rv_of(&id));
        }
    }

    /// The on-disk shape must be byte-for-byte what the old
    /// `HashMap<String, Sp00kyValue>` produced, or every existing snapshot
    /// stops loading and every warm restart becomes a cold rebuild.
    ///
    /// Note what that shape actually is: `Sp00kyValue`'s derived `Serialize`
    /// is externally tagged, so a row is `{"Object":{"n":{"Int":1}}}`, not
    /// plain JSON. The flat encoding is an in-memory detail and deliberately
    /// does not reach the snapshot.
    #[test]
    fn serialized_shape_matches_the_previous_hashmap_encoding() {
        let mut t = RowTable::new();
        put(&mut t, "abc", json!({ "n": 1 }));

        // What the old `HashMap<String, Sp00kyValue>` field would have emitted.
        let legacy: std::collections::HashMap<String, Sp00kyValue> =
            [("abc".to_string(), sv(json!({ "n": 1 })))].into_iter().collect();
        let expected: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&legacy).unwrap()).unwrap();
        let actual: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&t).unwrap()).unwrap();
        assert_eq!(actual, expected);
    }

    /// A snapshot in the pre-flat-encoding format must still load.
    #[test]
    fn deserializes_a_legacy_snapshot_payload() {
        let legacy = r#"{"abc":{"Object":{"n":{"Int":1}}},"def":{"Object":{"s":{"Str":"x"}}}}"#;
        let t: RowTable = serde_json::from_str(legacy).unwrap();
        assert_eq!(t.len(), 2);
        assert_eq!(t.get("abc").get("n").as_i64(), Some(1));
        assert_eq!(t.get("def").get("s").as_str(), Some("x"));
        // Digests are recomputed on load rather than carried in the snapshot.
        assert_eq!(
            t.digest_of("abc"),
            Some(digest_of("abc", &sv(json!({ "n": 1 }))))
        );
    }

    #[test]
    fn clone_is_independent_and_compact() {
        let mut t = RowTable::new();
        put(&mut t, "a", json!({ "v": 1 }));
        // Churn so the original carries dead bytes.
        for i in 0..10 {
            put(&mut t, "a", json!({ "v": i }));
        }
        assert!(t.dead_bytes() > 0);

        let c = t.clone();
        assert_eq!(c.dead_bytes(), 0, "clone rebuilds compactly");
        assert_eq!(c.get("a").to_owned_value(), t.get("a").to_owned_value());
        assert_eq!(c.digest_of("a"), t.digest_of("a"));

        // Mutating the clone must not touch the original.
        let mut c = c;
        c.remove("a");
        assert!(t.contains_key("a"));
    }

    #[test]
    fn clear_empties_everything() {
        let mut t = RowTable::new();
        put(&mut t, "a", json!({ "v": 1 }));
        t.clear();
        assert_eq!(t.len(), 0);
        assert!(t.get("a").is_missing());
        assert_eq!(t.dead_bytes(), 0);
    }

    #[test]
    fn field_names_are_stored_once_per_table_not_once_per_row() {
        // The entire point of the dictionary. 200 rows sharing 4 field names
        // must intern 4 entries, not 800.
        let mut t = RowTable::new();
        for i in 0..200 {
            put(
                &mut t,
                &format!("r{i}"),
                json!({ "alpha": i, "beta": "x", "gamma": true, "delta": null }),
            );
        }
        assert_eq!(t.dict().len(), 4);
    }
}
