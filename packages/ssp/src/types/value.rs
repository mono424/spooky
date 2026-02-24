use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

/// Dynamic record value type.
///
/// Represents the data content of a record in a collection.
/// Uses standard `String` and `HashMap` instead of SmolStr/FxHasher.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SpookyValue {
    Null,
    Bool(bool),
    Number(f64),
    Str(String),
    Array(Vec<SpookyValue>),
    Object(HashMap<String, SpookyValue>),
}

impl Default for SpookyValue {
    fn default() -> Self {
        SpookyValue::Null
    }
}

impl SpookyValue {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            SpookyValue::Str(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            SpookyValue::Number(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            SpookyValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&HashMap<String, SpookyValue>> {
        match self {
            SpookyValue::Object(map) => Some(map),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&Vec<SpookyValue>> {
        match self {
            SpookyValue::Array(arr) => Some(arr),
            _ => None,
        }
    }

    pub fn get(&self, key: &str) -> Option<&SpookyValue> {
        self.as_object()?.get(key)
    }

    pub fn is_null(&self) -> bool {
        matches!(self, SpookyValue::Null)
    }
}

impl From<Value> for SpookyValue {
    fn from(v: Value) -> Self {
        match v {
            Value::Null => SpookyValue::Null,
            Value::Bool(b) => SpookyValue::Bool(b),
            Value::Number(n) => SpookyValue::Number(n.as_f64().unwrap_or(0.0)),
            Value::String(s) => SpookyValue::Str(s),
            Value::Array(arr) => {
                SpookyValue::Array(arr.into_iter().map(SpookyValue::from).collect())
            }
            Value::Object(obj) => SpookyValue::Object(
                obj.into_iter()
                    .map(|(k, v)| (k, SpookyValue::from(v)))
                    .collect(),
            ),
        }
    }
}

impl From<SpookyValue> for Value {
    fn from(val: SpookyValue) -> Self {
        match val {
            SpookyValue::Null => Value::Null,
            SpookyValue::Bool(b) => Value::Bool(b),
            SpookyValue::Number(n) => json!(n),
            SpookyValue::Str(s) => Value::String(s),
            SpookyValue::Array(arr) => {
                Value::Array(arr.into_iter().map(|v| v.into()).collect())
            }
            SpookyValue::Object(obj) => {
                Value::Object(obj.into_iter().map(|(k, v)| (k, v.into())).collect())
            }
        }
    }
}
