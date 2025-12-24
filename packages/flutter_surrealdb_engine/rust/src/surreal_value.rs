use serde::{de::{self, Visitor, MapAccess, Deserialize}, Deserializer};
use std::fmt;

#[derive(Debug, PartialEq)]
pub enum SurrealValue {
    Strand(String),
    Datetime(String),
    Number(f64),
    Object(std::collections::HashMap<String, SurrealValue>),
    Array(Vec<SurrealValue>),
    None,
}

impl<'de> Deserialize<'de> for SurrealValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where D: Deserializer<'de> {
        struct SurrealVisitor;

        impl<'de> Visitor<'de> for SurrealVisitor {
            type Value = SurrealValue;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a SurrealDB extended JSON value")
            }

            // Processing Maps (Here unwrapping happens)
            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where M: MapAccess<'de> {
                let key: Option<String> = map.next_key()?;
                
                if let Some(k) = key {
                    match k.as_str() {
                        "Strand" => Ok(SurrealValue::Strand(map.next_value()?)),
                        "Datetime" => Ok(SurrealValue::Datetime(map.next_value()?)),
                        "Number" => Ok(SurrealValue::Number(map.next_value()?)),
                        "Object" => Ok(SurrealValue::Object(map.next_value()?)),
                        "Array" => Ok(SurrealValue::Array(map.next_value()?)),
                        // If it's not a tag, process as normal object
                        _ => {
                            let mut obj = std::collections::HashMap::new();
                            obj.insert(k, map.next_value()?);
                            while let Some((nk, nv)) = map.next_entry()? {
                                obj.insert(nk, nv);
                            }
                            Ok(SurrealValue::Object(obj))
                        }
                    }
                } else {
                    Ok(SurrealValue::None)
                }
            }

            // Processing Lists
            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where A: de::SeqAccess<'de> {
                let mut vec = Vec::new();
                while let Some(el) = seq.next_element()? {
                    vec.push(el);
                }
                Ok(SurrealValue::Array(vec))
            }

            // Primitive Types (if JSON is already flat)
            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E> {
                Ok(SurrealValue::Strand(v.to_owned()))
            }
        }

        deserializer.deserialize_any(SurrealVisitor)
    }
}

mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_deserialize_strand() {
        let json = json!({"Strand": "hello"});
        let val: SurrealValue = serde_json::from_value(json).unwrap();
        assert_eq!(val, SurrealValue::Strand("hello".to_string()));
    }

    #[test]
    fn test_deserialize_datetime() {
        let json = json!({"Datetime": "2023-10-27T10:00:00Z"});
        let val: SurrealValue = serde_json::from_value(json).unwrap();
        assert_eq!(val, SurrealValue::Datetime("2023-10-27T10:00:00Z".to_string()));
    }

     #[test]
    fn test_deserialize_number() {
        // SurrealDB often wraps numbers like {"Number": 123.45} or sometimes plain?
        // Let's test the wrapped version as per visitor logic
        let json = json!({"Number": 123.45});
        let val: SurrealValue = serde_json::from_value(json).unwrap();
        assert_eq!(val, SurrealValue::Number(123.45));
    }

    #[test]
    fn test_deserialize_array_wrapped() {
        let json = json!({"Array": [{"Strand": "item1"}, {"Strand": "item2"}]});
        let val: SurrealValue = serde_json::from_value(json).unwrap();
        if let SurrealValue::Array(arr) = val {
            assert_eq!(arr.len(), 2);
            assert_eq!(arr[0], SurrealValue::Strand("item1".to_string()));
             assert_eq!(arr[1], SurrealValue::Strand("item2".to_string()));
        } else {
            panic!("Expected Array");
        }
    }

     #[test]
    fn test_deserialize_flat_array() {
        let json = json!([{"Strand": "item1"}, {"Strand": "item2"}]);
        let val: SurrealValue = serde_json::from_value(json).unwrap();
        if let SurrealValue::Array(arr) = val {
            assert_eq!(arr.len(), 2);
             assert_eq!(arr[0], SurrealValue::Strand("item1".to_string()));
             assert_eq!(arr[1], SurrealValue::Strand("item2".to_string()));
        } else {
            panic!("Expected Array");
        }
    }

    #[test]
    fn test_deserialize_nested_object() {
        let json = json!({
            "key1": {"Strand": "value1"},
            "key2": {"Number": 42.0}
        });
        let val: SurrealValue = serde_json::from_value(json).unwrap();
        if let SurrealValue::Object(map) = val {
            assert_eq!(map.get("key1"), Some(&SurrealValue::Strand("value1".to_string())));
            assert_eq!(map.get("key2"), Some(&SurrealValue::Number(42.0)));
        } else {
            panic!("Expected Object");
        }
    }
}
