//! Differential test: the direct `Sp00kyValue` canonical writer must produce
//! byte-identical output to the `serde_json::Value` pipeline it replaces.
//!
//! This is the highest-consequence invariant in the SSP. The scheduler
//! verifies a rejoining SSP against these hashes and the SSP `exit(2)`s on
//! mismatch, so a divergence here does not surface as a wrong number — it
//! surfaces as a node that boots, fails verification, exits, re-registers, and
//! freezes the tenant's sync on every loop. `apps/scheduler/src/config.rs`
//! records that this exact shape livelocked a real deployment once.
//!
//! So the property under test is stated as an equality against the *old* path,
//! not against a hand-written expectation:
//!
//! ```text
//! sv.record_digest_into(id) == record_digest(id, &Value::from(sv.clone()))
//! ```
//!
//! Note the right-hand side goes through `Value::from(Sp00kyValue)`, which is
//! where the pipeline's quirks live (non-finite floats collapsing to null,
//! `u64 > i64::MAX` having already become a float on the way in). Comparing
//! against the *source* JSON instead would be testing the wrong thing.

use ssp::types::Sp00kyValue;
use ssp_protocol::snapshot_hash::{canonical_json, record_digest};
use serde_json::{json, Value};

/// The equality this whole file exists to defend, checked both at the digest
/// level and at the raw canonical bytes (which localizes a failure).
#[track_caller]
fn assert_equivalent(raw_id: &str, source: Value) {
    let sv = Sp00kyValue::from(source.clone());
    let via_value = Value::from(sv.clone());

    let mut buf = Vec::new();
    sv.write_canonical_record(&mut buf);
    let expected_bytes = canonical_json(&strip_like_protocol(&via_value));
    assert_eq!(
        String::from_utf8_lossy(&buf),
        String::from_utf8_lossy(&expected_bytes),
        "canonical bytes diverged for {source}"
    );

    let mut scratch = Vec::new();
    assert_eq!(
        sv.record_digest_into(raw_id, &mut scratch),
        record_digest(raw_id, &via_value),
        "digest diverged for id={raw_id} value={source}"
    );
}

/// `strip_reserved_keys` is private to ssp-protocol, so mirror it here purely
/// to compare canonical *bytes*. The digest assertion above goes through the
/// real one, so a mistake in this mirror cannot make the test pass wrongly.
fn strip_like_protocol(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(k, v)| !k.starts_with("_00_") && !v.is_null())
                .map(|(k, v)| {
                    if k == "id" {
                        if let Value::String(s) = v {
                            let cleaned: String = s
                                .chars()
                                .filter(|c| *c != '\u{27e8}' && *c != '\u{27e9}' && *c != '`')
                                .collect();
                            return (k.clone(), Value::String(cleaned));
                        }
                    }
                    (k.clone(), v.clone())
                })
                .collect(),
        ),
        other => other.clone(),
    }
}

#[test]
fn plain_row() {
    assert_equivalent(
        "t1",
        json!({ "title": "hello", "score": 42, "active": true, "ratio": 1.5 }),
    );
}

#[test]
fn key_order_is_normalized() {
    // Same content, different insertion order, must hash the same.
    let a = Sp00kyValue::from(json!({ "z": 1, "a": 2, "m": 3 }));
    let b = Sp00kyValue::from(json!({ "m": 3, "z": 1, "a": 2 }));
    let (mut x, mut y) = (Vec::new(), Vec::new());
    a.write_canonical_record(&mut x);
    b.write_canonical_record(&mut y);
    assert_eq!(x, y);
    assert_eq!(String::from_utf8(x).unwrap(), r#"{"a":2,"m":3,"z":1}"#);
}

#[test]
fn reserved_keys_are_stripped_at_top_level_only() {
    assert_equivalent(
        "t1",
        json!({
            "_00_rv": 7,
            "_00_anything": "x",
            "keep": 1,
            // Nested reserved keys survive — the replica's SELECT * only drops
            // undefined *top-level* fields.
            "nested": { "_00_rv": 9, "inner": 2 },
        }),
    );
}

#[test]
fn null_valued_top_level_keys_are_stripped_nested_ones_are_not() {
    assert_equivalent(
        "t1",
        json!({
            "gone": Value::Null,
            "kept": 1,
            "nested": { "inner_null": Value::Null, "x": 2 },
        }),
    );
}

/// The subtle one. `Value::from(f64)` maps non-finite floats to `Value::Null`,
/// and null-valued top-level keys are then stripped — so a field holding NaN
/// or an infinity is absent from the digest entirely. A writer that emitted a
/// number (or even `null`, without also stripping the key) would silently
/// change the hash of every row carrying one.
#[test]
fn non_finite_floats_collapse_to_null_and_are_stripped() {
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let sv = Sp00kyValue::Object(
            [
                ("bad".to_string(), Sp00kyValue::Float(bad)),
                ("good".to_string(), Sp00kyValue::Int(1)),
            ]
            .into_iter()
            .collect(),
        );
        let via_value = Value::from(sv.clone());
        let mut scratch = Vec::new();
        assert_eq!(
            sv.record_digest_into("t1", &mut scratch),
            record_digest("t1", &via_value),
            "digest diverged for non-finite float {bad}"
        );

        let mut buf = Vec::new();
        sv.write_canonical_record(&mut buf);
        assert_eq!(
            String::from_utf8(buf).unwrap(),
            r#"{"good":1}"#,
            "a non-finite float must be stripped, not emitted, for {bad}"
        );
    }
}

/// Nested non-finite floats are NOT stripped (only top-level keys are), so
/// they must be emitted as `null` — matching what the Value tree becomes.
#[test]
fn nested_non_finite_floats_emit_null() {
    let sv = Sp00kyValue::Object(
        [(
            "nested".to_string(),
            Sp00kyValue::Object(
                [("bad".to_string(), Sp00kyValue::Float(f64::NAN))]
                    .into_iter()
                    .collect(),
            ),
        )]
        .into_iter()
        .collect(),
    );
    let mut scratch = Vec::new();
    assert_eq!(
        sv.record_digest_into("t1", &mut scratch),
        record_digest("t1", &Value::from(sv.clone()))
    );
    let mut buf = Vec::new();
    sv.write_canonical_record(&mut buf);
    assert_eq!(String::from_utf8(buf).unwrap(), r#"{"nested":{"bad":null}}"#);
}

#[test]
fn id_escaping_is_normalized() {
    assert_equivalent("⟨_00_app_release⟩:web", json!({ "id": "⟨thread⟩:abc", "x": 1 }));
    assert_equivalent("`quoted`:1", json!({ "id": "`thread`:abc", "x": 1 }));
    // A non-string `id` must pass through untouched.
    assert_equivalent("t1", json!({ "id": 5, "x": 1 }));
}

#[test]
fn numeric_edges() {
    assert_equivalent("t1", json!({ "min": i64::MIN, "max": i64::MAX, "zero": 0 }));
    assert_equivalent("t1", json!({ "int_like": 5, "float_like": 5.0 }));
    assert_equivalent("t1", json!({ "neg_zero": -0.0, "tiny": 5e-324, "big": 1.7976931348623157e308 }));
    // u64 above i64::MAX becomes a Float on the way into Sp00kyValue; the
    // digest is taken after that conversion, so both sides must agree on it.
    assert_equivalent("t1", json!({ "huge": u64::MAX }));
}

#[test]
fn string_escaping() {
    assert_equivalent(
        "t1",
        json!({
            "quotes": "he said \"hi\"",
            "backslash": "a\\b",
            "control": "line\nbreak\ttab\u{0008}\u{000c}\r",
            "unicode": "héllo 世界 🎃",
            "nul": "a\u{0000}b",
            "del": "a\u{007f}b",
        }),
    );
}

/// Escaping must be identical for object *keys*, not just values — keys take a
/// different code path in the writer.
#[test]
fn key_escaping() {
    assert_equivalent(
        "t1",
        json!({
            "with \"quote\"": 1,
            "with\\backslash": 2,
            "with\nnewline": 3,
            "with 🎃 emoji": 4,
            "": 5,
        }),
    );
}

#[test]
fn arrays_and_empty_containers() {
    assert_equivalent("t1", json!({ "empty_obj": {}, "empty_arr": [], "empty_str": "" }));
    assert_equivalent("t1", json!({ "arr": [1, "two", true, Value::Null, 3.5, [], {}] }));
    // Arrays preserve order; nulls inside an array are NOT stripped.
    assert_equivalent("t1", json!({ "arr": [Value::Null, 1, Value::Null] }));
    assert_equivalent(
        "t1",
        json!({ "deep": [[[{ "a": [{ "b": 1 }] }]]] }),
    );
}

#[test]
fn non_object_records_pass_through() {
    assert_equivalent("t1", json!(5));
    assert_equivalent("t1", json!("a string"));
    assert_equivalent("t1", json!(Value::Null));
    assert_equivalent("t1", json!([1, 2, 3]));
    assert_equivalent("t1", json!(true));
}

/// A row where every key is stripped must still produce a well-formed empty
/// object, not an empty byte string.
#[test]
fn fully_stripped_row_is_an_empty_object() {
    let sv = Sp00kyValue::from(json!({ "_00_rv": 1, "gone": Value::Null }));
    let mut buf = Vec::new();
    sv.write_canonical_record(&mut buf);
    assert_eq!(String::from_utf8(buf).unwrap(), "{}");
    let mut scratch = Vec::new();
    assert_eq!(
        sv.record_digest_into("t1", &mut scratch),
        record_digest("t1", &Value::from(sv))
    );
}

/// End-to-end: the accumulator maintained incrementally through
/// `apply_mutation` must equal one computed from scratch over the final rows
/// via the original `serde_json::Value` path.
///
/// The per-mutation digests and the re-seed now share the same writer, so a
/// test comparing only those two would pass even if the writer were wrong.
/// This one pins both against `xor_table_hash`, which still goes through
/// `Value`.
#[test]
fn incremental_accumulator_matches_the_value_path() {
    use ssp::circuit::store::{Change, Store};

    let mut store = Store::new();
    store.ensure_collection("thread");

    let row = |i: usize, title: &str| {
        json!({
            "title": title,
            "score": i as i64,
            "_00_rv": i as i64,
            "archived_at": Value::Null,
            "meta": { "b": 2, "a": [1, Value::Null] },
            "id": format!("⟨thread⟩:t{i}"),
        })
    };

    for i in 0..12 {
        store.apply_change(&Change::create("thread", &format!("t{i}"), row(i, "first")));
    }
    // Updates replace content (prior XORed out, new XORed in).
    for i in 0..6 {
        store.apply_change(&Change::update("thread", &format!("t{i}"), row(i, "second")));
    }
    // Deletes remove it again.
    for i in 6..9 {
        store.apply_change(&Change::delete("thread", &format!("t{i}")));
    }
    // Re-creating a deleted id must land back in the accumulator.
    store.apply_change(&Change::create("thread", "t7", row(7, "third")));

    let coll = store.get_collection("thread").unwrap();
    let expected = ssp_protocol::snapshot_hash::xor_table_hash(
        coll.rows
            .iter()
            .map(|(id, v)| (id.clone(), Value::from(v.clone()))),
    );
    assert_eq!(
        ssp_protocol::snapshot_hash::xor_acc_to_hex(&coll.catchup_xor),
        expected,
        "incrementally maintained accumulator drifted from the Value path"
    );

    // And re-seeding must land on the same value.
    let mut reseeded = store;
    reseeded
        .collections
        .get_mut("thread")
        .unwrap()
        .reseed_catchup_xor();
    assert_eq!(
        ssp_protocol::snapshot_hash::xor_acc_to_hex(
            &reseeded.get_collection("thread").unwrap().catchup_xor
        ),
        expected,
        "re-seed diverged from the incremental accumulator"
    );
}

/// Deleting every row must return the accumulator to its empty state — the
/// property that makes XOR usable as a set hash at all.
#[test]
fn accumulator_returns_to_empty_after_deleting_everything() {
    use ssp::circuit::store::{Change, Store};

    let mut store = Store::new();
    store.ensure_collection("thread");
    for i in 0..8 {
        store.apply_change(&Change::create(
            "thread",
            &format!("t{i}"),
            json!({ "n": i, "_00_rv": i }),
        ));
    }
    assert_ne!(
        store.get_collection("thread").unwrap().catchup_xor,
        ssp_protocol::snapshot_hash::xor_empty()
    );
    for i in 0..8 {
        store.apply_change(&Change::delete("thread", &format!("t{i}")));
    }
    assert_eq!(
        store.get_collection("thread").unwrap().catchup_xor,
        ssp_protocol::snapshot_hash::xor_empty(),
        "accumulator must cancel exactly"
    );
}

/// Randomized sweep over generated shapes — catches combinations the
/// hand-written cases above do not enumerate. Deterministic seed so a failure
/// is reproducible.
#[test]
fn randomized_corpus_matches() {
    let mut rng: u64 = 0x5000_0000_0000_0001;
    let mut next = move || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        rng
    };

    fn gen(next: &mut impl FnMut() -> u64, depth: u32) -> Value {
        let keys = ["a", "z", "_00_rv", "id", "n", "with \"q\"", "é", ""];
        match next() % if depth >= 3 { 7 } else { 9 } {
            0 => Value::Null,
            1 => Value::Bool(next() % 2 == 0),
            2 => json!(next() as i64),
            3 => json!(i64::MIN + (next() % 1000) as i64),
            4 => json!((next() % 1000) as f64 / 8.0),
            5 => json!(format!("s{}", next() % 100)),
            6 => json!("⟨thread⟩:abc"),
            7 => Value::Array(
                (0..(next() % 4)).map(|_| gen(next, depth + 1)).collect(),
            ),
            _ => Value::Object(
                (0..(next() % 6))
                    .map(|_| {
                        let k = keys[(next() % keys.len() as u64) as usize].to_string();
                        (k, gen(next, depth + 1))
                    })
                    .collect(),
            ),
        }
    }

    for i in 0..2000 {
        let v = gen(&mut next, 0);
        let id = if i % 7 == 0 { "⟨t⟩:1" } else { "t1" };
        assert_equivalent(id, v);
    }
}
