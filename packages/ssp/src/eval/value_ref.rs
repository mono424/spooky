//! Borrowed view of a value, independent of how that value is stored.
//!
//! Every read the circuit performs on a row goes through this type: predicate
//! evaluation, join keys, sort keys, aggregate grouping. It exists so those
//! readers stop naming `Sp00kyValue` directly, which is what lets the row
//! store change representation underneath them without touching a single
//! operator.
//!
//! Today there is one backend ([`ObjRef::Mem`] / [`SeqRef::Mem`], a real
//! `Sp00kyValue` tree). A flat byte-encoded backend is added alongside it
//! later; the container enums exist now so that addition is additive.
//!
//! Two design points worth stating, because they are load-bearing:
//!
//! **Scalars are unified across backends.** Only containers are per-backend.
//! That keeps [`compare_values`] a flat match over scalar arms instead of a
//! cross product, and makes [`ValueRef::from_value`] a total, lossless
//! conversion — which matters because predicate evaluation resolves paths
//! against in-memory query params, not only against stored rows.
//!
//! **`Missing` replaces the outer `Option`.** The accessors used to return
//! `Option<&Sp00kyValue>`, so a caller juggled two layers of absence: "no such
//! field" and "field holds null". Folding the first into the type collapses
//! that, and the ordering is unchanged — see [`ValueRef::rank`].
//!
//! [`compare_values`]: crate::eval::compare_values

use crate::circuit::row_codec::FieldDict;
use crate::types::Sp00kyValue;
use std::collections::HashMap;

/// A borrowed value. `Copy`, allocation-free, cheap to pass by value.
#[derive(Clone, Copy, Debug)]
pub enum ValueRef<'a> {
    /// No such field. Distinct from [`ValueRef::Null`], which is a field that
    /// exists and holds null.
    Missing,
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(&'a str),
    Arr(SeqRef<'a>),
    Obj(ObjRef<'a>),
}

/// Borrowed object, by backend.
#[derive(Clone, Copy, Debug)]
pub enum ObjRef<'a> {
    Mem(&'a HashMap<String, Sp00kyValue>),
    Flat(FlatRef<'a>),
}

/// Borrowed array, by backend.
#[derive(Clone, Copy, Debug)]
pub enum SeqRef<'a> {
    Mem(&'a [Sp00kyValue]),
    Flat(FlatRef<'a>),
}

/// A flat-encoded value plus the dictionary needed to read its field names.
#[derive(Clone, Copy, Debug)]
pub struct FlatRef<'a> {
    pub bytes: &'a [u8],
    pub dict: &'a FieldDict,
}

impl<'a> FlatRef<'a> {
    /// Interpret encoded bytes, mapping an unknown or truncated tag to
    /// [`ValueRef::Missing`].
    ///
    /// Absence rather than an error is the right answer here: once these bytes
    /// can come from a file, unreadable input has to degrade to a missing
    /// value the same way an absent field does, never to a panic.
    pub fn value(self) -> ValueRef<'a> {
        use crate::circuit::row_codec as codec;
        let Some(&tag) = self.bytes.first() else {
            return ValueRef::Missing;
        };
        match tag {
            codec::TAG_NULL => ValueRef::Null,
            codec::TAG_FALSE => ValueRef::Bool(false),
            codec::TAG_TRUE => ValueRef::Bool(true),
            codec::TAG_INT => match self.bytes.get(1..9).and_then(|b| b.try_into().ok()) {
                Some(b) => ValueRef::Int(i64::from_le_bytes(b)),
                None => ValueRef::Missing,
            },
            codec::TAG_FLOAT => match self.bytes.get(1..9).and_then(|b| b.try_into().ok()) {
                Some(b) => ValueRef::Float(f64::from_bits(u64::from_le_bytes(b))),
                None => ValueRef::Missing,
            },
            codec::TAG_STR => match codec::str_bytes(self.bytes) {
                Some(s) => ValueRef::Str(s),
                None => ValueRef::Missing,
            },
            codec::TAG_ARR => ValueRef::Arr(SeqRef::Flat(self)),
            codec::TAG_OBJ => ValueRef::Obj(ObjRef::Flat(self)),
            _ => ValueRef::Missing,
        }
    }
}

impl<'a> ValueRef<'a> {
    /// Borrow an in-memory value. Total and lossless.
    pub fn from_value(value: &'a Sp00kyValue) -> Self {
        match value {
            Sp00kyValue::Null => ValueRef::Null,
            Sp00kyValue::Bool(b) => ValueRef::Bool(*b),
            Sp00kyValue::Int(i) => ValueRef::Int(*i),
            Sp00kyValue::Float(f) => ValueRef::Float(*f),
            Sp00kyValue::Str(s) => ValueRef::Str(s.as_str()),
            Sp00kyValue::Array(items) => ValueRef::Arr(SeqRef::Mem(items.as_slice())),
            Sp00kyValue::Object(map) => ValueRef::Obj(ObjRef::Mem(map)),
        }
    }

    /// Borrow an optional in-memory value, mapping `None` to [`Missing`].
    ///
    /// [`Missing`]: ValueRef::Missing
    pub fn from_opt(value: Option<&'a Sp00kyValue>) -> Self {
        match value {
            Some(v) => ValueRef::from_value(v),
            None => ValueRef::Missing,
        }
    }

    pub fn is_missing(self) -> bool {
        matches!(self, ValueRef::Missing)
    }

    pub fn is_null(self) -> bool {
        matches!(self, ValueRef::Null)
    }

    pub fn as_str(self) -> Option<&'a str> {
        match self {
            ValueRef::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_f64(self) -> Option<f64> {
        match self {
            ValueRef::Int(i) => Some(i as f64),
            ValueRef::Float(f) => Some(f),
            _ => None,
        }
    }

    pub fn as_i64(self) -> Option<i64> {
        match self {
            ValueRef::Int(i) => Some(i),
            ValueRef::Float(f) if f.is_finite() && f.fract() == 0.0 => Some(f as i64),
            _ => None,
        }
    }

    pub fn as_bool(self) -> Option<bool> {
        match self {
            ValueRef::Bool(b) => Some(b),
            _ => None,
        }
    }

    /// Look up a field. Returns [`Missing`] for a non-object or an absent key,
    /// so lookups chain without unwrapping.
    ///
    /// [`Missing`]: ValueRef::Missing
    pub fn get(self, key: &str) -> ValueRef<'a> {
        match self {
            ValueRef::Obj(ObjRef::Mem(map)) => ValueRef::from_opt(map.get(key)),
            ValueRef::Obj(ObjRef::Flat(flat)) => {
                // A name the dictionary has never seen cannot be on any row of
                // this table, so the lookup short-circuits before touching the
                // entry table at all.
                let Some(id) = flat.dict.id_of(key) else {
                    return ValueRef::Missing;
                };
                match crate::circuit::row_codec::obj_lookup(flat.bytes, id) {
                    Some(bytes) => FlatRef {
                        bytes,
                        dict: flat.dict,
                    }
                    .value(),
                    None => ValueRef::Missing,
                }
            }
            _ => ValueRef::Missing,
        }
    }

    /// Number of entries, for an object or array. Zero otherwise.
    fn container_len(self) -> usize {
        use crate::circuit::row_codec as codec;
        match self {
            ValueRef::Obj(ObjRef::Mem(map)) => map.len(),
            ValueRef::Arr(SeqRef::Mem(items)) => items.len(),
            ValueRef::Obj(ObjRef::Flat(f)) => {
                codec::obj_parts(f.bytes).map_or(0, |p| p.count)
            }
            ValueRef::Arr(SeqRef::Flat(f)) => codec::arr_parts(f.bytes).map_or(0, |(n, _)| n),
            _ => 0,
        }
    }

    /// Whether this is a string holding a SurrealDB record reference
    /// (`"table:id"`).
    ///
    /// Drives the `<field>.id` auto-dereference in path resolution: the store
    /// keeps record references as their string form, but SurrealDB's predicate
    /// semantics dereference `author.id` on a `record<user>` field to the
    /// linked id — and that string *is* the id.
    pub fn is_record_ref(self) -> bool {
        match self {
            ValueRef::Str(s) => s.contains(':'),
            _ => false,
        }
    }

    /// Sort rank across variants, used by comparison.
    ///
    /// Preserves the pre-`ValueRef` ordering exactly. Comparison used to take
    /// `Option<&Sp00kyValue>` and rank `None` below `Some(Null)` below
    /// everything else; `Missing` inherits `None`'s position, so nothing moves.
    pub(crate) fn rank(self) -> u8 {
        match self {
            ValueRef::Missing => 0,
            ValueRef::Null => 1,
            _ => 2,
        }
    }

    /// Materialize an owned copy. Allocates — only for callers that genuinely
    /// need ownership.
    pub fn to_owned_value(self) -> Sp00kyValue {
        use crate::circuit::row_codec as codec;
        match self {
            ValueRef::Missing | ValueRef::Null => Sp00kyValue::Null,
            ValueRef::Bool(b) => Sp00kyValue::Bool(b),
            ValueRef::Int(i) => Sp00kyValue::Int(i),
            ValueRef::Float(f) => Sp00kyValue::Float(f),
            ValueRef::Str(s) => Sp00kyValue::Str(s.to_string()),
            ValueRef::Arr(SeqRef::Mem(items)) => Sp00kyValue::Array(items.to_vec()),
            ValueRef::Obj(ObjRef::Mem(map)) => Sp00kyValue::Object(map.clone()),
            ValueRef::Arr(SeqRef::Flat(f)) | ValueRef::Obj(ObjRef::Flat(f)) => {
                // Undecodable bytes become Null rather than propagating an
                // error, matching how `Missing` materializes.
                codec::decode_value(f.bytes, f.dict).unwrap_or(Sp00kyValue::Null)
            }
        }
    }

    /// Stable string form for use as a grouping key.
    ///
    /// Explicitly *not* `Debug`. Group keys are opaque strings compared for
    /// equality and never persisted, so if their formatting silently changed,
    /// rows would repartition into different groups with no test failing
    /// anywhere. Pinning the format here makes that a visible edit.
    ///
    /// Variants are tagged so values of different types cannot collide: the
    /// string `"1"` and the integer `1` are different groups.
    pub fn group_key_repr(self) -> String {
        match self {
            ValueRef::Missing => "missing".to_string(),
            ValueRef::Null => "null".to_string(),
            ValueRef::Bool(b) => format!("b:{b}"),
            ValueRef::Int(i) => format!("i:{i}"),
            ValueRef::Float(f) => format!("f:{f}"),
            ValueRef::Str(s) => format!("s:{s}"),
            ValueRef::Arr(_) => format!("a:{}", self.container_len()),
            ValueRef::Obj(_) => format!("o:{}", self.container_len()),
        }
    }
}

/// `ValueRef` is passed by value on the hottest path in the circuit, so its
/// size is a real cost. The flat backend needs a slice (16 bytes) plus a
/// dictionary reference (8), which sets the floor at 24 for the payload.
const _: () = assert!(
    std::mem::size_of::<ValueRef<'_>>() <= 32,
    "ValueRef grew past two cache-line-friendly words; check what was added"
);

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn v(j: serde_json::Value) -> Sp00kyValue {
        Sp00kyValue::from(j)
    }

    #[test]
    fn scalars_round_trip() {
        for j in [json!(null), json!(true), json!(5), json!(5.5), json!("s")] {
            let owned = v(j.clone());
            let back = ValueRef::from_value(&owned).to_owned_value();
            assert_eq!(back, owned, "round trip changed {j}");
        }
    }

    #[test]
    fn containers_round_trip() {
        let owned = v(json!({ "a": [1, "two", null], "b": { "c": 3 } }));
        assert_eq!(ValueRef::from_value(&owned).to_owned_value(), owned);
    }

    #[test]
    fn missing_is_distinct_from_null() {
        let owned = v(json!({ "present": null }));
        let r = ValueRef::from_value(&owned);
        assert!(r.get("present").is_null());
        assert!(!r.get("present").is_missing());
        assert!(r.get("absent").is_missing());
        assert!(!r.get("absent").is_null());
    }

    #[test]
    fn get_on_a_non_object_is_missing_not_a_panic() {
        let owned = v(json!(5));
        assert!(ValueRef::from_value(&owned).get("anything").is_missing());
        assert!(ValueRef::Missing.get("anything").is_missing());
    }

    #[test]
    fn rank_orders_missing_below_null_below_scalars() {
        assert!(ValueRef::Missing.rank() < ValueRef::Null.rank());
        assert!(ValueRef::Null.rank() < ValueRef::Int(0).rank());
        assert_eq!(ValueRef::Int(0).rank(), ValueRef::Str("").rank());
    }

    #[test]
    fn record_ref_detection() {
        let owned = v(json!({ "ref": "user:abc", "plain": "abc", "n": 1 }));
        let r = ValueRef::from_value(&owned);
        assert!(r.get("ref").is_record_ref());
        assert!(!r.get("plain").is_record_ref());
        assert!(!r.get("n").is_record_ref());
        assert!(!ValueRef::Missing.is_record_ref());
    }

    #[test]
    fn as_i64_accepts_whole_floats_like_the_owned_type() {
        let owned = v(json!({ "i": 5, "f": 5.0, "frac": 5.5 }));
        let r = ValueRef::from_value(&owned);
        assert_eq!(r.get("i").as_i64(), Some(5));
        assert_eq!(r.get("f").as_i64(), Some(5));
        assert_eq!(r.get("frac").as_i64(), None);
        // Matching Sp00kyValue::as_i64 exactly.
        assert_eq!(r.get("f").as_i64(), owned.get("f").unwrap().as_i64());
    }

    #[test]
    fn group_key_repr_separates_types() {
        let owned = v(json!({ "s": "1", "i": 1, "b": true }));
        let r = ValueRef::from_value(&owned);
        let s = r.get("s").group_key_repr();
        let i = r.get("i").group_key_repr();
        assert_ne!(s, i, "a string and an int must not share a group");
        assert_ne!(i, r.get("b").group_key_repr());
        assert_ne!(
            ValueRef::Missing.group_key_repr(),
            ValueRef::Null.group_key_repr()
        );
    }
}
