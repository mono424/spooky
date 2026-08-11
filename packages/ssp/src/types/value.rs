use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

/// Dynamic record value type.
///
/// Represents the data content of a record in a collection.
/// Uses standard `String` and `HashMap` instead of SmolStr/FxHasher.
///
/// Numbers are split into `Int` and `Float` so a JSON `5` round-trips back to
/// `5` (not `5.0`). Hashing the same row through this type and through
/// `serde_json::Value` directly must produce identical bytes — see
/// `ssp_protocol::snapshot_hash::canonical_json` and the SSP/scheduler
/// integrity check.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Sp00kyValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Array(Vec<Sp00kyValue>),
    Object(HashMap<String, Sp00kyValue>),
}

impl Default for Sp00kyValue {
    fn default() -> Self {
        Sp00kyValue::Null
    }
}

impl Sp00kyValue {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Sp00kyValue::Str(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Sp00kyValue::Int(i) => Some(*i as f64),
            Sp00kyValue::Float(f) => Some(*f),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Sp00kyValue::Int(i) => Some(*i),
            Sp00kyValue::Float(f) if f.is_finite() && f.fract() == 0.0 => Some(*f as i64),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Sp00kyValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&HashMap<String, Sp00kyValue>> {
        match self {
            Sp00kyValue::Object(map) => Some(map),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&Vec<Sp00kyValue>> {
        match self {
            Sp00kyValue::Array(arr) => Some(arr),
            _ => None,
        }
    }

    pub fn get(&self, key: &str) -> Option<&Sp00kyValue> {
        self.as_object()?.get(key)
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Sp00kyValue::Null)
    }

    /// Whether this value would become JSON `null` on the way to a
    /// `serde_json::Value`.
    ///
    /// `Null` obviously does. So does a **non-finite `Float`**:
    /// `Number::from_f64` rejects NaN and the infinities, and
    /// `From<f64> for Value` falls back to `Value::Null`. That distinction is
    /// load-bearing — the hash pipeline drops null-valued top-level keys, so a
    /// field holding `NaN` is stripped from the digest today, and a writer
    /// that treated it as a number would change every such row's hash.
    fn is_json_null(&self) -> bool {
        match self {
            Sp00kyValue::Null => true,
            Sp00kyValue::Float(f) => !f.is_finite(),
            _ => false,
        }
    }

    /// Append this value's canonical JSON to `out` — objects with keys in
    /// lexicographic order, arrays in order, primitives formatted by
    /// `serde_json`.
    ///
    /// Byte-identical to `ssp_protocol::snapshot_hash::canonical_json` applied
    /// to `serde_json::Value::from(self.clone())`, without building that
    /// intermediate tree.
    pub fn write_canonical(&self, out: &mut Vec<u8>) {
        use ssp_protocol::snapshot_hash as sh;
        match self {
            Sp00kyValue::Null => out.extend_from_slice(b"null"),
            Sp00kyValue::Bool(true) => out.extend_from_slice(b"true"),
            Sp00kyValue::Bool(false) => out.extend_from_slice(b"false"),
            Sp00kyValue::Int(i) => sh::write_json_i64(*i, out),
            Sp00kyValue::Float(f) => sh::write_json_f64(*f, out),
            Sp00kyValue::Str(s) => sh::write_json_string(s, out),
            Sp00kyValue::Array(items) => {
                out.push(b'[');
                for (i, v) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(b',');
                    }
                    v.write_canonical(out);
                }
                out.push(b']');
            }
            Sp00kyValue::Object(map) => {
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort();
                out.push(b'{');
                for (i, k) in keys.iter().enumerate() {
                    if i > 0 {
                        out.push(b',');
                    }
                    sh::write_json_string(k, out);
                    out.push(b':');
                    map[*k].write_canonical(out);
                }
                out.push(b'}');
            }
        }
    }

    /// Append the canonical JSON of this value **as a record**: the same
    /// output as [`write_canonical`], except that at the top level reserved
    /// `_00_*` keys and null-valued keys are dropped and an `id` string is
    /// normalized.
    ///
    /// Mirrors `strip_reserved_keys` + `canonical_json`. The stripping is
    /// top-level only — nested objects keep their `_00_*` and null entries,
    /// matching the replica's `SELECT *`, which drops only undefined
    /// *top-level* fields.
    pub fn write_canonical_record(&self, out: &mut Vec<u8>) {
        use ssp_protocol::snapshot_hash as sh;
        let Sp00kyValue::Object(map) = self else {
            // Non-objects pass through unstripped, as in `strip_reserved_keys`.
            return self.write_canonical(out);
        };
        let mut keys: Vec<&String> = map
            .iter()
            .filter(|(k, v)| !sh::is_reserved_key(k) && !v.is_json_null())
            .map(|(k, _)| k)
            .collect();
        keys.sort();
        out.push(b'{');
        for (i, k) in keys.iter().enumerate() {
            if i > 0 {
                out.push(b',');
            }
            sh::write_json_string(k, out);
            out.push(b':');
            let value = &map[*k];
            // The row's own `id` carries the same escaping ambiguity as the
            // record key, so it gets the same normalization.
            if k.as_str() == "id" {
                if let Sp00kyValue::Str(s) = value {
                    sh::write_json_string(&sh::normalize_record_id(s), out);
                    continue;
                }
            }
            value.write_canonical(out);
        }
        out.push(b'}');
    }

    /// This row's digest — the unit of the catch-up XOR set-hash.
    ///
    /// Equal to `ssp_protocol::snapshot_hash::record_digest(raw_id,
    /// &Value::from(self.clone()))`, but computed straight off the stored
    /// value. `scratch` is a caller-owned buffer reused across rows so a bulk
    /// re-seed does not allocate per row.
    pub fn record_digest_into(&self, raw_id: &str, scratch: &mut Vec<u8>) -> [u8; 32] {
        scratch.clear();
        self.write_canonical_record(scratch);
        ssp_protocol::snapshot_hash::digest_from_canonical(raw_id, scratch)
    }

    /// Approximate heap bytes owned by this value, excluding the enum's own
    /// inline size (the parent's slot already accounts for that).
    ///
    /// The `Object` arm is where the cost lives: a fresh `String` allocation
    /// per field name on every row, with no interning, plus a bucket array
    /// sized for the field count. A 12-field row runs well over a kilobyte for
    /// what is a few hundred bytes of JSON.
    pub fn heap_bytes(&self) -> usize {
        match self {
            Sp00kyValue::Null
            | Sp00kyValue::Bool(_)
            | Sp00kyValue::Int(_)
            | Sp00kyValue::Float(_) => 0,
            Sp00kyValue::Str(s) => s.capacity(),
            Sp00kyValue::Array(items) => {
                crate::size::vec_bytes::<Sp00kyValue>(items.capacity())
                    + items.iter().map(Sp00kyValue::heap_bytes).sum::<usize>()
            }
            Sp00kyValue::Object(map) => {
                crate::size::map_table_bytes::<String, Sp00kyValue>(map.capacity())
                    + map
                        .iter()
                        .map(|(k, v)| k.capacity() + v.heap_bytes())
                        .sum::<usize>()
            }
        }
    }
}

impl From<Value> for Sp00kyValue {
    fn from(v: Value) -> Self {
        match v {
            Value::Null => Sp00kyValue::Null,
            Value::Bool(b) => Sp00kyValue::Bool(b),
            Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Sp00kyValue::Int(i)
                } else {
                    // u64 > i64::MAX or true float — collapse to Float.
                    Sp00kyValue::Float(n.as_f64().unwrap_or(0.0))
                }
            }
            Value::String(s) => Sp00kyValue::Str(s),
            Value::Array(arr) => {
                Sp00kyValue::Array(arr.into_iter().map(Sp00kyValue::from).collect())
            }
            Value::Object(obj) => Sp00kyValue::Object(
                obj.into_iter()
                    .map(|(k, v)| (k, Sp00kyValue::from(v)))
                    .collect(),
            ),
        }
    }
}

impl From<Sp00kyValue> for Value {
    fn from(val: Sp00kyValue) -> Self {
        match val {
            Sp00kyValue::Null => Value::Null,
            Sp00kyValue::Bool(b) => Value::Bool(b),
            Sp00kyValue::Int(i) => json!(i),
            Sp00kyValue::Float(f) => json!(f),
            Sp00kyValue::Str(s) => Value::String(s),
            Sp00kyValue::Array(arr) => {
                Value::Array(arr.into_iter().map(|v| v.into()).collect())
            }
            Sp00kyValue::Object(obj) => {
                Value::Object(obj.into_iter().map(|(k, v)| (k, v.into())).collect())
            }
        }
    }
}
