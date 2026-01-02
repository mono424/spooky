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

// Recursive converter: JSON -> Surreal Value
pub fn json_to_surreal(v: serde_json::Value) -> Value {
    match v {
        serde_json::Value::Null => Value::None, 
        serde_json::Value::Bool(b) => Value::Bool(b),
        serde_json::Value::Number(n) => {
             if let Some(i) = n.as_i64() {
                 Value::Number(Number::from(i))
             } else if let Some(f) = n.as_f64() {
                 Value::Number(Number::from(f))
             } else {
                 Value::None 
             }
        },
        serde_json::Value::String(s) => Value::String(s),
        serde_json::Value::Array(arr) => {
            let converted: Vec<Value> = arr.into_iter().map(json_to_surreal).collect();
            Value::Array(Array::from(converted))
        },
        serde_json::Value::Object(map) => {
             // HEURISTIC: {"table": "...", "key": "..."} -> RecordId
             if map.len() == 2 && map.contains_key("table") && map.contains_key("key") {
                 let table_val = map.get("table");
                 let key_val = map.get("key");
                 
                 if let (Some(serde_json::Value::String(t)), Some(k)) = (table_val, key_val) {
                     let key_str = match k {
                         serde_json::Value::String(s) => s.clone(),
                         serde_json::Value::Number(n) => n.to_string(),
                         serde_json::Value::Object(o) => {
                             // Handle RecordIdKey variants serialization (e.g. {"String": "..."})
                             if let Some(serde_json::Value::String(s)) = o.get("String") {
                                 s.clone()
                             } else if let Some(n) = o.get("Number") {
                                 n.to_string()
                             } else {
                                 k.to_string() 
                             }
                         },
                         _ => k.to_string(), 
                     };
                     
                     let rid = RecordId {
                         table: t.as_str().into(),
                         key: key_str.as_str().into(),
                     };
                     return Value::RecordId(rid);
                 }
             }
             
             // Normal Object
             let mut obj = BTreeMap::new();
             for (k, v) in map {
                 obj.insert(k, json_to_surreal(v));
             }
             Value::Object(Object::from(obj))
        }
    }
}
