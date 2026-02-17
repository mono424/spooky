use serde::ser::{Serialize, SerializeMap, SerializeSeq, Serializer};
use smol_str::SmolStr;
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::convert::TryFrom;
use std::hash::{Hash, Hasher};

pub type FastMap<K, V> = BTreeMap<K, V>;

// ─── SpookyNumber ───────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
pub enum SpookyNumber {
    I64(i64),
    U64(u64),
    F64(f64),
}

impl std::fmt::Debug for SpookyNumber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SpookyNumber::I64(i) => write!(f, "I64({})", i),
            SpookyNumber::U64(u) => write!(f, "U64({})", u),
            SpookyNumber::F64(v) => write!(f, "F64({})", v),
        }
    }
}

/// Canonical total ordering for f64:
///   NaN < -Inf < ... < -0 == +0 < ... < +Inf
/// This is required for deterministic ZSet operations.
fn canonical_f64_cmp(a: f64, b: f64) -> Ordering {
    a.total_cmp(&b)
}

fn canonical_f64_hash<H: Hasher>(f: f64, state: &mut H) {
    // Normalize -0.0 to +0.0 for hashing consistency
    if f == 0.0 {
        0.0_f64.to_bits().hash(state);
    } else {
        f.to_bits().hash(state);
    }
}

impl PartialEq for SpookyNumber {
    fn eq(&self, other: &Self) -> bool {
        self.cmp_canonical(other) == Ordering::Equal
    }
}

impl Eq for SpookyNumber {}

impl PartialOrd for SpookyNumber {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp_canonical(other))
    }
}

impl Ord for SpookyNumber {
    #[inline]
    fn cmp(&self, other: &Self) -> Ordering {
        self.cmp_canonical(other)
    }
}

impl Hash for SpookyNumber {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Promote everything to f64 for cross-variant consistency:
        // I64(1) and F64(1.0) should hash the same.
        let f = self.as_f64();
        canonical_f64_hash(f, state);
    }
}

impl SpookyNumber {
    /// Total ordering across all numeric variants.
    /// Compares via f64 promotion with canonical NaN/zero handling.
    #[inline]
    fn cmp_canonical(&self, other: &Self) -> Ordering {
        // Fast path: same variant, integer types
        match (self, other) {
            (SpookyNumber::I64(a), SpookyNumber::I64(b)) => a.cmp(b),
            (SpookyNumber::U64(a), SpookyNumber::U64(b)) => a.cmp(b),
            _ => canonical_f64_cmp(self.as_f64(), other.as_f64()),
        }
    }

    #[inline]
    pub fn as_f64(self) -> f64 {
        match self {
            SpookyNumber::I64(i) => i as f64,
            SpookyNumber::U64(u) => u as f64,
            SpookyNumber::F64(f) => f,
        }
    }

    #[inline]
    pub fn as_i64(self) -> Option<i64> {
        match self {
            SpookyNumber::I64(i) => Some(i),
            SpookyNumber::U64(u) => i64::try_from(u).ok(),
            SpookyNumber::F64(f) => {
                if f.fract() == 0.0 && f >= i64::MIN as f64 && f <= i64::MAX as f64 {
                    Some(f as i64)
                } else {
                    None
                }
            }
        }
    }

    #[inline]
    pub fn as_u64(self) -> Option<u64> {
        match self {
            SpookyNumber::U64(u) => Some(u),
            SpookyNumber::I64(i) => u64::try_from(i).ok(),
            SpookyNumber::F64(f) => {
                if f.fract() == 0.0 && f >= 0.0 && f <= u64::MAX as f64 {
                    Some(f as u64)
                } else {
                    None
                }
            }
        }
    }
}

// ─── SpookyValue ────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum SpookyValue {
    Null,
    Bool(bool),
    Number(SpookyNumber),
    Str(SmolStr),
    Array(Vec<SpookyValue>),
    Object(FastMap<SmolStr, SpookyValue>),
}

impl Default for SpookyValue {
    #[inline]
    fn default() -> Self {
        SpookyValue::Null
    }
}

// ─── Eq / Ord / Hash (required for ZSet keys) ──────────────────────────────

impl PartialEq for SpookyValue {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for SpookyValue {}

impl PartialOrd for SpookyValue {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SpookyValue {
    fn cmp(&self, other: &Self) -> Ordering {
        // Discriminant ordering: Null < Bool < Number < Str < Array < Object
        let disc = |v: &SpookyValue| -> u8 {
            match v {
                SpookyValue::Null => 0,
                SpookyValue::Bool(_) => 1,
                SpookyValue::Number(_) => 2,
                SpookyValue::Str(_) => 3,
                SpookyValue::Array(_) => 4,
                SpookyValue::Object(_) => 5,
            }
        };

        let da = disc(self);
        let db = disc(other);
        if da != db {
            return da.cmp(&db);
        }

        match (self, other) {
            (SpookyValue::Null, SpookyValue::Null) => Ordering::Equal,
            (SpookyValue::Bool(a), SpookyValue::Bool(b)) => a.cmp(b),
            (SpookyValue::Number(a), SpookyValue::Number(b)) => a.cmp(b),
            (SpookyValue::Str(a), SpookyValue::Str(b)) => a.cmp(b),
            (SpookyValue::Array(a), SpookyValue::Array(b)) => a.cmp(b),
            (SpookyValue::Object(a), SpookyValue::Object(b)) => a.cmp(b),
            _ => unreachable!(),
        }
    }
}

impl Hash for SpookyValue {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            SpookyValue::Null => {}
            SpookyValue::Bool(b) => b.hash(state),
            SpookyValue::Number(n) => n.hash(state),
            SpookyValue::Str(s) => s.hash(state),
            SpookyValue::Array(arr) => {
                arr.len().hash(state);
                for v in arr {
                    v.hash(state);
                }
            }
            SpookyValue::Object(map) => {
                map.len().hash(state);
                for (k, v) in map {
                    k.hash(state);
                    v.hash(state);
                }
            }
        }
    }
}

// ─── Accessors ──────────────────────────────────────────────────────────────

impl SpookyValue {
    #[inline]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            SpookyValue::Str(s) => Some(s.as_str()),
            _ => None,
        }
    }

    #[inline]
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            SpookyValue::Number(n) => Some(n.as_f64()),
            _ => None,
        }
    }

    #[inline]
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            SpookyValue::Number(n) => n.as_i64(),
            _ => None,
        }
    }

    #[inline]
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            SpookyValue::Number(n) => n.as_u64(),
            _ => None,
        }
    }

    #[inline]
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            SpookyValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    #[inline]
    pub fn as_object(&self) -> Option<&FastMap<SmolStr, SpookyValue>> {
        match self {
            SpookyValue::Object(map) => Some(map),
            _ => None,
        }
    }

    /// Mutable object access — avoids clone-modify-replace patterns.
    #[inline]
    pub fn as_object_mut(&mut self) -> Option<&mut FastMap<SmolStr, SpookyValue>> {
        match self {
            SpookyValue::Object(map) => Some(map),
            _ => None,
        }
    }

    #[inline]
    pub fn as_array(&self) -> Option<&Vec<SpookyValue>> {
        match self {
            SpookyValue::Array(arr) => Some(arr),
            _ => None,
        }
    }

    /// Mutable array access — avoids clone-modify-replace patterns.
    #[inline]
    pub fn as_array_mut(&mut self) -> Option<&mut Vec<SpookyValue>> {
        match self {
            SpookyValue::Array(arr) => Some(arr),
            _ => None,
        }
    }

    /// Field access by key. Uses BTreeMap's native lookup — no SmolStr allocation
    /// thanks to SmolStr implementing Borrow<str>.
    #[inline]
    pub fn get(&self, key: &str) -> Option<&SpookyValue> {
        // SmolStr implements Borrow<str>, and BTreeMap::get accepts Q where K: Borrow<Q>.
        // However, BTreeMap<SmolStr, V>::get(&str) requires Ord consistency.
        // SmolStr's Ord delegates to str's Ord, so this is safe.
        self.as_object()?.get(key)
    }

    /// Mutable field access by key.
    #[inline]
    pub fn get_mut(&mut self, key: &str) -> Option<&mut SpookyValue> {
        self.as_object_mut()?.get_mut(key)
    }

    #[inline]
    pub fn is_null(&self) -> bool {
        matches!(self, SpookyValue::Null)
    }

    #[inline]
    pub fn is_object(&self) -> bool {
        matches!(self, SpookyValue::Object(_))
    }

    #[inline]
    pub fn is_array(&self) -> bool {
        matches!(self, SpookyValue::Array(_))
    }

    #[inline]
    pub fn is_string(&self) -> bool {
        matches!(self, SpookyValue::Str(_))
    }

    #[inline]
    pub fn is_number(&self) -> bool {
        matches!(self, SpookyValue::Number(_))
    }
}

// ─── Serialize ──────────────────────────────────────────────────────────────

impl Serialize for SpookyValue {
    #[inline]
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            SpookyValue::Null => serializer.serialize_none(),
            SpookyValue::Bool(b) => serializer.serialize_bool(*b),
            SpookyValue::Number(n) => match n {
                SpookyNumber::I64(i) => serializer.serialize_i64(*i),
                SpookyNumber::U64(u) => serializer.serialize_u64(*u),
                SpookyNumber::F64(f) => serializer.serialize_f64(*f),
            },
            SpookyValue::Str(s) => serializer.serialize_str(s.as_str()),
            SpookyValue::Array(arr) => {
                let mut seq = serializer.serialize_seq(Some(arr.len()))?;
                for v in arr {
                    seq.serialize_element(v)?;
                }
                seq.end()
            }
            SpookyValue::Object(map) => {
                let mut m = serializer.serialize_map(Some(map.len()))?;
                for (k, v) in map {
                    m.serialize_entry(k.as_str(), v)?;
                }
                m.end()
            }
        }
    }
}

// ─── From impls ─────────────────────────────────────────────────────────────

impl From<f64> for SpookyValue {
    #[inline]
    fn from(n: f64) -> Self {
        SpookyValue::Number(SpookyNumber::F64(n))
    }
}

impl From<i64> for SpookyValue {
    #[inline]
    fn from(n: i64) -> Self {
        SpookyValue::Number(SpookyNumber::I64(n))
    }
}

impl From<i32> for SpookyValue {
    #[inline]
    fn from(n: i32) -> Self {
        SpookyValue::Number(SpookyNumber::I64(n as i64))
    }
}

impl From<u64> for SpookyValue {
    #[inline]
    fn from(n: u64) -> Self {
        SpookyValue::Number(SpookyNumber::U64(n))
    }
}

impl From<u32> for SpookyValue {
    #[inline]
    fn from(n: u32) -> Self {
        SpookyValue::Number(SpookyNumber::U64(n as u64))
    }
}

impl From<bool> for SpookyValue {
    #[inline]
    fn from(b: bool) -> Self {
        SpookyValue::Bool(b)
    }
}

impl From<&str> for SpookyValue {
    #[inline]
    fn from(s: &str) -> Self {
        SpookyValue::Str(SmolStr::from(s))
    }
}

impl From<String> for SpookyValue {
    #[inline]
    fn from(s: String) -> Self {
        SpookyValue::Str(SmolStr::from(s))
    }
}

impl From<SmolStr> for SpookyValue {
    #[inline]
    fn from(s: SmolStr) -> Self {
        SpookyValue::Str(s)
    }
}

// ─── From<ciborium::Value> ─────────────────────────────────────────────────

impl From<cbor4ii::core::Value> for SpookyValue {
    fn from(v: cbor4ii::core::Value) -> Self {
        match v {
            cbor4ii::core::Value::Null => SpookyValue::Null,
            cbor4ii::core::Value::Bool(b) => SpookyValue::Bool(b),
            cbor4ii::core::Value::Integer(i) => {
                if let Ok(val) = i64::try_from(i) {
                    SpookyValue::Number(SpookyNumber::I64(val))
                } else if let Ok(val) = u64::try_from(i) {
                    SpookyValue::Number(SpookyNumber::U64(val))
                } else {
                    SpookyValue::Number(SpookyNumber::F64(i as f64))
                }
            }
            cbor4ii::core::Value::Float(f) => SpookyValue::Number(SpookyNumber::F64(f)),
            cbor4ii::core::Value::Text(s) => SpookyValue::Str(SmolStr::from(s)),
            cbor4ii::core::Value::Array(arr) => {
                SpookyValue::Array(arr.into_iter().map(SpookyValue::from).collect())
            }
            cbor4ii::core::Value::Map(map) => SpookyValue::Object(
                map.into_iter()
                    .map(|(k, v)| {
                        let key = match k {
                            cbor4ii::core::Value::Text(s) => SmolStr::from(s),
                            cbor4ii::core::Value::Integer(i) => SmolStr::from(i.to_string()),
                            other => SmolStr::from(format!("{:?}", other)),
                        };
                        (key, SpookyValue::from(v))
                    })
                    .collect(),
            ),
            _ => SpookyValue::Null,
        }
    }
}

// ─── Into<cbor4ii::core::Value> ──────────────────────────────────────────────────

impl From<SpookyValue> for cbor4ii::core::Value {
    fn from(val: SpookyValue) -> Self {
        match val {
            SpookyValue::Null => cbor4ii::core::Value::Null,
            SpookyValue::Bool(b) => cbor4ii::core::Value::Bool(b),
            SpookyValue::Number(n) => match n {
                SpookyNumber::I64(i) => cbor4ii::core::Value::Integer(i as i128),
                SpookyNumber::U64(u) => cbor4ii::core::Value::Integer(u as i128),
                SpookyNumber::F64(f) => cbor4ii::core::Value::Float(f),
            },
            SpookyValue::Str(s) => cbor4ii::core::Value::Text(s.to_string()),
            SpookyValue::Array(arr) => {
                cbor4ii::core::Value::Array(arr.into_iter().map(|v| v.into()).collect())
            }
            SpookyValue::Object(obj) => cbor4ii::core::Value::Map(
                obj.into_iter()
                    .map(|(k, v)| (cbor4ii::core::Value::Text(k.to_string()), v.into()))
                    .collect(),
            ),
        }
    }
}

// ─── From/Into serde_json::Value ────────────────────────────────────────────

impl From<serde_json::Value> for SpookyValue {
    fn from(v: serde_json::Value) -> Self {
        match v {
            serde_json::Value::Null => SpookyValue::Null,
            serde_json::Value::Bool(b) => SpookyValue::Bool(b),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    SpookyValue::Number(SpookyNumber::I64(i))
                } else if let Some(u) = n.as_u64() {
                    SpookyValue::Number(SpookyNumber::U64(u))
                } else {
                    SpookyValue::Number(SpookyNumber::F64(n.as_f64().unwrap_or(0.0)))
                }
            }
            serde_json::Value::String(s) => SpookyValue::Str(SmolStr::from(s)),
            serde_json::Value::Array(arr) => {
                SpookyValue::Array(arr.into_iter().map(SpookyValue::from).collect())
            }
            serde_json::Value::Object(obj) => SpookyValue::Object(
                obj.into_iter()
                    .map(|(k, v)| (SmolStr::from(k), SpookyValue::from(v)))
                    .collect(),
            ),
        }
    }
}

impl From<SpookyValue> for serde_json::Value {
    fn from(val: SpookyValue) -> Self {
        match val {
            SpookyValue::Null => serde_json::Value::Null,
            SpookyValue::Bool(b) => serde_json::Value::Bool(b),
            SpookyValue::Number(n) => match n {
                SpookyNumber::I64(i) => serde_json::json!(i),
                SpookyNumber::U64(u) => serde_json::json!(u),
                SpookyNumber::F64(f) => serde_json::json!(f),
            },
            SpookyValue::Str(s) => serde_json::Value::String(s.to_string()),
            SpookyValue::Array(arr) => {
                serde_json::Value::Array(arr.into_iter().map(|v| v.into()).collect())
            }
            SpookyValue::Object(obj) => serde_json::Value::Object(
                obj.into_iter()
                    .map(|(k, v)| (k.to_string(), v.into()))
                    .collect(),
            ),
        }
    }
}

#[macro_export]
macro_rules! spooky_obj {
    ({ $($key:expr => $val:tt),* $(,)? }) => {{
        let mut map = FastMap::default();
        $(
            map.insert(
                SmolStr::new($key),
                SpookyValue::from(spooky_obj!(@value $val))
            );
        )*
        SpookyValue::Object(map)
    }};

    (@value { $($inner:tt)* }) => {
        spooky_obj!({ $($inner)* })
    };

    (@value $val:expr) => {
        $val
    };
}
