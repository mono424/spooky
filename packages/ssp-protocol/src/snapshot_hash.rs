//! Per-table content hashes used to detect drift between the upstream
//! SurrealDB, the scheduler's replica, and an SSP's circuit store.
//!
//! Both producers (scheduler replica, SSP circuit) feed `(raw_id, value)`
//! pairs through the same code path so digests are bit-identical when the
//! contents agree. Hash inputs are sorted by `raw_id` and JSON objects are
//! recursively key-sorted before serialization, so HashMap iteration order
//! and SurrealDB column ordering can never change the output.
//!
//! Output is lowercase hex of a blake3-256 digest, prefixed `b3:` so future
//! algorithm changes are visible in diagnostics.
//!
//! See `Replica::compute_table_hashes` (apps/scheduler/src/replica.rs) and
//! `Circuit::compute_table_hashes` (packages/ssp/src/circuit/circuit.rs).

use serde_json::Value;
use std::collections::BTreeMap;

const HASH_PREFIX: &str = "b3:";

/// Hash a table's contents. Iterator order does not matter; the function
/// sorts by `raw_id` internally.
pub fn hash_table<I>(records: I) -> String
where
    I: IntoIterator<Item = (String, Value)>,
{
    // Drop reserved `_00_*` metadata keys (notably `_00_rv`) before hashing so
    // the two producers stay bit-identical. The SSP circuit stores rows
    // verbatim from the ingest event payload, which the `_00_<table>_mutation`
    // DB events stamp with `_00_rv` (the record version, read back in
    // `Collection::get_record_version`). The scheduler replica instead hashes a
    // SCHEMAFULL `SELECT *`, which never returns that undefined field. Without
    // this, any table touched by event replay (e.g. `comment` after a write)
    // hashes differently on the two sides, fails the post-replay integrity
    // check, and force-re-bootstraps the SSP in an endless loop. See
    // `strip_reserved_keys`.
    let mut pairs: Vec<(String, Value)> = records
        .into_iter()
        .map(|(id, value)| (normalize_record_id(&id), strip_reserved_keys(value)))
        .collect();
    pairs.sort_by(|a, b| a.0.cmp(&b.0));

    let mut hasher = blake3::Hasher::new();
    for (id, value) in &pairs {
        hasher.update(id.as_bytes());
        hasher.update(b"\0");
        let canonical = canonical_json(value);
        hasher.update(&canonical);
        hasher.update(b"\0");
    }

    format!("{}{}", HASH_PREFIX, hasher.finalize().to_hex())
}

/// Strip SurrealDB's identifier escaping (`⟨...⟩` and backticks) from a record
/// id so both hash producers agree on one spelling. Table names that are not
/// plain identifiers (e.g. the `_00_*` synced meta tables, whose leading
/// underscore makes some serializers emit `⟨_00_app_release⟩:web` while others
/// emit `_00_app_release:web`) otherwise hash differently on the scheduler and
/// SSP sides and crash-loop the SSP on a false integrity mismatch.
pub fn normalize_record_id(id: &str) -> String {
    if id.contains('\u{27e8}') || id.contains('`') {
        id.chars().filter(|c| *c != '\u{27e8}' && *c != '\u{27e9}' && *c != '`').collect()
    } else {
        id.to_string()
    }
}

/// Hash for an empty table — useful when comparing tables that exist on one
/// side but not the other.
pub fn empty_table_hash() -> String {
    hash_table(std::iter::empty())
}

/// Normalize a record's top-level object so the scheduler replica and the SSP
/// circuit hash equal content identically. Two top-level keys are dropped:
///
/// 1. Reserved `_00_*` metadata keys — a reserved sp00ky prefix, never user
///    content. The SSP circuit keeps them (the ingest event payload stamps
///    `_00_rv`); the replica's SCHEMAFULL `SELECT *` never returns them.
/// 2. Keys whose value is `null` — treated as absent. The replica's SCHEMAFULL
///    `SELECT *` materializes an unset `option<...>` field as JSON `null`,
///    whereas the SSP circuit stores rows verbatim from the ingest event
///    payload, which OMITS unset optionals. Without this, any row with an unset
///    optional (e.g. `broadcast.target_renderer`, `renderer_device.owner`)
///    hashes differently on the two sides, fails the catch-up integrity check,
///    and force-re-bootstraps the SSP forever.
///
/// Only **top-level** keys are stripped — matching the replica's `SELECT *`,
/// which drops only undefined top-level fields while preserving nested object
/// content inside a defined field. Non-object values pass through unchanged.
fn strip_reserved_keys(value: Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .filter(|(k, v)| !k.starts_with("_00_") && !v.is_null())
                .map(|(k, v)| {
                    // The row's own `id` field carries the same
                    // escaping ambiguity as the pair key (see
                    // `normalize_record_id`) — normalize it too.
                    if k == "id" {
                        if let Value::String(s) = &v {
                            return (k, Value::String(normalize_record_id(s)));
                        }
                    }
                    (k, v)
                })
                .collect(),
        ),
        other => other,
    }
}

/// Recursively-keyed canonical JSON serialization. Objects are emitted with
/// keys in lexicographic order; arrays preserve order; primitives are
/// formatted by `serde_json` as usual.
pub fn canonical_json(value: &Value) -> Vec<u8> {
    let mut buf = Vec::with_capacity(64);
    write_canonical(value, &mut buf);
    buf
}

fn write_canonical(value: &Value, out: &mut Vec<u8>) {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push(b'{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                // serde_json escapes the key the same way it would inside a
                // serialized object — reuse it via a temporary Value to keep
                // the encoding identical to non-canonical output for keys.
                let escaped = serde_json::to_vec(&Value::String((*k).clone()))
                    .expect("string serialization is infallible");
                out.extend_from_slice(&escaped);
                out.push(b':');
                write_canonical(&map[*k], out);
            }
            out.push(b'}');
        }
        Value::Array(arr) => {
            out.push(b'[');
            for (i, v) in arr.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write_canonical(v, out);
            }
            out.push(b']');
        }
        other => {
            // For primitives, serde_json's default output is already
            // canonical (numbers don't get reformatted, strings get the
            // same escaping rules).
            let bytes = serde_json::to_vec(other)
                .expect("primitive serialization is infallible");
            out.extend_from_slice(&bytes);
        }
    }
}

/// Compare two per-table hash maps and return the tables that disagree.
/// A table missing on one side counts as a mismatch (paired with
/// `empty_table_hash()` on the missing side).
pub fn diff_table_hashes(
    a: &BTreeMap<String, String>,
    b: &BTreeMap<String, String>,
) -> Vec<TableHashMismatch> {
    let empty = empty_table_hash();
    let all: std::collections::BTreeSet<&String> = a.keys().chain(b.keys()).collect();
    all.into_iter()
        .filter_map(|table| {
            let av = a.get(table).cloned().unwrap_or_else(|| empty.clone());
            let bv = b.get(table).cloned().unwrap_or_else(|| empty.clone());
            if av == bv {
                None
            } else {
                Some(TableHashMismatch {
                    table: table.clone(),
                    a: av,
                    b: bv,
                })
            }
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableHashMismatch {
    pub table: String,
    pub a: String,
    pub b: String,
}

// ---------------------------------------------------------------------------
// XOR set-hash (catch-up verification)
//
// `hash_table` above sorts all records and digests them together — correct, but
// it must rescan the whole table to recompute. The catch-up path instead needs
// a hash that can be maintained INCREMENTALLY as records are ingested one event
// at a time, on both the SSP circuit and the scheduler's projection, so the two
// can be compared at the exact sequence cut the SSP caught up to.
//
// The XOR of per-record digests is exactly that: commutative+associative (so
// order-independent, no sort) and self-inverse (so a record can be removed by
// XOR-ing its digest again). On an update, `xor_out(old); xor_in(new)`. Records
// feed through the SAME `strip_reserved_keys` + `canonical_json` path as
// `hash_table`, so a record hashes identically here and there — `_00_*`
// (including `_00_rv`) never affects the digest. Output is prefixed `x3:` to
// keep it visibly distinct from the `b3:` sorted hash in diagnostics.
// ---------------------------------------------------------------------------

/// Reserved-prefix-stripped per-record digest (raw blake3-256 bytes) — the unit
/// of the XOR set-hash. Same canonicalization as [`hash_table`], so equal record
/// content yields an identical digest on the SSP circuit and the scheduler.
pub fn record_digest(raw_id: &str, value: &Value) -> [u8; 32] {
    let stripped = strip_reserved_keys(value.clone());
    let mut hasher = blake3::Hasher::new();
    hasher.update(raw_id.as_bytes());
    hasher.update(b"\0");
    hasher.update(&canonical_json(&stripped));
    *hasher.finalize().as_bytes()
}

/// XOR a record's digest into an accumulator (add the record to the set).
pub fn xor_in(acc: &mut [u8; 32], raw_id: &str, value: &Value) {
    let d = record_digest(raw_id, value);
    for (a, b) in acc.iter_mut().zip(d.iter()) {
        *a ^= *b;
    }
}

/// Remove a record from the accumulator. XOR is self-inverse, so this is the
/// same operation as [`xor_in`]; named separately for call-site clarity.
pub fn xor_out(acc: &mut [u8; 32], raw_id: &str, value: &Value) {
    xor_in(acc, raw_id, value);
}

/// The accumulator for an empty table (all zeros).
pub fn xor_empty() -> [u8; 32] {
    [0u8; 32]
}

/// Format an XOR accumulator as `x3:<hex>`.
pub fn xor_acc_to_hex(acc: &[u8; 32]) -> String {
    let hex: String = acc.iter().map(|b| format!("{:02x}", b)).collect();
    format!("x3:{}", hex)
}

/// On-demand XOR set-hash of a table's records — equivalent to folding
/// [`xor_in`] over every record. Used to seed an accumulator from existing rows.
pub fn xor_table_hash<I>(records: I) -> String
where
    I: IntoIterator<Item = (String, Value)>,
{
    let mut acc = xor_empty();
    for (id, value) in records {
        xor_in(&mut acc, &id, &value);
    }
    xor_acc_to_hex(&acc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn order_independent() {
        let a = vec![
            ("u1".to_string(), json!({"name": "alice", "age": 30})),
            ("u2".to_string(), json!({"name": "bob", "age": 25})),
            ("u3".to_string(), json!({"name": "carol", "age": 40})),
        ];
        let b = vec![
            ("u3".to_string(), json!({"age": 40, "name": "carol"})),
            ("u1".to_string(), json!({"age": 30, "name": "alice"})),
            ("u2".to_string(), json!({"age": 25, "name": "bob"})),
        ];
        assert_eq!(hash_table(a), hash_table(b));
    }

    #[test]
    fn detects_value_change() {
        let a = vec![("u1".to_string(), json!({"v": 1}))];
        let b = vec![("u1".to_string(), json!({"v": 2}))];
        assert_ne!(hash_table(a), hash_table(b));
    }

    #[test]
    fn detects_missing_record() {
        let a = vec![
            ("u1".to_string(), json!({"v": 1})),
            ("u2".to_string(), json!({"v": 2})),
        ];
        let b = vec![("u1".to_string(), json!({"v": 1}))];
        assert_ne!(hash_table(a), hash_table(b));
    }

    #[test]
    fn detects_id_collision_split() {
        // "u1" record with all fields vs two records — id boundary must be
        // honored, otherwise concatenating ids+values could collide.
        let a = vec![("u1".to_string(), json!({"x": 1, "y": 2}))];
        let b = vec![
            ("u1".to_string(), json!({"x": 1})),
            ("u1y".to_string(), json!({"y": 2})),
        ];
        assert_ne!(hash_table(a), hash_table(b));
    }

    #[test]
    fn empty_is_stable() {
        assert_eq!(empty_table_hash(), empty_table_hash());
        assert_eq!(empty_table_hash(), hash_table(std::iter::empty()));
    }

    #[test]
    fn nested_object_canonicalized() {
        let a = vec![("r1".to_string(), json!({"a": {"x": 1, "y": 2}, "b": [1, 2]}))];
        let b = vec![("r1".to_string(), json!({"b": [1, 2], "a": {"y": 2, "x": 1}}))];
        assert_eq!(hash_table(a), hash_table(b));
    }

    #[test]
    fn ignores_reserved_metadata_keys() {
        // The SSP circuit keeps `_00_rv` (injected by the ingest event payload);
        // the SCHEMAFULL replica row never has it. The two MUST hash identically
        // — otherwise every replayed table (e.g. `comment`) trips the integrity
        // check and the SSP re-bootstraps forever.
        let circuit = vec![(
            "CM_x".to_string(),
            json!({"id": "comment:CM_x", "content": "hi", "game": "game:g1", "_00_rv": 7}),
        )];
        let replica = vec![(
            "CM_x".to_string(),
            json!({"id": "comment:CM_x", "content": "hi", "game": "game:g1"}),
        )];
        assert_eq!(hash_table(circuit), hash_table(replica));
    }

    #[test]
    fn reserved_strip_is_top_level_only() {
        // SCHEMAFULL drops only undefined *top-level* fields; nested content
        // inside a defined object field is preserved on both sides, so a change
        // there must still change the hash (don't over-strip).
        let a = vec![("r1".to_string(), json!({"meta": {"_00_rv": 1}}))];
        let b = vec![("r1".to_string(), json!({"meta": {"_00_rv": 2}}))];
        assert_ne!(hash_table(a), hash_table(b));
    }

    #[test]
    fn ignores_null_optional_fields() {
        // The SCHEMAFULL replica `SELECT *` materializes an unset `option<...>`
        // field as JSON null; the SSP circuit stores the event payload, which
        // omits it. The two MUST hash identically — otherwise a row with an
        // unset optional (e.g. `broadcast.target_renderer`) trips the catch-up
        // check and re-bootstraps the SSP forever.
        let replica = vec![(
            "B_x".to_string(),
            json!({"id": "broadcast:B_x", "title": "G", "target_renderer": null, "camera_key": null}),
        )];
        let circuit = vec![(
            "B_x".to_string(),
            json!({"id": "broadcast:B_x", "title": "G"}),
        )];
        assert_eq!(hash_table(replica), hash_table(circuit));
    }

    #[test]
    fn null_strip_is_top_level_only() {
        // A null nested *inside* a defined field is real content on both sides;
        // changing it must still change the hash (don't over-strip).
        let a = vec![("r1".to_string(), json!({"meta": {"x": null}}))];
        let b = vec![("r1".to_string(), json!({"meta": {"x": 1}}))];
        assert_ne!(hash_table(a), hash_table(b));
    }

    #[test]
    fn diff_finds_differing_tables() {
        let mut a = BTreeMap::new();
        a.insert("t1".to_string(), "b3:aaaa".to_string());
        a.insert("t2".to_string(), "b3:bbbb".to_string());

        let mut b = BTreeMap::new();
        b.insert("t1".to_string(), "b3:aaaa".to_string());
        b.insert("t2".to_string(), "b3:cccc".to_string());
        b.insert("t3".to_string(), "b3:dddd".to_string());

        let diffs = diff_table_hashes(&a, &b);
        let names: Vec<&str> = diffs.iter().map(|d| d.table.as_str()).collect();
        assert_eq!(names, vec!["t2", "t3"]);
    }

    // --- XOR set-hash ---

    #[test]
    fn xor_is_order_independent() {
        let a = vec![
            ("u1".to_string(), json!({"v": 1})),
            ("u2".to_string(), json!({"v": 2})),
            ("u3".to_string(), json!({"v": 3})),
        ];
        let b = vec![
            ("u3".to_string(), json!({"v": 3})),
            ("u1".to_string(), json!({"v": 1})),
            ("u2".to_string(), json!({"v": 2})),
        ];
        assert_eq!(xor_table_hash(a), xor_table_hash(b));
    }

    #[test]
    fn xor_create_update_delete_round_trips_to_empty() {
        // Lifecycle of one record applied incrementally must return to the
        // empty-table accumulator: create v1, update v1->v2, delete v2.
        let mut acc = xor_empty();
        xor_in(&mut acc, "r1", &json!({"v": 1})); // create
        xor_out(&mut acc, "r1", &json!({"v": 1})); // update: remove old
        xor_in(&mut acc, "r1", &json!({"v": 2})); //         add new
        xor_out(&mut acc, "r1", &json!({"v": 2})); // delete
        assert_eq!(acc, xor_empty());
    }

    #[test]
    fn xor_incremental_matches_on_demand() {
        // Building the accumulator event-by-event (with an intermediate update)
        // must equal hashing the final row set in one pass.
        let mut acc = xor_empty();
        xor_in(&mut acc, "a", &json!({"v": 1}));
        xor_in(&mut acc, "b", &json!({"v": 2}));
        xor_out(&mut acc, "a", &json!({"v": 1})); // a updated 1 -> 9
        xor_in(&mut acc, "a", &json!({"v": 9}));
        let incremental = xor_acc_to_hex(&acc);

        let final_set = vec![
            ("a".to_string(), json!({"v": 9})),
            ("b".to_string(), json!({"v": 2})),
        ];
        assert_eq!(incremental, xor_table_hash(final_set));
    }

    #[test]
    fn xor_ignores_reserved_metadata_keys() {
        // Same parity guarantee as the sorted hash: the SSP circuit keeps
        // `_00_rv`; the SCHEMAFULL replica row never has it. Digests must match.
        let circuit = vec![(
            "CM_x".to_string(),
            json!({"id": "comment:CM_x", "content": "hi", "_00_rv": 7}),
        )];
        let replica = vec![(
            "CM_x".to_string(),
            json!({"id": "comment:CM_x", "content": "hi"}),
        )];
        assert_eq!(xor_table_hash(circuit), xor_table_hash(replica));
    }

    #[test]
    fn xor_ignores_null_optional_fields() {
        // Same null==absent parity as the sorted hash, on the incremental path.
        let replica = vec![(
            "B_x".to_string(),
            json!({"id": "broadcast:B_x", "title": "G", "target_renderer": null}),
        )];
        let circuit = vec![(
            "B_x".to_string(),
            json!({"id": "broadcast:B_x", "title": "G"}),
        )];
        assert_eq!(xor_table_hash(replica), xor_table_hash(circuit));
    }

    #[test]
    fn xor_empty_is_stable_and_detects_change() {
        assert_eq!(xor_table_hash(std::iter::empty()), xor_acc_to_hex(&xor_empty()));
        let a = vec![("u1".to_string(), json!({"v": 1}))];
        let b = vec![("u1".to_string(), json!({"v": 2}))];
        assert_ne!(xor_table_hash(a), xor_table_hash(b));
    }
}
