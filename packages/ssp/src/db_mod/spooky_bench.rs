#[path = "data/cbor_flat_map.rs"]
pub mod cbor_flat_map;
use criterion::{Criterion, criterion_group, criterion_main};
use spooky_db_module::deserialization::RecordDeserialize;
use spooky_db_module::serialization::{from_bytes, from_cbor, from_spooky, serialize_into};
use spooky_db_module::spooky_record::record_mut::SpookyRecordMut;
use spooky_db_module::spooky_record::{SpookyReadable, SpookyRecord};
use spooky_db_module::spooky_value::SpookyValue;
use std::hint::black_box;

// ─── Test Data ──────────────────────────────────────────────────────────────

/// A complex CBOR payload with all supported types:
/// - Primitives: strings, i64, u64, f64, bool, null
/// - Arrays: simple and mixed-type arrays
/// - Nested objects: 3 levels deep
///
/// Structure:
/// {
///   "id": "user:abc123",
///   "name": "Alice",
///   "age": 28,
///   "score": 99.5,
///   "active": true,
///   "deleted": false,
///   "metadata": null,
///   "tags": ["developer", "rust", "database"],
///   "count": 1000,
///   "profile": {
///     "bio": "Software engineer",
///     "avatar": "https://example.com/avatar.jpg",
///     "settings": {
///       "theme": "dark",
///       "notifications": true,
///       "privacy": {
///         "public": false,
///         "level": 3
///       }
///     }
///   },
///   "history": [
///     {"action": "login", "timestamp": 1234567890},
///     {"action": "update", "timestamp": 1234567900}
///   ],
///   "mixed_array": [42, "text", true, {"nested": "value"}]
/// }
const BENCH_CBOR: &[u8] = &[
    172, 102, 97, 99, 116, 105, 118, 101, 245, 99, 97, 103, 101, 24, 28, 101, 99, 111, 117, 110,
    116, 25, 3, 232, 103, 100, 101, 108, 101, 116, 101, 100, 244, 103, 104, 105, 115, 116, 111,
    114, 121, 130, 162, 102, 97, 99, 116, 105, 111, 110, 101, 108, 111, 103, 105, 110, 105, 116,
    105, 109, 101, 115, 116, 97, 109, 112, 26, 73, 150, 2, 210, 162, 102, 97, 99, 116, 105, 111,
    110, 102, 117, 112, 100, 97, 116, 101, 105, 116, 105, 109, 101, 115, 116, 97, 109, 112, 26, 73,
    150, 2, 220, 98, 105, 100, 107, 117, 115, 101, 114, 58, 97, 98, 99, 49, 50, 51, 104, 109, 101,
    116, 97, 100, 97, 116, 97, 246, 107, 109, 105, 120, 101, 100, 95, 97, 114, 114, 97, 121, 132,
    24, 42, 100, 116, 101, 120, 116, 245, 161, 102, 110, 101, 115, 116, 101, 100, 101, 118, 97,
    108, 117, 101, 100, 110, 97, 109, 101, 101, 65, 108, 105, 99, 101, 103, 112, 114, 111, 102,
    105, 108, 101, 163, 102, 97, 118, 97, 116, 97, 114, 120, 30, 104, 116, 116, 112, 115, 58, 47,
    47, 101, 120, 97, 109, 112, 108, 101, 46, 99, 111, 109, 47, 97, 118, 97, 116, 97, 114, 46, 106,
    112, 103, 99, 98, 105, 111, 113, 83, 111, 102, 116, 119, 97, 114, 101, 32, 101, 110, 103, 105,
    110, 101, 101, 114, 104, 115, 101, 116, 116, 105, 110, 103, 115, 163, 109, 110, 111, 116, 105,
    102, 105, 99, 97, 116, 105, 111, 110, 115, 245, 103, 112, 114, 105, 118, 97, 99, 121, 162, 101,
    108, 101, 118, 101, 108, 3, 102, 112, 117, 98, 108, 105, 99, 244, 101, 116, 104, 101, 109, 101,
    100, 100, 97, 114, 107, 101, 115, 99, 111, 114, 101, 251, 64, 88, 224, 0, 0, 0, 0, 0, 100, 116,
    97, 103, 115, 131, 105, 100, 101, 118, 101, 108, 111, 112, 101, 114, 100, 114, 117, 115, 116,
    104, 100, 97, 116, 97, 98, 97, 115, 101,
];

/// Parse the CBOR payload into a SpookyValue once.
fn make_spooky_value() -> SpookyValue {
    let cbor_val: cbor4ii::core::Value = cbor4ii::serde::from_slice(BENCH_CBOR).unwrap();
    SpookyValue::from(cbor_val)
}

/// Get a pre-serialized binary buffer for reading/mutation benchmarks.
fn make_binary() -> Vec<u8> {
    let cbor_val: cbor4ii::core::Value = cbor4ii::serde::from_slice(black_box(BENCH_CBOR)).unwrap();
    let (buf, _fc) = from_cbor(&cbor_val).unwrap();
    buf
}

/// Create a SpookyRecordMut from a binary buffer.
fn make_record_mut(binary: &[u8]) -> SpookyRecordMut {
    let (_, fc) = from_bytes(binary).unwrap();
    SpookyRecordMut::new(binary.to_vec(), fc)
}

// ═══════════════════════════════════════════════════════════════════════════
// Group 1: Creating SpookyRecord
// ═══════════════════════════════════════════════════════════════════════════

fn bench_creating_spooky_record(c: &mut Criterion) {
    let mut group = c.benchmark_group("creating_spooky_record");

    // 1a. Full pipeline: CBOR → SpookyValue → from_spooky
    group.bench_function("from_spooky", |b| {
        b.iter(|| {
            let spooky_val = SpookyValue::from_cbor_bytes(BENCH_CBOR).unwrap();
            from_spooky(black_box(&spooky_val)).unwrap()
        })
    });

    group.bench_function("from_cbor", |b| {
        b.iter(|| {
            let cbor_val: cbor4ii::core::Value =
                cbor4ii::serde::from_slice(black_box(BENCH_CBOR)).unwrap();
            from_cbor(black_box(&cbor_val)).unwrap()
        })
    });

    // 2. SpookyRecordMut::new_empty
    group.bench_function("SpookyRecordMut::new_empty", |b| {
        b.iter(|| SpookyRecordMut::new_empty())
    });

    // 3. SpookyRecordMut from existing bytes
    let binary = make_binary();
    group.bench_function("SpookyRecordMut::new (from bytes)", |b| {
        b.iter(|| {
            let bin = black_box(binary.clone());
            let (_, fc) = from_bytes(&bin).unwrap();
            SpookyRecordMut::new(bin, fc)
        })
    });

    group.finish();
}

// ═══════════════════════════════════════════════════════════════════════════
// Group 2: Reading Values
// ═══════════════════════════════════════════════════════════════════════════

fn bench_reading_values(c: &mut Criterion) {
    let mut group = c.benchmark_group("reading_values");
    group.sample_size(500);
    group.measurement_time(std::time::Duration::from_secs(8));

    let binary = make_binary();
    let (buf_ref, fc) = from_bytes(&binary).unwrap();
    let record = SpookyRecord::new(buf_ref, fc);
    let mut_record = make_record_mut(&binary);

    // ── SpookyRecord (immutable) getters ──

    group.bench_function("SpookyRecord::get_field", |b| {
        b.iter(|| black_box(record.get_field::<SpookyValue>(black_box("name"))))
    });

    group.bench_function("SpookyRecord::get_str", |b| {
        b.iter(|| black_box(record.get_str(black_box("name"))))
    });

    group.bench_function("SpookyRecord::get_i64", |b| {
        b.iter(|| black_box(record.get_i64(black_box("age"))))
    });

    group.bench_function("SpookyRecord::get_bool", |b| {
        b.iter(|| black_box(record.get_bool(black_box("active"))))
    });

    // ── SpookyRecordMut getters ──

    group.bench_function("SpookyRecordMut::get_field", |b| {
        b.iter(|| black_box(mut_record.get_field::<SpookyValue>(black_box("name"))))
    });

    group.bench_function("SpookyRecordMut::get_str", |b| {
        b.iter(|| black_box(mut_record.get_str(black_box("name"))))
    });

    group.bench_function("SpookyRecordMut::get_i64", |b| {
        b.iter(|| black_box(mut_record.get_i64(black_box("age"))))
    });

    group.bench_function("SpookyRecordMut::get_u64", |b| {
        b.iter(|| black_box(mut_record.get_u64(black_box("age"))))
    });

    group.bench_function("SpookyRecordMut::get_f64", |b| {
        b.iter(|| black_box(mut_record.get_f64(black_box("age"))))
    });

    group.bench_function("SpookyRecordMut::get_bool", |b| {
        b.iter(|| black_box(mut_record.get_bool(black_box("active"))))
    });

    group.finish();
}

// ═══════════════════════════════════════════════════════════════════════════
// Group 3: Set Values
// ═══════════════════════════════════════════════════════════════════════════

fn bench_set_values(c: &mut Criterion) {
    let mut group = c.benchmark_group("set_values");
    group.sample_size(500);
    group.measurement_time(std::time::Duration::from_secs(8));

    let binary = make_binary();

    // Helper: create a record with extra typed fields for benchmarking
    let make_typed_record = || -> SpookyRecordMut {
        let mut rec = make_record_mut(&binary);
        // Add fields of each type so we can benchmark set_* on matching types
        rec.add_field("bench_u64", &SpookyValue::from(100u64))
            .unwrap();
        rec.add_field("bench_f64", &SpookyValue::from(3.14f64))
            .unwrap();
        rec.add_field("bench_bool", &SpookyValue::from(true))
            .unwrap();
        rec
    };

    group.bench_function("set_i64", |b| {
        let mut rec = make_typed_record();
        b.iter(|| black_box(rec.set_i64(black_box("age"), black_box(42))))
    });

    group.bench_function("set_u64", |b| {
        let mut rec = make_typed_record();
        b.iter(|| black_box(rec.set_u64(black_box("bench_u64"), black_box(42))))
    });

    group.bench_function("set_f64", |b| {
        let mut rec = make_typed_record();
        b.iter(|| black_box(rec.set_f64(black_box("bench_f64"), black_box(42.5))))
    });

    group.bench_function("set_bool", |b| {
        let mut rec = make_typed_record();
        b.iter(|| black_box(rec.set_bool(black_box("bench_bool"), black_box(true))))
    });

    group.bench_function("set_str (same len)", |b| {
        let mut rec = make_typed_record();
        // "Alice" → "Bobby" (5 bytes each)
        b.iter(|| black_box(rec.set_str(black_box("name"), black_box("Bobby"))))
    });

    group.bench_function("set_str (diff len)", |b| {
        let mut rec = make_typed_record();
        let long = "Alice Modified Name";
        let short = "Al";
        let mut toggle = false;
        b.iter(|| {
            let val = if toggle { short } else { long };
            toggle = !toggle;
            black_box(rec.set_str(black_box("name"), black_box(val)))
        })
    });

    group.bench_function("set_str_exact", |b| {
        let mut rec = make_typed_record();
        b.iter(|| black_box(rec.set_str_exact(black_box("name"), black_box("Bobby"))))
    });

    group.bench_function("set_field", |b| {
        let mut rec = make_typed_record();
        let val = SpookyValue::from(99i64);
        b.iter(|| black_box(rec.set_field(black_box("age"), black_box(&val))))
    });

    group.bench_function("set_null", |b| {
        let mut rec = make_typed_record();
        b.iter(|| black_box(rec.set_null(black_box("age"))))
    });

    group.finish();
}

// ═══════════════════════════════════════════════════════════════════════════
// Group 4: Field Migration (add/remove)
// ═══════════════════════════════════════════════════════════════════════════

fn bench_field_migration(c: &mut Criterion) {
    let mut group = c.benchmark_group("field_migration");

    let binary = make_binary();

    group.bench_function("add_field", |b| {
        b.iter_batched(
            || make_record_mut(&binary),
            |mut rec| {
                rec.add_field(
                    black_box("new_bench_field"),
                    black_box(&SpookyValue::from(12345i64)),
                )
                .unwrap();
            },
            criterion::BatchSize::SmallInput,
        )
    });

    group.bench_function("remove_field", |b| {
        b.iter_batched(
            || make_record_mut(&binary),
            |mut rec| {
                rec.remove_field(black_box("name")).unwrap();
            },
            criterion::BatchSize::SmallInput,
        )
    });

    group.finish();
}

// ═══════════════════════════════════════════════════════════════════════════
// Group 5: FieldSlot — cached O(1) access vs by-name O(log n)
// ═══════════════════════════════════════════════════════════════════════════

fn bench_fieldslot(c: &mut Criterion) {
    let mut group = c.benchmark_group("fieldslot");
    // Sub-nanosecond operations need many samples and longer measurement
    // to reduce noise and avoid measurement overhead dominating.
    group.sample_size(1000);
    group.measurement_time(std::time::Duration::from_secs(10));
    group.warm_up_time(std::time::Duration::from_secs(3));

    let binary = make_binary();
    let mut rec = make_record_mut(&binary);

    // Resolve slots up-front
    let age_slot = rec.resolve("age").unwrap();
    let name_slot = rec.resolve("name").unwrap();
    let active_slot = rec.resolve("active").unwrap();
    let score_slot = rec.resolve("score").unwrap();

    // ── Reads ──
    // black_box on BOTH inputs and return values prevents the compiler
    // from eliding the work — critical for sub-ns operations.

    group.bench_function("get_i64 (by name)", |b| {
        b.iter(|| black_box(rec.get_i64(black_box("age"))))
    });

    group.bench_function("get_i64_at (slot)", |b| {
        b.iter(|| black_box(rec.get_i64_at(black_box(&age_slot))))
    });

    group.bench_function("get_str (by name)", |b| {
        b.iter(|| black_box(rec.get_str(black_box("name"))))
    });

    group.bench_function("get_str_at (slot)", |b| {
        b.iter(|| black_box(rec.get_str_at(black_box(&name_slot))))
    });

    group.bench_function("get_bool (by name)", |b| {
        b.iter(|| black_box(rec.get_bool(black_box("active"))))
    });

    group.bench_function("get_bool_at (slot)", |b| {
        b.iter(|| black_box(rec.get_bool_at(black_box(&active_slot))))
    });

    group.bench_function("get_f64 (by name)", |b| {
        b.iter(|| black_box(rec.get_f64(black_box("score"))))
    });

    group.bench_function("get_f64_at (slot)", |b| {
        b.iter(|| black_box(rec.get_f64_at(black_box(&score_slot))))
    });

    // ── Writes ──
    // black_box the Result to prevent the compiler from eliding the write.

    group.bench_function("set_i64 (by name)", |b| {
        b.iter(|| black_box(rec.set_i64(black_box("age"), black_box(42))))
    });

    group.bench_function("set_i64_at (slot)", |b| {
        b.iter(|| black_box(rec.set_i64_at(black_box(&age_slot), black_box(42))))
    });

    group.bench_function("set_str_exact (by name)", |b| {
        b.iter(|| black_box(rec.set_str_exact(black_box("name"), black_box("Bobby"))))
    });

    group.bench_function("set_str_at (slot, same len)", |b| {
        b.iter(|| black_box(rec.set_str_at(black_box(&name_slot), black_box("Bobby"))))
    });

    group.finish();
}

// ═══════════════════════════════════════════════════════════════════════════
// Group 6: Buffer Reuse — serialize_into vs from_spooky
// ═══════════════════════════════════════════════════════════════════════════

fn bench_buffer_reuse(c: &mut Criterion) {
    let mut group = c.benchmark_group("buffer_reuse");

    let value = make_spooky_value();
    let map = match &value {
        SpookyValue::Object(m) => m,
        _ => panic!("Expected object"),
    };

    // ── from_spooky (fresh alloc) vs serialize_into (reuse) ──

    group.bench_function("from_spooky (fresh alloc)", |b| {
        b.iter(|| from_spooky(black_box(&value)).unwrap())
    });

    group.bench_function("serialize_into (reuse)", |b| {
        let mut buf = Vec::new();
        b.iter(|| serialize_into(black_box(map), &mut buf).unwrap())
    });

    group.finish();
}

// ─── Criterion Main ─────────────────────────────────────────────────────────

criterion_group!(
    benches,
    bench_creating_spooky_record,
    bench_reading_values,
    bench_set_values,
    bench_field_migration,
    bench_fieldslot,
    bench_buffer_reuse,
);
criterion_main!(benches);
