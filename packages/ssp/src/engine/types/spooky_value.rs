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
            Value::Array(arr) => {
                SpookyValue::Array(arr.into_iter().map(SpookyValue::from).collect())
            }
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
                obj.into_iter()
                    .map(|(k, v)| (k.to_string(), v.into()))
                    .collect(),
            ),
        }
    }
}

#[cfg(test)]
mod spooky_value_tests {
    use super::*;

    #[test]
    fn test_spooky_null() {
        let value = SpookyValue::Null;
        assert!(value.is_null());

        // Test that other accessors return None
        assert!(value.as_str().is_none());
        assert!(value.as_f64().is_none());
        assert!(value.as_bool().is_none());
        assert!(value.as_object().is_none());
        assert!(value.as_array().is_none());
    }

    #[test]
    fn test_spooky_bool() {
        // Test true
        let true_val = SpookyValue::Bool(true);
        assert_eq!(true_val.as_bool(), Some(true));

        // Test false
        let false_val = SpookyValue::Bool(false);
        assert_eq!(false_val.as_bool(), Some(false));

        // Test that other accessors return None
        assert!(true_val.as_str().is_none());
        assert!(true_val.as_f64().is_none());
        assert!(true_val.as_object().is_none());
        assert!(true_val.as_array().is_none());
        assert!(!true_val.is_null());
    }

    #[test]
    fn test_spooky_number() {
        // Test basic number
        let num = SpookyValue::Number(42.0);
        assert_eq!(num.as_f64(), Some(42.0));

        // Test decimal precision
        let decimal = SpookyValue::Number(3.14159265359);
        assert_eq!(decimal.as_f64(), Some(3.14159265359));

        // Test negative number
        let negative = SpookyValue::Number(-123.456);
        assert_eq!(negative.as_f64(), Some(-123.456));

        // Test zero
        let zero = SpookyValue::Number(0.0);
        assert_eq!(zero.as_f64(), Some(0.0));

        // Test very large number
        let large = SpookyValue::Number(1e308);
        assert_eq!(large.as_f64(), Some(1e308));

        // Test very small number
        let small = SpookyValue::Number(1e-308);
        assert_eq!(small.as_f64(), Some(1e-308));

        // Test that other accessors return None
        assert!(num.as_str().is_none());
        assert!(num.as_bool().is_none());
        assert!(num.as_object().is_none());
        assert!(num.as_array().is_none());
        assert!(!num.is_null());
    }

    #[test]
    fn test_spooky_number_special_values() {
        // Test NaN
        let nan = SpookyValue::Number(f64::NAN);
        assert!(nan.as_f64().unwrap().is_nan());

        // Test positive infinity
        let pos_inf = SpookyValue::Number(f64::INFINITY);
        assert_eq!(pos_inf.as_f64(), Some(f64::INFINITY));
        assert!(pos_inf.as_f64().unwrap().is_infinite());

        // Test negative infinity
        let neg_inf = SpookyValue::Number(f64::NEG_INFINITY);
        assert_eq!(neg_inf.as_f64(), Some(f64::NEG_INFINITY));
        assert!(neg_inf.as_f64().unwrap().is_infinite());
    }

    #[test]
    fn test_spooky_string() {
        let smol = SmolStr::new("test");
        let spooky_str = SpookyValue::Str("test".into());

        // Test equality
        assert_eq!(spooky_str, SpookyValue::Str(smol));

        // Test as_str accessor
        assert_eq!(spooky_str.as_str(), Some("test"));

        // Test empty string
        let empty = SpookyValue::Str(SmolStr::new(""));
        assert_eq!(empty.as_str(), Some(""));

        // Test that other accessors return None
        assert!(spooky_str.as_f64().is_none());
        assert!(spooky_str.as_bool().is_none());
        assert!(spooky_str.as_object().is_none());
        assert!(spooky_str.as_array().is_none());
        assert!(!spooky_str.is_null());
    }

    #[test]
    fn test_spooky_array() {
        // Test creating an array
        let arr = SpookyValue::Array(vec![
            SpookyValue::Number(1.0),
            SpookyValue::Number(2.0),
            SpookyValue::Number(3.0),
        ]);

        // Test as_array accessor
        let inner = arr.as_array();
        assert!(inner.is_some());
        assert_eq!(inner.unwrap().len(), 3);

        // Test accessing elements
        let elements = arr.as_array().unwrap();
        assert_eq!(elements[0].as_f64(), Some(1.0));
        assert_eq!(elements[1].as_f64(), Some(2.0));
        assert_eq!(elements[2].as_f64(), Some(3.0));

        // Test that other accessors return None
        assert!(arr.as_str().is_none());
        assert!(arr.as_f64().is_none());
        assert!(arr.as_bool().is_none());
        assert!(arr.as_object().is_none());
        assert!(!arr.is_null());
    }

    #[test]
    fn test_spooky_array_empty() {
        let arr = SpookyValue::Array(vec![]);

        assert!(arr.as_array().is_some());
        assert_eq!(arr.as_array().unwrap().len(), 0);
        assert!(arr.as_array().unwrap().is_empty());
    }

    #[test]
    fn test_spooky_array_mixed_types() {
        // Array with different SpookyValue types
        let arr = SpookyValue::Array(vec![
            SpookyValue::Null,
            SpookyValue::Bool(true),
            SpookyValue::Number(42.0),
            SpookyValue::Str(SmolStr::new("hello")),
            SpookyValue::Array(vec![SpookyValue::Number(1.0)]),
        ]);

        let elements = arr.as_array().unwrap();
        assert_eq!(elements.len(), 5);

        assert!(elements[0].is_null());
        assert_eq!(elements[1].as_bool(), Some(true));
        assert_eq!(elements[2].as_f64(), Some(42.0));
        assert_eq!(elements[3].as_str(), Some("hello"));
        assert!(elements[4].as_array().is_some());
    }

    #[test]
    fn test_spooky_array_nested() {
        // Nested arrays
        let inner1 = SpookyValue::Array(vec![SpookyValue::Number(1.0), SpookyValue::Number(2.0)]);
        let inner2 = SpookyValue::Array(vec![SpookyValue::Number(3.0), SpookyValue::Number(4.0)]);
        let outer = SpookyValue::Array(vec![inner1, inner2]);

        let outer_arr = outer.as_array().unwrap();
        assert_eq!(outer_arr.len(), 2);

        let first_inner = outer_arr[0].as_array().unwrap();
        assert_eq!(first_inner[0].as_f64(), Some(1.0));
        assert_eq!(first_inner[1].as_f64(), Some(2.0));

        let second_inner = outer_arr[1].as_array().unwrap();
        assert_eq!(second_inner[0].as_f64(), Some(3.0));
        assert_eq!(second_inner[1].as_f64(), Some(4.0));
    }

    #[test]
    fn test_spooky_object_empty() {
        let obj = SpookyValue::Object(FastMap::default());

        assert!(obj.as_object().is_some());
        assert!(obj.as_object().unwrap().is_empty());
        assert!(obj.get("anything").is_none());
    }

    #[test]
    fn test_spooky_object() {
        // Create object manually
        let mut map: FastMap<SmolStr, SpookyValue> = FastMap::default();
        map.insert(
            SmolStr::new("name"),
            SpookyValue::Str(SmolStr::new("Alice")),
        );
        map.insert(SmolStr::new("age"), SpookyValue::Number(30.0));

        let obj = SpookyValue::Object(map);

        // Test as_object accessor
        assert!(obj.as_object().is_some());
        assert_eq!(obj.as_object().unwrap().len(), 2);

        // Test get method for key lookup
        assert_eq!(obj.get("name").and_then(|v| v.as_str()), Some("Alice"));
        assert_eq!(obj.get("age").and_then(|v| v.as_f64()), Some(30.0));

        // Test missing key
        assert!(obj.get("missing").is_none());

        // Test that other accessors return None
        assert!(obj.as_str().is_none());
        assert!(obj.as_f64().is_none());
        assert!(obj.as_bool().is_none());
        assert!(obj.as_array().is_none());
        assert!(!obj.is_null());
    }

    #[test]
    fn test_spooky_object_nested() {
        // Create nested object: { "user": { "profile": { "id": 123 } } }
        let mut inner_most: FastMap<SmolStr, SpookyValue> = FastMap::default();
        inner_most.insert(SmolStr::new("id"), SpookyValue::Number(123.0));

        let mut middle: FastMap<SmolStr, SpookyValue> = FastMap::default();
        middle.insert(SmolStr::new("profile"), SpookyValue::Object(inner_most));

        let mut outer: FastMap<SmolStr, SpookyValue> = FastMap::default();
        outer.insert(SmolStr::new("user"), SpookyValue::Object(middle));

        let obj = SpookyValue::Object(outer);

        // Navigate nested structure
        let user = obj.get("user").unwrap();
        let profile = user.get("profile").unwrap();
        let id = profile.get("id").unwrap();

        assert_eq!(id.as_f64(), Some(123.0));
    }

    #[test]
    fn test_spooky_object_mixed_values() {
        let mut map: FastMap<SmolStr, SpookyValue> = FastMap::default();
        map.insert(SmolStr::new("null_field"), SpookyValue::Null);
        map.insert(SmolStr::new("bool_field"), SpookyValue::Bool(true));
        map.insert(SmolStr::new("num_field"), SpookyValue::Number(42.0));
        map.insert(
            SmolStr::new("str_field"),
            SpookyValue::Str(SmolStr::new("hello")),
        );
        map.insert(
            SmolStr::new("arr_field"),
            SpookyValue::Array(vec![SpookyValue::Number(1.0)]),
        );

        let obj = SpookyValue::Object(map);

        assert!(obj.get("null_field").unwrap().is_null());
        assert_eq!(obj.get("bool_field").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(obj.get("num_field").and_then(|v| v.as_f64()), Some(42.0));
        assert_eq!(obj.get("str_field").and_then(|v| v.as_str()), Some("hello"));
        assert_eq!(
            obj.get("arr_field")
                .and_then(|v| v.as_array())
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn test_spooky_from_json_null() {
        let json = serde_json::Value::Null;
        let spooky: SpookyValue = json.into();

        assert!(spooky.is_null());
        assert_eq!(spooky, SpookyValue::Null);
    }

    #[test]
    fn test_spooky_from_json_primitives() {
        // Bool - true
        let json_true = serde_json::json!(true);
        let spooky_true: SpookyValue = json_true.into();
        assert_eq!(spooky_true.as_bool(), Some(true));

        // Bool - false
        let json_false = serde_json::json!(false);
        let spooky_false: SpookyValue = json_false.into();
        assert_eq!(spooky_false.as_bool(), Some(false));

        // Number - integer
        let json_int = serde_json::json!(42);
        let spooky_int: SpookyValue = json_int.into();
        assert_eq!(spooky_int.as_f64(), Some(42.0));

        // Number - float
        let json_float = serde_json::json!(3.14159);
        let spooky_float: SpookyValue = json_float.into();
        assert_eq!(spooky_float.as_f64(), Some(3.14159));

        // Number - negative
        let json_neg = serde_json::json!(-100);
        let spooky_neg: SpookyValue = json_neg.into();
        assert_eq!(spooky_neg.as_f64(), Some(-100.0));

        // Number - zero
        let json_zero = serde_json::json!(0);
        let spooky_zero: SpookyValue = json_zero.into();
        assert_eq!(spooky_zero.as_f64(), Some(0.0));

        // String - regular
        let json_str = serde_json::json!("hello world");
        let spooky_str: SpookyValue = json_str.into();
        assert_eq!(spooky_str.as_str(), Some("hello world"));

        // String - empty
        let json_empty = serde_json::json!("");
        let spooky_empty: SpookyValue = json_empty.into();
        assert_eq!(spooky_empty.as_str(), Some(""));

        // String - unicode
        let json_unicode = serde_json::json!("こんにちは 🎉");
        let spooky_unicode: SpookyValue = json_unicode.into();
        assert_eq!(spooky_unicode.as_str(), Some("こんにちは 🎉"));
    }

    #[test]
    fn test_spooky_from_json_array() {
        // Empty array
        let json_empty = serde_json::json!([]);
        let spooky_empty: SpookyValue = json_empty.into();
        assert!(spooky_empty.as_array().unwrap().is_empty());

        // Homogeneous array (all numbers)
        let json_nums = serde_json::json!([1, 2, 3, 4, 5]);
        let spooky_nums: SpookyValue = json_nums.into();
        let arr = spooky_nums.as_array().unwrap();
        assert_eq!(arr.len(), 5);
        assert_eq!(arr[0].as_f64(), Some(1.0));
        assert_eq!(arr[4].as_f64(), Some(5.0));

        // Heterogeneous array (mixed types)
        let json_mixed = serde_json::json!([null, true, 42, "text", [1, 2], {"key": "value"}]);
        let spooky_mixed: SpookyValue = json_mixed.into();
        let arr = spooky_mixed.as_array().unwrap();
        assert_eq!(arr.len(), 6);
        assert!(arr[0].is_null());
        assert_eq!(arr[1].as_bool(), Some(true));
        assert_eq!(arr[2].as_f64(), Some(42.0));
        assert_eq!(arr[3].as_str(), Some("text"));
        assert!(arr[4].as_array().is_some());
        assert!(arr[5].as_object().is_some());

        // Nested arrays
        let json_nested = serde_json::json!([[1, 2], [3, 4], [5, 6]]);
        let spooky_nested: SpookyValue = json_nested.into();
        let outer = spooky_nested.as_array().unwrap();
        assert_eq!(outer.len(), 3);
        assert_eq!(outer[0].as_array().unwrap()[0].as_f64(), Some(1.0));
        assert_eq!(outer[1].as_array().unwrap()[1].as_f64(), Some(4.0));
        assert_eq!(outer[2].as_array().unwrap()[0].as_f64(), Some(5.0));
    }

    #[test]
    fn test_spooky_from_json_nested_object() {
        // Simple nested object
        let json_simple = serde_json::json!({
            "level1": {
                "level2": {
                    "value": 123
                }
            }
        });
        let spooky_simple: SpookyValue = json_simple.into();
        let level1 = spooky_simple.get("level1").unwrap();
        let level2 = level1.get("level2").unwrap();
        let value = level2.get("value").unwrap();
        assert_eq!(value.as_f64(), Some(123.0));

        // Complex nested object (realistic example)
        let json_complex = serde_json::json!({
            "user": {
                "id": "user:abc123",
                "profile": {
                    "name": "Alice",
                    "age": 30,
                    "verified": true,
                    "tags": ["admin", "developer"],
                    "metadata": {
                        "created_at": "2024-01-01",
                        "login_count": 42
                    }
                },
                "settings": {
                    "theme": "dark",
                    "notifications": {
                        "email": true,
                        "push": false
                    }
                }
            }
        });

        let spooky_complex: SpookyValue = json_complex.into();

        // Navigate and verify structure
        let user = spooky_complex.get("user").unwrap();
        assert_eq!(user.get("id").and_then(|v| v.as_str()), Some("user:abc123"));

        let profile = user.get("profile").unwrap();
        assert_eq!(profile.get("name").and_then(|v| v.as_str()), Some("Alice"));
        assert_eq!(profile.get("age").and_then(|v| v.as_f64()), Some(30.0));
        assert_eq!(
            profile.get("verified").and_then(|v| v.as_bool()),
            Some(true)
        );

        let tags = profile.get("tags").and_then(|v| v.as_array()).unwrap();
        assert_eq!(tags.len(), 2);
        assert_eq!(tags[0].as_str(), Some("admin"));
        assert_eq!(tags[1].as_str(), Some("developer"));

        let metadata = profile.get("metadata").unwrap();
        assert_eq!(
            metadata.get("created_at").and_then(|v| v.as_str()),
            Some("2024-01-01")
        );
        assert_eq!(
            metadata.get("login_count").and_then(|v| v.as_f64()),
            Some(42.0)
        );

        let settings = user.get("settings").unwrap();
        assert_eq!(settings.get("theme").and_then(|v| v.as_str()), Some("dark"));

        let notifications = settings.get("notifications").unwrap();
        assert_eq!(
            notifications.get("email").and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            notifications.get("push").and_then(|v| v.as_bool()),
            Some(false)
        );
    }

    #[test]
    fn test_spooky_to_json_roundtrip() {
        // Null roundtrip
        let null_orig = SpookyValue::Null;
        let null_json: serde_json::Value = null_orig.clone().into();
        let null_back: SpookyValue = null_json.into();
        assert_eq!(null_orig, null_back);

        // Bool roundtrip
        let bool_orig = SpookyValue::Bool(true);
        let bool_json: serde_json::Value = bool_orig.clone().into();
        let bool_back: SpookyValue = bool_json.into();
        assert_eq!(bool_orig, bool_back);

        // Number roundtrip
        let num_orig = SpookyValue::Number(42.5);
        let num_json: serde_json::Value = num_orig.clone().into();
        let num_back: SpookyValue = num_json.into();
        assert_eq!(num_orig, num_back);

        // String roundtrip
        let str_orig = SpookyValue::Str(SmolStr::new("hello"));
        let str_json: serde_json::Value = str_orig.clone().into();
        let str_back: SpookyValue = str_json.into();
        assert_eq!(str_orig, str_back);

        // Array roundtrip
        let arr_orig = SpookyValue::Array(vec![
            SpookyValue::Number(1.0),
            SpookyValue::Str(SmolStr::new("two")),
            SpookyValue::Bool(false),
        ]);
        let arr_json: serde_json::Value = arr_orig.clone().into();
        let arr_back: SpookyValue = arr_json.into();
        assert_eq!(arr_orig, arr_back);

        // Object roundtrip
        let mut map: FastMap<SmolStr, SpookyValue> = FastMap::default();
        map.insert(
            SmolStr::new("name"),
            SpookyValue::Str(SmolStr::new("Alice")),
        );
        map.insert(SmolStr::new("age"), SpookyValue::Number(30.0));
        let obj_orig = SpookyValue::Object(map);
        let obj_json: serde_json::Value = obj_orig.clone().into();
        let obj_back: SpookyValue = obj_json.into();
        assert_eq!(obj_orig, obj_back);
    }

    #[test]
    fn test_spooky_to_json_roundtrip_complex() {
        // Complex nested structure
        let mut inner: FastMap<SmolStr, SpookyValue> = FastMap::default();
        inner.insert(
            SmolStr::new("id"),
            SpookyValue::Str(SmolStr::new("user:123")),
        );
        inner.insert(SmolStr::new("score"), SpookyValue::Number(99.5));
        inner.insert(SmolStr::new("active"), SpookyValue::Bool(true));
        inner.insert(
            SmolStr::new("tags"),
            SpookyValue::Array(vec![
                SpookyValue::Str(SmolStr::new("admin")),
                SpookyValue::Str(SmolStr::new("verified")),
            ]),
        );

        let mut outer: FastMap<SmolStr, SpookyValue> = FastMap::default();
        outer.insert(SmolStr::new("user"), SpookyValue::Object(inner));
        outer.insert(SmolStr::new("timestamp"), SpookyValue::Number(1234567890.0));

        let orig = SpookyValue::Object(outer);

        // Roundtrip
        let json: serde_json::Value = orig.clone().into();
        let back: SpookyValue = json.into();

        assert_eq!(orig, back);

        // Verify structure preserved
        let user = back.get("user").unwrap();
        assert_eq!(user.get("id").and_then(|v| v.as_str()), Some("user:123"));
        assert_eq!(user.get("score").and_then(|v| v.as_f64()), Some(99.5));
        assert_eq!(user.get("active").and_then(|v| v.as_bool()), Some(true));

        let tags = user.get("tags").and_then(|v| v.as_array()).unwrap();
        assert_eq!(tags.len(), 2);
        assert_eq!(tags[0].as_str(), Some("admin"));
    }

    #[test]
    fn test_spooky_default() {
        let default_value = SpookyValue::default();

        // Default should be Null
        assert_eq!(default_value, SpookyValue::Null);
        assert!(default_value.is_null());

        // Verify other accessors return None
        assert!(default_value.as_str().is_none());
        assert!(default_value.as_f64().is_none());
        assert!(default_value.as_bool().is_none());
        assert!(default_value.as_object().is_none());
        assert!(default_value.as_array().is_none());
    }

    #[test]
    fn test_spooky_default_in_context() {
        // Test default in a Vec context
        let defaults: Vec<SpookyValue> = vec![
            SpookyValue::default(),
            SpookyValue::default(),
            SpookyValue::default(),
        ];

        for val in defaults {
            assert!(val.is_null());
        }

        // Test default in Option::unwrap_or_default context
        let maybe_value: Option<SpookyValue> = None;
        let value = maybe_value.unwrap_or_default();
        assert!(value.is_null());
    }
}
