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
