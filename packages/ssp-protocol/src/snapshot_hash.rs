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
    let mut pairs: Vec<(String, Value)> = records.into_iter().collect();
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

/// Hash for an empty table — useful when comparing tables that exist on one
/// side but not the other.
pub fn empty_table_hash() -> String {
    hash_table(std::iter::empty())
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
}
