use super::zset::FastMap;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use smol_str::SmolStr;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SpookyValue {
    Null,
    Bool(bool),
    Number(f64),
    Str(SmolStr),
    Array(Vec<SpookyValue>),
    Object(FastMap<SmolStr, SpookyValue>),
}

impl Default for SpookyValue {
    fn default() -> Self {
        SpookyValue::Null
    }
}

impl SpookyValue {
    /// Get value as string reference
    pub fn as_str(&self) -> Option<&str> {
        match self {
            SpookyValue::Str(s) => Some(s.as_str()),
            _ => None,
        }
    }

    /// Get value as f64
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            SpookyValue::Number(n) => Some(*n),
            _ => None,
        }
    }

    /// Get value as bool
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            SpookyValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Get value as object reference
    pub fn as_object(&self) -> Option<&FastMap<SmolStr, SpookyValue>> {
        match self {
            SpookyValue::Object(map) => Some(map),
            _ => None,
        }
    }

    /// Get value as array reference
    pub fn as_array(&self) -> Option<&Vec<SpookyValue>> {
        match self {
            SpookyValue::Array(arr) => Some(arr),
            _ => None,
        }
    }

    /// Get nested value by key (for objects)
    pub fn get(&self, key: &str) -> Option<&SpookyValue> {
        self.as_object()?.get(&SmolStr::new(key))
    }

    /// Check if value is null
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
            Value::String(s) => SpookyValue::Str(SmolStr::from(s)),
            Value::Array(arr) => SpookyValue::Array(arr.into_iter().map(SpookyValue::from).collect()),
            Value::Object(obj) => SpookyValue::Object(
                obj.into_iter()
                    .map(|(k, v)| (SmolStr::from(k), SpookyValue::from(v)))
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
            SpookyValue::Str(s) => Value::String(s.to_string()),
            SpookyValue::Array(arr) => Value::Array(arr.into_iter().map(|v| v.into()).collect()),
            SpookyValue::Object(obj) => Value::Object(
                obj.into_iter().map(|(k, v)| (k.to_string(), v.into())).collect(),
            ),
        }
    }
}
