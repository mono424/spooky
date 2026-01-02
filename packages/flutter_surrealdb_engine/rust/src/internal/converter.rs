use surrealdb::types::{Value, RecordId, Number, Array, Object};
use std::collections::{BTreeMap, HashMap};
use anyhow::Result;

// Helper: Takes &str to avoid unnecessary cloning
pub fn parse_vars(vars: Option<&str>) -> Result<HashMap<String, serde_json::Value>> {
    match vars {
        Some(v) if !v.is_empty() => Ok(serde_json::from_str(v)?),
        _ => Ok(HashMap::new()),
    }
}

pub fn json_to_surreal(v: serde_json::Value) -> Value {
    match v {
        serde_json::Value::Null => Value::None,
        serde_json::Value::Bool(b) => Value::Bool(b),
        serde_json::Value::Number(n) => {
            // Using into() is cleaner and handles the internal Number logic
            if let Some(i) = n.as_i64() {
                Value::Number(i.into())
            } else {
                Value::Number(n.as_f64().unwrap_or(0.0).into())
            }
        }
        serde_json::Value::String(s) => Value::String(s),
        serde_json::Value::Array(arr) => {
            // In 3.0, Array implements FromIterator<Value>
            Value::Array(arr.into_iter().map(json_to_surreal).collect())
        }
        serde_json::Value::Object(mut map) => {
            // OPTIMIZATION: Use remove() to take ownership of fields without cloning
            if map.len() == 2 && map.contains_key("table") && map.contains_key("key") {
                if let (Some(serde_json::Value::String(t)), Some(k)) = (map.remove("table"), map.remove("key")) {
                    let key_val = match k {
                        serde_json::Value::String(s) => s.into(),
                        serde_json::Value::Number(n) => n.to_string().into(),
                        serde_json::Value::Object(mut o) => {
                            // Extract nested values if they exist
                            if let Some(serde_json::Value::String(s)) = o.remove("String") {
                                s.into()
                            } else if let Some(n) = o.remove("Number") {
                                n.to_string().into()
                            } else {
                                // Fallback: serialize the whole sub-object if it doesn't match
                                serde_json::Value::Object(o).to_string().into()
                            }
                        }
                        _ => k.to_string().into(),
                    };

                    return Value::RecordId(RecordId {
                        table: t.into(),
                        key: key_val,
                    });
                }
            }

            // Normal Object: collect into BTreeMap efficiently
            let obj: BTreeMap<String, Value> = map
                .into_iter()
                .map(|(k, v)| (k, json_to_surreal(v)))
                .collect();
            
            Value::Object(obj.into())
        }
    }
}