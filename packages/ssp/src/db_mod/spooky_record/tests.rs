// ═══════════════════════════════════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════════════════════════════════
mod spooky_record_tests {
    use crate::serialization::{from_bytes, from_spooky, serialize_into};
    use crate::spooky_record::SpookyReadable;
    use crate::spooky_record::SpookyRecord;
    use crate::spooky_value::{FastMap, SpookyValue};
    use crate::types::*;
    use smol_str::SmolStr;

    fn make_test_record() -> SpookyValue {
        let mut map = FastMap::new();
        map.insert(SmolStr::from("id"), SpookyValue::from("user:123"));
        map.insert(SmolStr::from("name"), SpookyValue::from("Alice"));
        map.insert(SmolStr::from("age"), SpookyValue::from(30i64));
        map.insert(SmolStr::from("score"), SpookyValue::from(99.5f64));
        map.insert(SmolStr::from("active"), SpookyValue::from(true));
        map.insert(SmolStr::from("version"), SpookyValue::from(42u64));
        SpookyValue::Object(map)
    }

    /// Record with exactly 4 fields — exercises the linear-search path.
    fn make_linear_record() -> SpookyValue {
        let mut map = FastMap::new();
        map.insert(SmolStr::from("a"), SpookyValue::from("alpha"));
        map.insert(SmolStr::from("b"), SpookyValue::from(1i64));
        map.insert(SmolStr::from("c"), SpookyValue::from(2.0f64));
        map.insert(SmolStr::from("d"), SpookyValue::from(true));
        SpookyValue::Object(map)
    }

    /// Build a simple single-field record.
    fn make_single_field(key: &str, val: SpookyValue) -> SpookyValue {
        let mut map = FastMap::new();
        map.insert(SmolStr::from(key), val);
        SpookyValue::Object(map)
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Basic round-trip
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn test_roundtrip_flat_fields() {
        let original = make_test_record();
        let (buf, fc) = from_spooky(&original).unwrap();
        let record = SpookyRecord::new(&buf, fc);

        assert_eq!(record.field_count(), 6);
        assert_eq!(record.get_str("id"), Some("user:123"));
        assert_eq!(record.get_str("name"), Some("Alice"));
        assert_eq!(record.get_i64("age"), Some(30));
        assert_eq!(record.get_f64("score"), Some(99.5));
        assert_eq!(record.get_bool("active"), Some(true));
        assert_eq!(record.get_u64("version"), Some(42));
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Empty record
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn test_empty_record() {
        let obj = SpookyValue::Object(FastMap::new());
        let (buf, fc) = from_spooky(&obj).unwrap();
        let record = SpookyRecord::new(&buf, fc);

        assert_eq!(record.field_count(), 0);
        assert!(!record.has_field("anything"));
        assert!(record.get_str("anything").is_none());
        assert!(record.get_i64("anything").is_none());
        assert!(record.get_f64("anything").is_none());
        assert!(record.get_u64("anything").is_none());
        assert!(record.get_bool("anything").is_none());
        assert!(record.get_raw("anything").is_none());
        assert!(record.get_field::<SpookyValue>("anything").is_none());
        assert!(record.get_number_as_f64("anything").is_none());
        assert!(record.field_type("anything").is_none());
        assert_eq!(record.iter_fields().count(), 0);
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Single-field records (one per type tag)
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn test_single_string_field() {
        let obj = make_single_field("s", SpookyValue::from("hello"));
        let (buf, fc) = from_spooky(&obj).unwrap();
        let record = SpookyRecord::new(&buf, fc);

        assert_eq!(record.field_count(), 1);
        assert_eq!(record.get_str("s"), Some("hello"));
        assert!(record.has_field("s"));
        assert_eq!(record.field_type("s"), Some(TAG_STR));
    }

    #[test]
    fn test_single_i64_field() {
        let obj = make_single_field("n", SpookyValue::from(-999i64));
        let (buf, fc) = from_spooky(&obj).unwrap();
        let record = SpookyRecord::new(&buf, fc);

        assert_eq!(record.get_i64("n"), Some(-999));
        assert_eq!(record.field_type("n"), Some(TAG_I64));
    }

    #[test]
    fn test_single_u64_field() {
        let obj = make_single_field("n", SpookyValue::from(u64::MAX));
        let (buf, fc) = from_spooky(&obj).unwrap();
        let record = SpookyRecord::new(&buf, fc);

        assert_eq!(record.get_u64("n"), Some(u64::MAX));
        assert_eq!(record.field_type("n"), Some(TAG_U64));
    }

    #[test]
    fn test_single_f64_field() {
        let obj = make_single_field("n", SpookyValue::from(std::f64::consts::PI));
        let (buf, fc) = from_spooky(&obj).unwrap();
        let record = SpookyRecord::new(&buf, fc);

        assert_eq!(record.get_f64("n"), Some(std::f64::consts::PI));
        assert_eq!(record.field_type("n"), Some(TAG_F64));
    }

    #[test]
    fn test_single_bool_true() {
        let obj = make_single_field("b", SpookyValue::from(true));
        let (buf, fc) = from_spooky(&obj).unwrap();
        let record = SpookyRecord::new(&buf, fc);

        assert_eq!(record.get_bool("b"), Some(true));
        assert_eq!(record.field_type("b"), Some(TAG_BOOL));
    }

    #[test]
    fn test_single_bool_false() {
        let obj = make_single_field("b", SpookyValue::from(false));
        let (buf, fc) = from_spooky(&obj).unwrap();
        let record = SpookyRecord::new(&buf, fc);

        assert_eq!(record.get_bool("b"), Some(false));
    }

    #[test]
    fn test_single_null_field() {
        let obj = make_single_field("x", SpookyValue::Null);
        let (buf, fc) = from_spooky(&obj).unwrap();
        let record = SpookyRecord::new(&buf, fc);

        assert_eq!(record.field_type("x"), Some(TAG_NULL));
        assert_eq!(record.get_field::<SpookyValue>("x"), Some(SpookyValue::Null));
        // Null must not be returned by typed getters
        assert!(record.get_str("x").is_none());
        assert!(record.get_i64("x").is_none());
        assert!(record.get_f64("x").is_none());
        assert!(record.get_u64("x").is_none());
        assert!(record.get_bool("x").is_none());
        assert!(record.get_number_as_f64("x").is_none());
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Type-mismatch: every getter returns None for wrong type
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn test_type_mismatch_get_str_on_i64() {
        let obj = make_single_field("n", SpookyValue::from(42i64));
        let (buf, fc) = from_spooky(&obj).unwrap();
        let record = SpookyRecord::new(&buf, fc);
        assert!(record.get_str("n").is_none());
    }

    #[test]
    fn test_type_mismatch_get_i64_on_str() {
        let obj = make_single_field("s", SpookyValue::from("text"));
        let (buf, fc) = from_spooky(&obj).unwrap();
        let record = SpookyRecord::new(&buf, fc);
        assert!(record.get_i64("s").is_none());
    }

    #[test]
    fn test_type_mismatch_get_f64_on_bool() {
        let obj = make_single_field("b", SpookyValue::from(true));
        let (buf, fc) = from_spooky(&obj).unwrap();
        let record = SpookyRecord::new(&buf, fc);
        assert!(record.get_f64("b").is_none());
    }

    #[test]
    fn test_type_mismatch_get_u64_on_f64() {
        let obj = make_single_field("f", SpookyValue::from(1.5f64));
        let (buf, fc) = from_spooky(&obj).unwrap();
        let record = SpookyRecord::new(&buf, fc);
        assert!(record.get_u64("f").is_none());
    }

    #[test]
    fn test_type_mismatch_get_bool_on_str() {
        let obj = make_single_field("s", SpookyValue::from("yes"));
        let (buf, fc) = from_spooky(&obj).unwrap();
        let record = SpookyRecord::new(&buf, fc);
        assert!(record.get_bool("s").is_none());
    }

    #[test]
    fn test_type_mismatch_get_i64_on_u64() {
        let obj = make_single_field("n", SpookyValue::from(7u64));
        let (buf, fc) = from_spooky(&obj).unwrap();
        let record = SpookyRecord::new(&buf, fc);
        // i64 and u64 have different type tags
        assert!(record.get_i64("n").is_none());
    }

    #[test]
    fn test_type_mismatch_get_u64_on_i64() {
        let obj = make_single_field("n", SpookyValue::from(7i64));
        let (buf, fc) = from_spooky(&obj).unwrap();
        let record = SpookyRecord::new(&buf, fc);
        assert!(record.get_u64("n").is_none());
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Missing field — every getter
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn test_missing_field() {
        let original = make_test_record();
        let (buf, fc) = from_spooky(&original).unwrap();
        let record = SpookyRecord::new(&buf, fc);

        assert!(record.get_raw("nonexistent").is_none());
        assert!(record.get_str("nonexistent").is_none());
        assert!(record.get_i64("nonexistent").is_none());
        assert!(record.get_u64("nonexistent").is_none());
        assert!(record.get_f64("nonexistent").is_none());
        assert!(record.get_bool("nonexistent").is_none());
        assert!(record.get_field::<SpookyValue>("nonexistent").is_none());
        assert!(record.get_number_as_f64("nonexistent").is_none());
        assert!(!record.has_field("nonexistent"));
        assert!(record.field_type("nonexistent").is_none());
    }

    // ═══════════════════════════════════════════════════════════════════════
    // has_field
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn test_has_field() {
        let original = make_test_record();
        let (buf, fc) = from_spooky(&original).unwrap();
        let record = SpookyRecord::new(&buf, fc);

        assert!(record.has_field("id"));
        assert!(record.has_field("age"));
        assert!(record.has_field("score"));
        assert!(record.has_field("active"));
        assert!(record.has_field("version"));
        assert!(record.has_field("name"));
        assert!(!record.has_field("missing"));
    }

    // ═══════════════════════════════════════════════════════════════════════
    // get_number_as_f64 — cross-type conversion
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn test_get_number_as_f64() {
        let original = make_test_record();
        let (buf, fc) = from_spooky(&original).unwrap();
        let record = SpookyRecord::new(&buf, fc);

        // i64 → f64
        assert_eq!(record.get_number_as_f64("age"), Some(30.0));
        // f64 → f64
        assert_eq!(record.get_number_as_f64("score"), Some(99.5));
        // u64 → f64
        assert_eq!(record.get_number_as_f64("version"), Some(42.0));
        // string → None
        assert_eq!(record.get_number_as_f64("name"), None);
        // bool → None
        assert_eq!(record.get_number_as_f64("active"), None);
        // missing → None
        assert_eq!(record.get_number_as_f64("nope"), None);
    }

    #[test]
    fn test_get_number_as_f64_negative_i64() {
        let obj = make_single_field("n", SpookyValue::from(-42i64));
        let (buf, fc) = from_spooky(&obj).unwrap();
        let record = SpookyRecord::new(&buf, fc);
        assert_eq!(record.get_number_as_f64("n"), Some(-42.0));
    }

    #[test]
    fn test_get_number_as_f64_zero() {
        let obj = make_single_field("n", SpookyValue::from(0i64));
        let (buf, fc) = from_spooky(&obj).unwrap();
        let record = SpookyRecord::new(&buf, fc);
        assert_eq!(record.get_number_as_f64("n"), Some(0.0));
    }

    #[test]
    fn test_get_number_as_f64_large_u64() {
        let obj = make_single_field("n", SpookyValue::from(u64::MAX));
        let (buf, fc) = from_spooky(&obj).unwrap();
        let record = SpookyRecord::new(&buf, fc);
        assert_eq!(record.get_number_as_f64("n"), Some(u64::MAX as f64));
    }

    #[test]
    fn test_get_number_as_f64_on_null() {
        let obj = make_single_field("x", SpookyValue::Null);
        let (buf, fc) = from_spooky(&obj).unwrap();
        let record = SpookyRecord::new(&buf, fc);
        assert_eq!(record.get_number_as_f64("x"), None);
    }

    // ═══════════════════════════════════════════════════════════════════════
    // field_type — every tag
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn test_field_type_all_tags() {
        let original = make_test_record();
        let (buf, fc) = from_spooky(&original).unwrap();
        let record = SpookyRecord::new(&buf, fc);

        assert_eq!(record.field_type("id"), Some(TAG_STR));
        assert_eq!(record.field_type("age"), Some(TAG_I64));
        assert_eq!(record.field_type("score"), Some(TAG_F64));
        assert_eq!(record.field_type("version"), Some(TAG_U64));
        assert_eq!(record.field_type("active"), Some(TAG_BOOL));
        assert_eq!(record.field_type("nope"), None);
    }

    #[test]
    fn test_field_type_null() {
        let obj = make_single_field("x", SpookyValue::Null);
        let (buf, fc) = from_spooky(&obj).unwrap();
        let record = SpookyRecord::new(&buf, fc);
        assert_eq!(record.field_type("x"), Some(TAG_NULL));
    }

    #[test]
    fn test_field_type_nested_cbor() {
        let mut map = FastMap::new();
        map.insert(
            SmolStr::from("arr"),
            SpookyValue::Array(vec![SpookyValue::from(1i64)]),
        );
        let obj = SpookyValue::Object(map);
        let (buf, fc) = from_spooky(&obj).unwrap();
        let record = SpookyRecord::new(&buf, fc);
        assert_eq!(record.field_type("arr"), Some(TAG_NESTED_CBOR));
    }

    // ═══════════════════════════════════════════════════════════════════════
    // get_raw
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn test_get_raw_returns_correct_data() {
        let obj = make_single_field("val", SpookyValue::from(42i64));
        let (buf, fc) = from_spooky(&obj).unwrap();
        let record = SpookyRecord::new(&buf, fc);

        let raw = record.get_raw("val").unwrap();
        assert_eq!(raw.type_tag, TAG_I64);
        assert_eq!(raw.data.len(), 8);
        assert_eq!(i64::from_le_bytes(raw.data.try_into().unwrap()), 42);
    }

    #[test]
    fn test_get_raw_string_bytes() {
        let obj = make_single_field("s", SpookyValue::from("abc"));
        let (buf, fc) = from_spooky(&obj).unwrap();
        let record = SpookyRecord::new(&buf, fc);

        let raw = record.get_raw("s").unwrap();
        assert_eq!(raw.type_tag, TAG_STR);
        assert_eq!(raw.data, b"abc");
    }

    #[test]
    fn test_get_raw_null_zero_length() {
        let obj = make_single_field("x", SpookyValue::Null);
        let (buf, fc) = from_spooky(&obj).unwrap();
        let record = SpookyRecord::new(&buf, fc);

        let raw = record.get_raw("x").unwrap();
        assert_eq!(raw.type_tag, TAG_NULL);
        assert_eq!(raw.data.len(), 0);
    }

    // ═══════════════════════════════════════════════════════════════════════
    // get_field (full decode via CBOR / native)
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn test_get_field_decodes_all_flat_types() {
        let original = make_test_record();
        let (buf, fc) = from_spooky(&original).unwrap();
        let record = SpookyRecord::new(&buf, fc);

        assert_eq!(record.get_field::<SpookyValue>("id"), Some(SpookyValue::from("user:123")));
        assert_eq!(record.get_field::<SpookyValue>("age"), Some(SpookyValue::from(30i64)));
        assert_eq!(record.get_field::<SpookyValue>("score"), Some(SpookyValue::from(99.5f64)));
        assert_eq!(record.get_field::<SpookyValue>("active"), Some(SpookyValue::from(true)));
        assert_eq!(record.get_field::<SpookyValue>("version"), Some(SpookyValue::from(42u64)));
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Nested CBOR (objects and arrays)
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn test_nested_cbor_object() {
        let mut map = FastMap::new();
        let mut inner = FastMap::new();
        inner.insert(SmolStr::from("city"), SpookyValue::from("Berlin"));
        map.insert(SmolStr::from("address"), SpookyValue::Object(inner));
        let obj = SpookyValue::Object(map);

        let (buf, fc) = from_spooky(&obj).unwrap();
        let record = SpookyRecord::new(&buf, fc);

        let addr = record.get_field::<SpookyValue>("address").unwrap();
        assert_eq!(addr.get("city").and_then(|v| v.as_str()), Some("Berlin"));
    }

    #[test]
    fn test_nested_cbor_array() {
        let mut map = FastMap::new();
        map.insert(
            SmolStr::from("tags"),
            SpookyValue::Array(vec![
                SpookyValue::from("a"),
                SpookyValue::from("b"),
                SpookyValue::from("c"),
            ]),
        );
        let obj = SpookyValue::Object(map);

        let (buf, fc) = from_spooky(&obj).unwrap();
        let record = SpookyRecord::new(&buf, fc);

        let tags = record.get_field::<SpookyValue>("tags").unwrap();
        let arr = tags.as_array().unwrap();
        assert_eq!(arr.len(), 3);
    }

    #[test]
    fn test_nested_cbor_empty_array() {
        let mut map = FastMap::new();
        map.insert(SmolStr::from("empty"), SpookyValue::Array(vec![]));
        let obj = SpookyValue::Object(map);

        let (buf, fc) = from_spooky(&obj).unwrap();
        let record = SpookyRecord::new(&buf, fc);

        let val = record.get_field::<SpookyValue>("empty").unwrap();
        assert_eq!(val.as_array().unwrap().len(), 0);
    }

    #[test]
    fn test_nested_cbor_empty_object() {
        let mut map = FastMap::new();
        map.insert(SmolStr::from("obj"), SpookyValue::Object(FastMap::new()));
        let obj = SpookyValue::Object(map);

        let (buf, fc) = from_spooky(&obj).unwrap();
        let record = SpookyRecord::new(&buf, fc);

        let val = record.get_field::<SpookyValue>("obj").unwrap();
        assert!(val.as_object().unwrap().is_empty());
    }

    #[test]
    fn test_deeply_nested_cbor() {
        let mut level3 = FastMap::new();
        level3.insert(SmolStr::from("deep"), SpookyValue::from("value"));
        let mut level2 = FastMap::new();
        level2.insert(SmolStr::from("l3"), SpookyValue::Object(level3));
        let mut level1 = FastMap::new();
        level1.insert(SmolStr::from("l2"), SpookyValue::Object(level2));
        let mut root = FastMap::new();
        root.insert(SmolStr::from("l1"), SpookyValue::Object(level1));
        let obj = SpookyValue::Object(root);

        let (buf, fc) = from_spooky(&obj).unwrap();
        let record = SpookyRecord::new(&buf, fc);

        let l1 = record.get_field::<SpookyValue>("l1").unwrap();
        let l2 = l1.get("l2").unwrap();
        let l3 = l2.get("l3").unwrap();
        assert_eq!(l3.get("deep").and_then(|v| v.as_str()), Some("value"));
    }

    #[test]
    fn test_mixed_flat_and_nested() {
        let mut map = FastMap::new();
        map.insert(SmolStr::from("flat_str"), SpookyValue::from("hello"));
        map.insert(SmolStr::from("flat_num"), SpookyValue::from(7i64));
        map.insert(
            SmolStr::from("nested"),
            SpookyValue::Array(vec![SpookyValue::from(1i64), SpookyValue::from(2i64)]),
        );
        map.insert(SmolStr::from("flat_bool"), SpookyValue::from(false));
        let obj = SpookyValue::Object(map);

        let (buf, fc) = from_spooky(&obj).unwrap();
        let record = SpookyRecord::new(&buf, fc);

        assert_eq!(record.get_str("flat_str"), Some("hello"));
        assert_eq!(record.get_i64("flat_num"), Some(7));
        assert_eq!(record.get_bool("flat_bool"), Some(false));
        let arr = record.get_field::<SpookyValue>("nested").unwrap();
        assert_eq!(arr.as_array().unwrap().len(), 2);
        // Typed getters must not return nested CBOR
        assert!(record.get_str("nested").is_none());
        assert!(record.get_i64("nested").is_none());
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Null field
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn test_null_field() {
        let mut map = FastMap::new();
        map.insert(SmolStr::from("nothing"), SpookyValue::Null);
        let obj = SpookyValue::Object(map);

        let (buf, fc) = from_spooky(&obj).unwrap();
        let record = SpookyRecord::new(&buf, fc);
        assert_eq!(record.get_field::<SpookyValue>("nothing"), Some(SpookyValue::Null));
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Serialization: from_spooky rejects non-objects
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn test_not_an_object_string() {
        assert!(from_spooky(&SpookyValue::from("not an object")).is_err());
    }

    #[test]
    fn test_not_an_object_number() {
        assert!(from_spooky(&SpookyValue::from(42i64)).is_err());
    }

    #[test]
    fn test_not_an_object_array() {
        assert!(from_spooky(&SpookyValue::Array(vec![])).is_err());
    }

    #[test]
    fn test_not_an_object_null() {
        assert!(from_spooky(&SpookyValue::Null).is_err());
    }

    #[test]
    fn test_not_an_object_bool() {
        assert!(from_spooky(&SpookyValue::from(true)).is_err());
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Iterator (FieldIter)
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn test_iter_fields_count() {
        let original = make_test_record();
        let (buf, fc) = from_spooky(&original).unwrap();
        let record = SpookyRecord::new(&buf, fc);

        let fields: Vec<_> = record.iter_fields().collect();
        assert_eq!(fields.len(), 6);
    }

    #[test]
    fn test_iter_fields_exact_size() {
        let original = make_test_record();
        let (buf, fc) = from_spooky(&original).unwrap();
        let record = SpookyRecord::new(&buf, fc);

        let iter = record.iter_fields();
        assert_eq!(iter.len(), 6);
    }

    #[test]
    fn test_iter_fields_size_hint_decrements() {
        let original = make_test_record();
        let (buf, fc) = from_spooky(&original).unwrap();
        let record = SpookyRecord::new(&buf, fc);

        let mut iter = record.iter_fields();
        assert_eq!(iter.size_hint(), (6, Some(6)));
        iter.next();
        assert_eq!(iter.size_hint(), (5, Some(5)));
    }

    #[test]
    fn test_iter_fields_empty_record() {
        let obj = SpookyValue::Object(FastMap::new());
        let (buf, fc) = from_spooky(&obj).unwrap();
        let record = SpookyRecord::new(&buf, fc);

        assert_eq!(record.iter_fields().count(), 0);
        assert_eq!(record.iter_fields().len(), 0);
    }

    #[test]
    fn test_iter_fields_type_tags_present() {
        let original = make_test_record();
        let (buf, fc) = from_spooky(&original).unwrap();
        let record = SpookyRecord::new(&buf, fc);

        let tags: Vec<u8> = record.iter_fields().map(|f| f.type_tag).collect();
        // We should see all our type tags (order is sorted by hash, so just check membership)
        assert!(tags.contains(&TAG_STR));
        assert!(tags.contains(&TAG_I64));
        assert!(tags.contains(&TAG_F64));
        assert!(tags.contains(&TAG_BOOL));
        assert!(tags.contains(&TAG_U64));
    }

    // ═══════════════════════════════════════════════════════════════════════
    // read_index — bounds checking
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn test_read_index_valid() {
        let original = make_test_record();
        let (buf, fc) = from_spooky(&original).unwrap();
        let record = SpookyRecord::new(&buf, fc);

        for i in 0..fc {
            assert!(record.read_index(i).is_some());
        }
    }

    #[test]
    fn test_read_index_out_of_bounds() {
        let original = make_test_record();
        let (buf, fc) = from_spooky(&original).unwrap();
        let record = SpookyRecord::new(&buf, fc);

        assert!(record.read_index(fc).is_none());
        assert!(record.read_index(fc + 1).is_none());
        assert!(record.read_index(usize::MAX).is_none());
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Linear search path (≤ 4 fields)
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn test_linear_search_path() {
        let original = make_linear_record();
        let (buf, fc) = from_spooky(&original).unwrap();
        assert!(fc <= 4, "should use linear search for ≤ 4 fields");
        let record = SpookyRecord::new(&buf, fc);

        assert_eq!(record.get_str("a"), Some("alpha"));
        assert_eq!(record.get_i64("b"), Some(1));
        assert_eq!(record.get_f64("c"), Some(2.0));
        assert_eq!(record.get_bool("d"), Some(true));
        assert!(!record.has_field("e"));
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Binary search path (> 4 fields)
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn test_binary_search_path() {
        let original = make_test_record();
        let (buf, fc) = from_spooky(&original).unwrap();
        assert!(fc > 4, "should use binary search for > 4 fields");
        let record = SpookyRecord::new(&buf, fc);

        // Verify every field is still found by binary search
        assert_eq!(record.get_str("id"), Some("user:123"));
        assert_eq!(record.get_str("name"), Some("Alice"));
        assert_eq!(record.get_i64("age"), Some(30));
        assert_eq!(record.get_f64("score"), Some(99.5));
        assert_eq!(record.get_bool("active"), Some(true));
        assert_eq!(record.get_u64("version"), Some(42));
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Edge-case numeric values
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn test_i64_boundaries() {
        for val in [i64::MIN, -1, 0, 1, i64::MAX] {
            let obj = make_single_field("n", SpookyValue::from(val));
            let (buf, fc) = from_spooky(&obj).unwrap();
            let record = SpookyRecord::new(&buf, fc);
            assert_eq!(record.get_i64("n"), Some(val), "failed for i64 = {val}");
        }
    }

    #[test]
    fn test_u64_boundaries() {
        for val in [0u64, 1, u64::MAX] {
            let obj = make_single_field("n", SpookyValue::from(val));
            let (buf, fc) = from_spooky(&obj).unwrap();
            let record = SpookyRecord::new(&buf, fc);
            assert_eq!(record.get_u64("n"), Some(val), "failed for u64 = {val}");
        }
    }

    #[test]
    fn test_f64_special_values() {
        for val in [
            0.0f64,
            -0.0,
            f64::MIN,
            f64::MAX,
            f64::INFINITY,
            f64::NEG_INFINITY,
        ] {
            let obj = make_single_field("n", SpookyValue::from(val));
            let (buf, fc) = from_spooky(&obj).unwrap();
            let record = SpookyRecord::new(&buf, fc);
            assert_eq!(record.get_f64("n"), Some(val), "failed for f64 = {val}");
        }
    }

    #[test]
    fn test_f64_nan_roundtrip() {
        let obj = make_single_field("n", SpookyValue::from(f64::NAN));
        let (buf, fc) = from_spooky(&obj).unwrap();
        let record = SpookyRecord::new(&buf, fc);
        assert!(record.get_f64("n").unwrap().is_nan());
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Edge-case strings
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn test_empty_string() {
        let obj = make_single_field("s", SpookyValue::from(""));
        let (buf, fc) = from_spooky(&obj).unwrap();
        let record = SpookyRecord::new(&buf, fc);
        assert_eq!(record.get_str("s"), Some(""));
    }

    #[test]
    fn test_unicode_string() {
        let obj = make_single_field("s", SpookyValue::from("Héllo 🌍 日本語"));
        let (buf, fc) = from_spooky(&obj).unwrap();
        let record = SpookyRecord::new(&buf, fc);
        assert_eq!(record.get_str("s"), Some("Héllo 🌍 日本語"));
    }

    #[test]
    fn test_long_string() {
        let long = "x".repeat(10_000);
        let obj = make_single_field("s", SpookyValue::from(long.as_str()));
        let (buf, fc) = from_spooky(&obj).unwrap();
        let record = SpookyRecord::new(&buf, fc);
        assert_eq!(record.get_str("s"), Some(long.as_str()));
    }

    // ═══════════════════════════════════════════════════════════════════════
    // from_bytes validation
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn test_from_bytes_valid() {
        let original = make_test_record();
        let (buf, fc) = from_spooky(&original).unwrap();
        let (parsed_buf, parsed_fc) = from_bytes(&buf).unwrap();
        assert_eq!(parsed_fc, fc);
        assert_eq!(parsed_buf.len(), buf.len());
    }

    #[test]
    fn test_from_bytes_too_short() {
        let short = vec![0u8; 10]; // less than HEADER_SIZE (20)
        assert!(from_bytes(&short).is_err());
    }

    #[test]
    fn test_from_bytes_empty() {
        assert!(from_bytes(&[]).is_err());
    }

    #[test]
    fn test_from_bytes_header_only_zero_fields() {
        // 20 bytes, field_count = 0 → valid
        let mut buf = vec![0u8; HEADER_SIZE];
        buf[0..4].copy_from_slice(&0u32.to_le_bytes());
        let (_, fc) = from_bytes(&buf).unwrap();
        assert_eq!(fc, 0);
    }

    #[test]
    fn test_from_bytes_claims_fields_but_too_short() {
        // Header says 5 fields but buffer is only header-sized
        let mut buf = vec![0u8; HEADER_SIZE];
        buf[0..4].copy_from_slice(&5u32.to_le_bytes());
        assert!(from_bytes(&buf).is_err());
    }

    // ═══════════════════════════════════════════════════════════════════════
    // serialize_into (reusable buffer path)
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn test_serialize_into_roundtrip() {
        let mut map = FastMap::new();
        map.insert(SmolStr::from("x"), SpookyValue::from(10i64));
        map.insert(SmolStr::from("y"), SpookyValue::from("hi"));
        let mut buf = Vec::new();
        let fc = serialize_into(&map, &mut buf).unwrap();
        let record = SpookyRecord::new(&buf, fc);

        assert_eq!(record.get_i64("x"), Some(10));
        assert_eq!(record.get_str("y"), Some("hi"));
    }

    #[test]
    fn test_serialize_into_reuses_buffer() {
        let mut map = FastMap::new();
        map.insert(SmolStr::from("a"), SpookyValue::from(1i64));
        let mut buf = Vec::new();

        // First serialization
        let fc1 = serialize_into(&map, &mut buf).unwrap();
        let r1 = SpookyRecord::new(&buf, fc1);
        assert_eq!(r1.get_i64("a"), Some(1));

        // Second serialization into the *same* buffer
        map.clear();
        map.insert(SmolStr::from("b"), SpookyValue::from(2i64));
        let fc2 = serialize_into(&map, &mut buf).unwrap();
        let r2 = SpookyRecord::new(&buf, fc2);
        assert_eq!(r2.get_i64("b"), Some(2));
        assert!(r2.get_i64("a").is_none()); // old field gone
    }

    // ═══════════════════════════════════════════════════════════════════════
    // to_value (current placeholder behaviour)
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn test_to_value_returns_null_placeholder() {
        let original = make_test_record();
        let (buf, fc) = from_spooky(&original).unwrap();
        let record = SpookyRecord::new(&buf, fc);

        // Current implementation is a placeholder
        assert_eq!(record.to_value(), SpookyValue::Null);
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Multiple records from the same original check independence
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn test_two_records_independent() {
        let obj1 = make_single_field("a", SpookyValue::from(1i64));
        let obj2 = make_single_field("b", SpookyValue::from(2i64));
        let (buf1, fc1) = from_spooky(&obj1).unwrap();
        let (buf2, fc2) = from_spooky(&obj2).unwrap();
        let r1 = SpookyRecord::new(&buf1, fc1);
        let r2 = SpookyRecord::new(&buf2, fc2);

        assert_eq!(r1.get_i64("a"), Some(1));
        assert!(r1.get_i64("b").is_none());
        assert_eq!(r2.get_i64("b"), Some(2));
        assert!(r2.get_i64("a").is_none());
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Many fields (stress binary search)
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn test_many_fields() {
        let mut map = FastMap::new();
        for i in 0..30 {
            map.insert(
                SmolStr::from(format!("field_{i}")),
                SpookyValue::from(i as i64),
            );
        }
        let obj = SpookyValue::Object(map);
        let (buf, fc) = from_spooky(&obj).unwrap();
        let record = SpookyRecord::new(&buf, fc);

        assert_eq!(record.field_count(), 30);
        for i in 0..30 {
            assert_eq!(
                record.get_i64(&format!("field_{i}")),
                Some(i as i64),
                "field_{i} not found"
            );
        }
    }
}
// ─── Spooky Record Mut Tests ──────────────────────────────────────────────────────────────────
#[cfg(test)]
mod spooky_record_mut_tests {
    use crate::error::RecordError;
    use crate::serialization::{from_bytes, from_spooky, serialize_into};
    use crate::spooky_record::SpookyRecord;
    use crate::spooky_record::read_op::SpookyReadable;
    use crate::spooky_record::record_mut::SpookyRecordMut;
    use crate::spooky_value::FastMap;
    use crate::spooky_value::SpookyValue;
    use crate::types::*;
    use smol_str::SmolStr;

    fn make_test_value() -> SpookyValue {
        let mut map = FastMap::new();
        map.insert(SmolStr::from("id"), SpookyValue::from("user:123"));
        map.insert(SmolStr::from("name"), SpookyValue::from("Alice"));
        map.insert(SmolStr::from("age"), SpookyValue::from(30i64));
        map.insert(SmolStr::from("score"), SpookyValue::from(99.5f64));
        map.insert(SmolStr::from("active"), SpookyValue::from(true));
        map.insert(SmolStr::from("level"), SpookyValue::from(42u64));
        SpookyValue::Object(map)
    }

    fn make_record_mut() -> SpookyRecordMut {
        let (buf, fc) = from_spooky(&make_test_value()).unwrap();
        SpookyRecordMut::new(buf, fc)
    }

    // ── Construction ────────────────────────────────────────────────────────

    #[test]
    fn test_from_spooky_value_roundtrip() {
        let rec = make_record_mut();
        assert_eq!(rec.field_count(), 6);
        assert_eq!(rec.get_str("id"), Some("user:123"));
        assert_eq!(rec.get_str("name"), Some("Alice"));
        assert_eq!(rec.get_i64("age"), Some(30));
        assert_eq!(rec.get_f64("score"), Some(99.5));
        assert_eq!(rec.get_bool("active"), Some(true));
        assert_eq!(rec.get_u64("level"), Some(42));
    }

    #[test]
    fn test_from_serialize_record() {
        // Verify SpookyRecordMut works with buffers from serialize_record()
        let val = make_test_value();
        let (bytes, fc) = from_spooky(&val).unwrap();
        let rec = SpookyRecordMut::new(bytes, fc);
        assert_eq!(rec.get_str("name"), Some("Alice"));
        assert_eq!(rec.get_i64("age"), Some(30));
    }

    #[test]
    fn test_from_vec_roundtrip() {
        let original = make_record_mut();
        let bytes = original.data_buf.clone();
        let (_, fc) = from_bytes(&bytes).unwrap();
        let restored = SpookyRecordMut::new(bytes, fc);
        assert_eq!(restored.get_str("name"), Some("Alice"));
        assert_eq!(restored.get_i64("age"), Some(30));
    }

    #[test]
    fn test_new_empty() {
        let rec = SpookyRecordMut::new_empty();
        assert_eq!(rec.field_count(), 0);
        assert!(!rec.has_field("anything"));
    }

    // ── Typed setters (in-place) ────────────────────────────────────────────

    #[test]
    fn test_set_i64() {
        let mut rec = make_record_mut();
        assert_eq!(rec.get_i64("age"), Some(30));
        rec.set_i64("age", 31).unwrap();
        assert_eq!(rec.get_i64("age"), Some(31));
        rec.set_i64("age", i64::MAX).unwrap();
        assert_eq!(rec.get_i64("age"), Some(i64::MAX));
        rec.set_i64("age", i64::MIN).unwrap();
        assert_eq!(rec.get_i64("age"), Some(i64::MIN));
    }

    #[test]
    fn test_set_u64() {
        let mut rec = make_record_mut();
        rec.set_u64("level", 99).unwrap();
        assert_eq!(rec.get_u64("level"), Some(99));
        rec.set_u64("level", u64::MAX).unwrap();
        assert_eq!(rec.get_u64("level"), Some(u64::MAX));
    }

    #[test]
    fn test_set_f64() {
        let mut rec = make_record_mut();
        rec.set_f64("score", 100.0).unwrap();
        assert_eq!(rec.get_f64("score"), Some(100.0));
        rec.set_f64("score", f64::NEG_INFINITY).unwrap();
        assert_eq!(rec.get_f64("score"), Some(f64::NEG_INFINITY));
    }

    #[test]
    fn test_set_bool() {
        let mut rec = make_record_mut();
        assert_eq!(rec.get_bool("active"), Some(true));
        rec.set_bool("active", false).unwrap();
        assert_eq!(rec.get_bool("active"), Some(false));
        rec.set_bool("active", true).unwrap();
        assert_eq!(rec.get_bool("active"), Some(true));
    }

    #[test]
    fn test_typed_setter_type_mismatch() {
        let mut rec = make_record_mut();
        assert!(matches!(
            rec.set_u64("age", 5),
            Err(RecordError::TypeMismatch { .. })
        ));
        assert!(matches!(
            rec.set_i64("name", 5),
            Err(RecordError::TypeMismatch { .. })
        ));
    }

    #[test]
    fn test_setter_field_not_found() {
        let mut rec = make_record_mut();
        assert!(matches!(
            rec.set_i64("nope", 5),
            Err(RecordError::FieldNotFound)
        ));
    }

    // ── String setters ──────────────────────────────────────────────────────

    #[test]
    fn test_set_str_same_length() {
        let mut rec = make_record_mut();
        rec.set_str("name", "Bobby").unwrap(); // 5 → 5 bytes
        assert_eq!(rec.get_str("name"), Some("Bobby"));
        assert_eq!(rec.get_i64("age"), Some(30));
        assert_eq!(rec.get_str("id"), Some("user:123"));
    }

    #[test]
    fn test_set_str_grow() {
        let mut rec = make_record_mut();
        let old_len = rec.data_buf.len();
        rec.set_str("name", "Alexander").unwrap(); // 5 → 9 bytes
        assert_eq!(rec.get_str("name"), Some("Alexander"));
        assert_eq!(rec.data_buf.len(), old_len + 4);

        // All other fields intact
        assert_eq!(rec.get_str("id"), Some("user:123"));
        assert_eq!(rec.get_i64("age"), Some(30));
        assert_eq!(rec.get_f64("score"), Some(99.5));
        assert_eq!(rec.get_bool("active"), Some(true));
        assert_eq!(rec.get_u64("level"), Some(42));
    }

    #[test]
    fn test_set_str_shrink() {
        let mut rec = make_record_mut();
        let old_len = rec.data_buf.len();
        rec.set_str("name", "Al").unwrap(); // 5 → 2 bytes
        assert_eq!(rec.get_str("name"), Some("Al"));
        assert_eq!(rec.data_buf.len(), old_len - 3);

        assert_eq!(rec.get_str("id"), Some("user:123"));
        assert_eq!(rec.get_i64("age"), Some(30));
        assert_eq!(rec.get_f64("score"), Some(99.5));
    }

    #[test]
    fn test_set_str_exact() {
        let mut rec = make_record_mut();
        rec.set_str_exact("name", "Bobby").unwrap();
        assert_eq!(rec.get_str("name"), Some("Bobby"));
        assert!(matches!(
            rec.set_str_exact("name", "Al"),
            Err(RecordError::LengthMismatch { .. })
        ));
    }

    // ── Generic setter ──────────────────────────────────────────────────────

    #[test]
    fn test_set_field_same_type_same_size() {
        let mut rec = make_record_mut();
        rec.set_field("age", &SpookyValue::from(99i64)).unwrap();
        assert_eq!(rec.get_i64("age"), Some(99));
    }

    #[test]
    fn test_add_field_resize() {
        let mut rec = make_record_mut();
        // Add a field that forces buffer growth/move
        let long_str = "x".repeat(100);
        rec.add_field("description", &SpookyValue::from(long_str.as_str()))
            .unwrap();

        assert_eq!(rec.get_str("description"), Some(long_str.as_str()));
        // Verify old fields still work
        assert_eq!(rec.get_str("name"), Some("Alice"));
    }

    #[test]
    fn test_parity_methods() {
        let rec = make_record_mut();
        assert!(rec.get_raw("age").is_some());
        assert!(rec.get_raw("missing").is_none());

        assert_eq!(rec.get_number_as_f64("age"), Some(30.0));
        assert_eq!(rec.get_number_as_f64("score"), Some(99.5));
        assert_eq!(rec.get_number_as_f64("name"), None);

        let fields: Vec<_> = rec.iter_fields().collect();
        assert_eq!(fields.len(), 6);
        assert_eq!(rec.get_f64("score"), Some(99.5));
    }

    #[test]
    fn test_set_field_type_change() {
        let mut rec = make_record_mut();
        rec.set_field("age", &SpookyValue::from("thirty")).unwrap();
        assert_eq!(rec.get_str("age"), Some("thirty"));
        assert_eq!(rec.get_i64("age"), None);
        assert_eq!(rec.get_str("name"), Some("Alice"));
        assert_eq!(rec.get_f64("score"), Some(99.5));
    }

    #[test]
    fn test_set_field_to_null() {
        let mut rec = make_record_mut();
        rec.set_null("name").unwrap();
        assert_eq!(rec.get_str("name"), None);
        assert_eq!(rec.field_type("name"), Some(TAG_NULL));
        assert_eq!(rec.get_i64("age"), Some(30));
    }

    #[test]
    fn test_set_field_nested_object() {
        let mut rec = make_record_mut();
        let mut inner = FastMap::new();
        inner.insert(SmolStr::from("city"), SpookyValue::from("Berlin"));
        let obj = SpookyValue::Object(inner);

        rec.set_field("name", &obj).unwrap();
        let result = rec.get_field::<SpookyValue>("name").unwrap();
        assert_eq!(result.get("city").and_then(|v| v.as_str()), Some("Berlin"));
        assert_eq!(rec.get_i64("age"), Some(30));
    }

    // ── add_field ───────────────────────────────────────────────────────────

    #[test]
    fn test_add_field() {
        let mut rec = make_record_mut();
        assert_eq!(rec.field_count(), 6);
        rec.add_field("email", &SpookyValue::from("alice@example.com"))
            .unwrap();

        assert_eq!(rec.field_count(), 7);
        assert_eq!(rec.get_str("email"), Some("alice@example.com"));

        // All original fields intact
        assert_eq!(rec.get_str("id"), Some("user:123"));
        assert_eq!(rec.get_str("name"), Some("Alice"));
        assert_eq!(rec.get_i64("age"), Some(30));
        assert_eq!(rec.get_f64("score"), Some(99.5));
        assert_eq!(rec.get_bool("active"), Some(true));
        assert_eq!(rec.get_u64("level"), Some(42));
    }

    #[test]
    fn test_add_field_duplicate() {
        let mut rec = make_record_mut();
        assert!(matches!(
            rec.add_field("name", &SpookyValue::from("Bob")),
            Err(RecordError::FieldExists)
        ));
    }

    #[test]
    fn test_add_multiple_fields() {
        let mut rec = make_record_mut();
        rec.add_field("email", &SpookyValue::from("alice@test.com"))
            .unwrap();
        rec.add_field("country", &SpookyValue::from("DE")).unwrap();
        rec.add_field("verified", &SpookyValue::from(true)).unwrap();

        assert_eq!(rec.field_count(), 9);
        assert_eq!(rec.get_str("email"), Some("alice@test.com"));
        assert_eq!(rec.get_str("country"), Some("DE"));
        assert_eq!(rec.get_bool("verified"), Some(true));
        assert_eq!(rec.get_str("name"), Some("Alice"));
    }

    // ── remove_field ────────────────────────────────────────────────────────

    #[test]
    fn test_remove_field() {
        let mut rec = make_record_mut();
        rec.remove_field("name").unwrap();

        assert_eq!(rec.field_count(), 5);
        assert!(!rec.has_field("name"));
        assert_eq!(rec.get_str("id"), Some("user:123"));
        assert_eq!(rec.get_i64("age"), Some(30));
        assert_eq!(rec.get_f64("score"), Some(99.5));
        assert_eq!(rec.get_bool("active"), Some(true));
        assert_eq!(rec.get_u64("level"), Some(42));
    }

    #[test]
    fn test_remove_nonexistent() {
        let mut rec = make_record_mut();
        assert!(matches!(
            rec.remove_field("nope"),
            Err(RecordError::FieldNotFound)
        ));
    }

    #[test]
    fn test_remove_then_add() {
        let mut rec = make_record_mut();
        rec.remove_field("name").unwrap();
        rec.add_field("name", &SpookyValue::from("Bob")).unwrap();
        assert_eq!(rec.get_str("name"), Some("Bob"));
        assert_eq!(rec.field_count(), 6);
    }

    #[test]
    fn test_remove_all_fields() {
        let mut rec = make_record_mut();
        for f in &["id", "name", "age", "score", "active", "level"] {
            rec.remove_field(f).unwrap();
        }
        assert_eq!(rec.field_count(), 0);
        assert_eq!(rec.data_buf.len(), HEADER_SIZE);
    }

    // ── Multiple mutations ──────────────────────────────────────────────────

    #[test]
    fn test_multiple_mutations_sequence() {
        let mut rec = make_record_mut();
        rec.set_i64("age", 31).unwrap();
        rec.set_f64("score", 100.5).unwrap();
        rec.set_bool("active", false).unwrap();
        rec.set_str("name", "Bob").unwrap();
        rec.set_u64("level", 43).unwrap();

        assert_eq!(rec.get_i64("age"), Some(31));
        assert_eq!(rec.get_f64("score"), Some(100.5));
        assert_eq!(rec.get_bool("active"), Some(false));
        assert_eq!(rec.get_str("name"), Some("Bob"));
        assert_eq!(rec.get_u64("level"), Some(43));
        assert_eq!(rec.get_str("id"), Some("user:123"));
    }

    #[test]
    fn test_rapid_fire_same_field() {
        let mut rec = make_record_mut();
        for i in 0..1000 {
            rec.set_i64("age", i).unwrap();
        }
        assert_eq!(rec.get_i64("age"), Some(999));
        assert_eq!(rec.get_str("name"), Some("Alice"));
    }

    // ── as_record interop ───────────────────────────────────────────────────

    #[test]
    fn test_as_record_interop() {
        let mut rec = make_record_mut();
        rec.set_i64("age", 50).unwrap();
        rec.set_str("name", "Charlie").unwrap();

        let reader = rec.as_record();
        assert_eq!(reader.get_i64("age"), Some(50));
        assert_eq!(reader.get_str("name"), Some("Charlie"));
        assert_eq!(reader.field_count(), 6);
    }

    // ── Persist + restore ───────────────────────────────────────────────────

    #[test]
    fn test_mutate_persist_restore() {
        let mut rec = make_record_mut();
        rec.set_i64("age", 99).unwrap();
        rec.set_str("name", "Modified").unwrap();
        rec.add_field("new_field", &SpookyValue::from(42i64))
            .unwrap();

        let bytes = rec.data_buf.clone();
        let (_, fc) = from_bytes(&bytes).unwrap();
        let restored = SpookyRecordMut::new(bytes, fc);

        assert_eq!(restored.get_i64("age"), Some(99));
        assert_eq!(restored.get_str("name"), Some("Modified"));
        assert_eq!(restored.get_i64("new_field"), Some(42));
        assert_eq!(restored.field_count(), 7);
    }

    // ── Edge cases ──────────────────────────────────────────────────────────

    #[test]
    fn test_empty_string_field() {
        let mut rec = make_record_mut();
        rec.set_str("name", "").unwrap();
        assert_eq!(rec.get_str("name"), Some(""));
        rec.set_str("name", "back").unwrap();
        assert_eq!(rec.get_str("name"), Some("back"));
    }

    #[test]
    fn test_add_field_to_empty_record() {
        let mut rec = SpookyRecordMut::new_empty();
        rec.add_field("first", &SpookyValue::from("hello")).unwrap();
        assert_eq!(rec.field_count(), 1);
        assert_eq!(rec.get_str("first"), Some("hello"));

        rec.add_field("second", &SpookyValue::from(42i64)).unwrap();
        assert_eq!(rec.field_count(), 2);
        assert_eq!(rec.get_i64("second"), Some(42));
    }

    #[test]
    fn test_unicode_strings() {
        let mut rec = make_record_mut();
        rec.set_str("name", "Ünïcödé 🎃").unwrap();
        assert_eq!(rec.get_str("name"), Some("Ünïcödé 🎃"));
        assert_eq!(rec.get_i64("age"), Some(30));
    }

    #[test]
    fn test_large_string_growth() {
        let mut rec = make_record_mut();
        let large = "x".repeat(10_000);
        rec.set_str("name", &large).unwrap();
        assert_eq!(rec.get_str("name"), Some(large.as_str()));
        assert_eq!(rec.get_i64("age"), Some(30));
    }

    #[test]
    fn test_multiple_splices_accumulate_correctly() {
        let mut rec = make_record_mut();
        // Grow, shrink, grow, shrink — stress test offset fixups
        rec.set_str("name", "A very long name indeed").unwrap();
        assert_eq!(rec.get_str("name"), Some("A very long name indeed"));
        assert_eq!(rec.get_i64("age"), Some(30));

        rec.set_str("name", "X").unwrap();
        assert_eq!(rec.get_str("name"), Some("X"));
        assert_eq!(rec.get_i64("age"), Some(30));

        rec.set_str("id", "user:999999999").unwrap();
        assert_eq!(rec.get_str("id"), Some("user:999999999"));
        assert_eq!(rec.get_str("name"), Some("X"));
        assert_eq!(rec.get_i64("age"), Some(30));

        rec.set_str("id", "u").unwrap();
        assert_eq!(rec.get_str("id"), Some("u"));
        assert_eq!(rec.get_str("name"), Some("X"));
        assert_eq!(rec.get_i64("age"), Some(30));
        assert_eq!(rec.get_f64("score"), Some(99.5));
        assert_eq!(rec.get_bool("active"), Some(true));
        assert_eq!(rec.get_u64("level"), Some(42));
    }

    #[test]
    fn test_add_then_mutate() {
        let mut rec = make_record_mut();
        rec.add_field("email", &SpookyValue::from("old@test.com"))
            .unwrap();
        rec.set_str("email", "new@test.com").unwrap();
        assert_eq!(rec.get_str("email"), Some("new@test.com"));

        rec.set_str("email", "x@y.z").unwrap(); // shrink
        assert_eq!(rec.get_str("email"), Some("x@y.z"));

        // Original fields still fine
        assert_eq!(rec.get_str("name"), Some("Alice"));
    }

    #[test]
    fn test_null_field_add_and_read() {
        let mut rec = make_record_mut();
        rec.add_field("nothing", &SpookyValue::Null).unwrap();
        assert_eq!(rec.field_type("nothing"), Some(TAG_NULL));
        assert_eq!(rec.get_field::<SpookyValue>("nothing"), Some(SpookyValue::Null));
    }

    // ── Phase 1: FieldSlot Tests ───────────────────────────────────────────

    #[test]
    fn test_resolve_and_get_at() {
        let rec = make_record_mut();

        // Resolve all fields
        let id_slot = rec.resolve("id").expect("id exists");
        let age_slot = rec.resolve("age").expect("age exists");
        let score_slot = rec.resolve("score").expect("score exists");
        let active_slot = rec.resolve("active").expect("active exists");
        let level_slot = rec.resolve("level").expect("level exists");

        // Read via slots - should match by-name accessors
        assert_eq!(rec.get_str_at(&id_slot), Some("user:123"));
        assert_eq!(rec.get_i64_at(&age_slot), Some(30));
        assert_eq!(rec.get_f64_at(&score_slot), Some(99.5));
        assert_eq!(rec.get_bool_at(&active_slot), Some(true));
        assert_eq!(rec.get_u64_at(&level_slot), Some(42));
    }

    #[test]
    fn test_set_at_fixed_width() {
        let mut rec = make_record_mut();

        let age_slot = rec.resolve("age").unwrap();
        let score_slot = rec.resolve("score").unwrap();
        let active_slot = rec.resolve("active").unwrap();
        let level_slot = rec.resolve("level").unwrap();

        // Mutate via slots
        rec.set_i64_at(&age_slot, 31).unwrap();
        rec.set_f64_at(&score_slot, 100.0).unwrap();
        rec.set_bool_at(&active_slot, false).unwrap();
        rec.set_u64_at(&level_slot, 43).unwrap();

        // Read back
        assert_eq!(rec.get_i64_at(&age_slot), Some(31));
        assert_eq!(rec.get_f64_at(&score_slot), Some(100.0));
        assert_eq!(rec.get_bool_at(&active_slot), Some(false));
        assert_eq!(rec.get_u64_at(&level_slot), Some(43));

        // Slots should still be valid (generation didn't change)
        assert_eq!(rec.get_i64("age"), Some(31));
    }

    #[test]
    fn test_set_str_at_same_length() {
        let mut rec = make_record_mut();
        let name_slot = rec.resolve("name").unwrap();

        // "Alice" is 5 bytes, "Carol" is also 5 bytes
        rec.set_str_at(&name_slot, "Carol").unwrap();
        assert_eq!(rec.get_str_at(&name_slot), Some("Carol"));

        // Slot still valid
        assert_eq!(rec.get_str("name"), Some("Carol"));
    }

    #[test]
    fn test_set_str_at_length_mismatch() {
        let mut rec = make_record_mut();
        let name_slot = rec.resolve("name").unwrap();

        // "Alice" is 5 bytes, "Bob" is 3 bytes
        let result = rec.set_str_at(&name_slot, "Bob");
        assert!(matches!(result, Err(RecordError::LengthMismatch { .. })));
    }

    #[test]
    fn test_generation_bump_on_splice() {
        let mut rec = make_record_mut();
        let old_gen = rec.generation;

        // Resolve slot
        let name_slot = rec.resolve("name").unwrap();
        assert_eq!(name_slot.generation, old_gen);

        // Splice triggers generation bump (different length)
        rec.set_str("name", "Alexander").unwrap();
        assert_eq!(rec.generation, old_gen + 1);

        // Re-resolve to get fresh slot
        let new_slot = rec.resolve("name").unwrap();
        assert_eq!(new_slot.generation, old_gen + 1);
        assert_eq!(rec.get_str_at(&new_slot), Some("Alexander"));
    }

    #[test]
    fn test_generation_bump_on_add_remove() {
        let mut rec = make_record_mut();
        let old_gen = rec.generation;

        let age_slot = rec.resolve("age").unwrap();
        assert_eq!(age_slot.generation, old_gen);

        // add_field bumps generation
        rec.add_field("email", &SpookyValue::from("test@example.com"))
            .unwrap();
        assert_eq!(rec.generation, old_gen + 1);

        // remove_field bumps again
        rec.remove_field("email").unwrap();
        assert_eq!(rec.generation, old_gen + 2);
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "stale FieldSlot")]
    fn test_stale_slot_debug_panic() {
        let mut rec = make_record_mut();
        let age_slot = rec.resolve("age").unwrap();

        // Invalidate the slot by adding a field
        rec.add_field("email", &SpookyValue::from("test@test.com"))
            .unwrap();

        // This should panic in debug mode
        let _ = rec.get_i64_at(&age_slot);
    }

    #[test]
    fn test_resolve_missing_field() {
        let rec = make_record_mut();
        assert!(rec.resolve("nonexistent").is_none());
    }

    // ── Phase 2: Buffer Reuse Tests ────────────────────────────────────────

    #[test]
    fn test_serialize_into_roundtrip() {
        let value = make_test_value();

        let (buf, fc) = from_spooky(&value).unwrap();

        let rec = SpookyRecord::new(&buf, fc);
        assert_eq!(rec.field_count(), 6);
        assert_eq!(rec.get_str("id"), Some("user:123"));
        assert_eq!(rec.get_str("name"), Some("Alice"));
        assert_eq!(rec.get_i64("age"), Some(30));
    }

    #[test]
    fn test_serialize_into_reuse() {
        let value_a = make_test_value();
        let mut map_b = FastMap::new();
        map_b.insert(SmolStr::from("x"), SpookyValue::from(100i64));
        map_b.insert(SmolStr::from("y"), SpookyValue::from(200i64));
        let value_b = SpookyValue::Object(map_b);

        let map_a = match value_a {
            SpookyValue::Object(ref m) => m,
            _ => panic!("Expected object"),
        };
        let map_b = match value_b {
            SpookyValue::Object(ref m) => m,
            _ => panic!("Expected object"),
        };

        let mut buf = Vec::new();

        // Serialize record A
        serialize_into(map_a, &mut buf).unwrap();
        let cap_after_a = buf.capacity();

        // Serialize record B into same buffer
        serialize_into(map_b, &mut buf).unwrap();

        // Buffer should be reused (capacity shouldn't decrease)
        assert!(buf.capacity() >= cap_after_a);

        // Verify B's data is correct
        let (buf_slice, fc) = from_bytes(&buf).unwrap();
        let rec_b = SpookyRecord::new(buf_slice, fc);
        assert_eq!(rec_b.field_count(), 2);
        assert_eq!(rec_b.get_i64("x"), Some(100));
        assert_eq!(rec_b.get_i64("y"), Some(200));

        // A's data should be gone
        assert_eq!(rec_b.get_str("id"), None);
    }

    #[test]
    fn test_from_spooky_value_into_roundtrip() {
        let value = make_test_value();

        let (buf, fc) = from_spooky(&value).unwrap();
        let rec = SpookyRecordMut::new(buf, fc);

        assert_eq!(rec.field_count(), 6);
        assert_eq!(rec.get_str("id"), Some("user:123"));
        assert_eq!(rec.get_i64("age"), Some(30));
        assert_eq!(rec.get_f64("score"), Some(99.5));
    }

    #[test]
    fn test_from_spooky_value_into_reuse() {
        let value_a = make_test_value();

        let mut map_b = FastMap::new();
        map_b.insert(SmolStr::from("foo"), SpookyValue::from(777i64));

        // Build record A
        let (buf_a, _fc_a) = from_spooky(&value_a).unwrap();
        let cap_after_a = buf_a.capacity();
        
        // Reuse that buffer for record B
        let mut buf_b = buf_a;
        serialize_into(&map_b, &mut buf_b).unwrap();
        let (_, fc_b) = from_bytes(&buf_b).unwrap();
        let rec_b = SpookyRecordMut::new(buf_b, fc_b);

        // Buffer should be reused
        assert!(rec_b.data_buf.len() <= cap_after_a);

        // Verify B is correct
        assert_eq!(rec_b.field_count(), 1);
        assert_eq!(rec_b.get_i64("foo"), Some(777));

        // A's data should be gone
        assert_eq!(rec_b.get_str("id"), None);
    }
}
