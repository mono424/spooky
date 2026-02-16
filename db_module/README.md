# 🎃 SpookyDB Module

[![CodSpeed](https://img.shields.io/endpoint?url=https://codspeed.io/badge.json)](https://codspeed.io/timothybesel/spooky_db_module?utm_source=badge)

A high-performance, zero-copy binary record format for Rust. SpookyDB serializes structured data into a compact hybrid format with **O(log n) field lookups**, **O(1) cached access via FieldSlots**, and **nanosecond-level mutation** — no parsing required until you access a field.

## Architecture

SpookyDB uses a **hybrid binary format** that combines native encoding for flat fields with CBOR for nested data. It abstracts over value types using the `RecordSerialize` and `RecordDeserialize` traits, allowing seamless interoperability between `SpookyValue`, `serde_json::Value`, and `cbor4ii::core::Value`.

```
   ┌─────────────────────────────────────────────────────────────┐
   │  Generic Values (RecordSerialize / RecordDeserialize)       │
   │  ├── SpookyValue                                            │
   │  ├── serde_json::Value                                      │
   │  └── cbor4ii::core::Value                                   │
   └──────────────┬──────────────────────────────────────────────┘
                  │                              ▲
        serialization::serialize                 │
                  │                  deserialization::decode_field
                  ▼                              │
┌────────────────────────────────────────────────┴───────────────────┐
│   ┌────────────────────────────┐  ┌────────────────────────────┐   │  
│   │     SpookyRecord<'a>       │  │     SpookyRecordMut        │   │
│   │     (immutable, &[u8])     │  │     (mutable, Vec<u8>)     │   │
│   │     • zero-copy reads      │  │     • in-place updates     │   │
│   │     • no allocations       │  │     • add/remove fields    │   │
│   │     • Copy trait           │  │     • generic setters      │   │
│   └────────────────────────────┘  └────────────────────────────┘   │
└────────────────────────────────────────────────────────────────────┘
```

### Binary Format

```
┌────────────────────── Header (20 bytes) ──────────────────┐
│  field_count: u32 (LE)  |  reserved: [u8; 16]             │
├────────────────── Index (20 bytes × N) ───────────────────┤
│  name_hash:   u64 (LE)   ← SORTED for binary search       │
│  data_offset: u32 (LE)                                    │
│  data_length: u32 (LE)                                    │
│  type_tag:    u8                                          │
│  _padding:    [u8; 3]                                     │
├──────────────────── Field Data ───────────────────────────┤
│  Flat types: native LE bytes (i64, u64, f64, bool)        │
│  Strings: raw UTF-8 bytes                                 │
│  Nested objects/arrays: CBOR-encoded                      │
└───────────────────────────────────────────────────────────┘
```

## Usage

### Creating Records

```rust
use spooky_db_module::spooky_value::SpookyValue;
use spooky_db_module::serialization::{serialize, from_cbor};
use spooky_db_module::spooky_record::SpookyRecord;
use spooky_db_module::spooky_record_mut::SpookyRecordMut;
use serde_json::json;

// 1. Serialize from SpookyValue
let data = SpookyValue::from(json!({"name": "Alice", "age": 30}));
let (bytes, count) = serialize(&data.as_map().unwrap()).unwrap();

// 2. Immutable zero-copy access
let record = SpookyRecord::new(&bytes, count);
let name = record.get_str("name");                // Option<&str> — zero-copy
let age  = record.get_i64("age");                 // Option<i64>
let val: SpookyValue = record.get_field("age").unwrap(); // Generic get

// 3. Mutable in-place access
let mut rec = SpookyRecordMut::new(bytes, count);
rec.set_i64("age", 29).unwrap();                  // ~6 ns
rec.set_str("name", "Bobby").unwrap();            // ~13 ns (same len)

// 4. Generic Setters (works with any RecordSerialize type)
rec.add_field("active", &true).unwrap();          // generic boolean
rec.set_field("meta", &json!({"foo": "bar"})).unwrap(); // generic JSON
```

### FieldSlot Cached Access (O(1))

For hot paths where the same fields are read/written repeatedly (e.g. DBSP change detection), resolve a field once and access it via cached `FieldSlot` — **up to 14× faster** than by-name lookups:

```rust
// Resolve once — O(log n) binary search
let age_slot = rec.resolve("age").unwrap();

// Read via slot — O(1), no hashing, no search (~1 ns)
let age = rec.get_i64_at(&age_slot);       // Some(29)

// Write via slot — O(1), in-place (~0.6 ns)
rec.set_i64_at(&age_slot, 30).unwrap();
```

### Buffer Reuse for Bulk Serialization

Eliminate per-record heap allocations when serializing many records (**~17% faster**):

```rust
use spooky_db_module::serialization::serialize_into;

// Serialize thousands of records with one allocation
let mut buf = Vec::new();
for record in incoming_stream {
    serialize_into(&record, &mut buf)?;
    store.put(key, &buf);  // buf reused on next iteration
}
```

### SpookyRecord (Immutable)

| Method | Returns | Description |
|---|---|---|
| `get_str(name)` | `Option<&str>` | Zero-copy string access |
| `get_i64(name)` | `Option<i64>` | Read i64 field |
| `get_u64(name)` | `Option<u64>` | Read u64 field |
| `get_f64(name)` | `Option<f64>` | Read f64 field |
| `get_bool(name)` | `Option<bool>` | Read bool field |
| `get_field<V>(name)` | `Option<V>` | **Generic**: Deserialize any field to `V` |
| `get_raw(name)` | `Option<FieldRef>` | Raw field reference |
| `get_number_as_f64(name)` | `Option<f64>` | Any numeric → f64 |
| `has_field(name)` | `bool` | Existence check |
| `iter_fields()` | `FieldIter` | Iterate raw fields |
| `field_count()` | `u32` | Number of fields |

### SpookyRecordMut (Mutable)

| Method | Description |
|---|---|
| `new(Vec<u8>, usize)` | Create from existing buffer |
| `new_empty()` | Create empty record |
| **By-name access** | |
| `set_i64(name, val)` | In-place i64 overwrite |
| `set_u64(name, val)` | In-place u64 overwrite |
| `set_f64(name, val)` | In-place f64 overwrite |
| `set_bool(name, val)` | In-place bool overwrite |
| `set_str(name, val)` | String set (splice if needed) |
| `set_str_exact(name, val)` | Same-length string only |
| `set_field<V>(name, &V)` | **Generic**: Set any `RecordSerialize` value |
| `set_null(name)` | Set field to null |
| `add_field<V>(name, &V)` | **Generic**: Add new field |
| `remove_field(name)` | Remove field |
| **FieldSlot cached access** | |
| `resolve(name)` | Resolve field → `FieldSlot` |
| `get_*_at(&slot)` | O(1) cached read |
| `set_*_at(&slot, val)` | O(1) cached write |

### Supported Types

The module supports generic serialization via `RecordSerialize` and `RecordDeserialize`:

- `SpookyValue`: Dynamic value enum (Null, Bool, Number, Str, Array, Object)
- `serde_json::Value`: Standard JSON types
- `cbor4ii::core::Value`: Low-level CBOR types

## Benchmarks

Measured with [Criterion.rs](https://github.com/bheisler/criterion.rs).

### Test Data

The benchmark uses a **341-byte CBOR payload** with 12 top-level fields covering every supported type:

```json
{
  "id": "user:abc123",           // string
  "name": "Alice",               // string
  "age": 28,                     // i64
  "count": 1000,                 // u64
  "score": 99.5,                 // f64
  "active": true,                // bool
  "deleted": false,              // bool
  "metadata": null,              // null
  "tags": ["developer", "rust", "database"],                 // array of strings
  "profile": {                                                // nested object (3 levels deep)
    "bio": "Software engineer",
    "avatar": "https://example.com/avatar.jpg",
    "settings": {
      "theme": "dark",
      "notifications": true,
      "privacy": { "public": false, "level": 3 }
    }
  },
  "history": [                                                // array of objects
    {"action": "login", "timestamp": 1234567890},
    {"action": "update", "timestamp": 1234567900}
  ],
  "mixed_array": [42, "text", true, {"nested": "value"}]     // mixed-type array
}
```

| Format | Size |
|---|---|
| CBOR input | **341 bytes** |
| SpookyRecord binary | **~580 bytes** (header + sorted index + field data) |

### Creating Records

| Operation | Median | Throughput |
|---|---|---|
| `SpookyRecord::serialize` | **3.90 µs** | ~256K records/sec |
| `SpookyRecordMut::from_spooky_value` | **3.84 µs** | ~260K records/sec |
| `SpookyRecordMut::new_empty` | **17.18 ns** | ~58.2M records/sec |
| `SpookyRecordMut::from_vec` | **41.21 ns** | ~24.3M records/sec |

### Reading Values

| Operation | Median | Throughput | Allocs |
|---|---|---|---|
| `SpookyRecord::get_str` | **10.60 ns** | ~94.3M reads/sec | 0 |
| `SpookyRecord::get_i64` | **10.62 ns** | ~94.2M reads/sec | 0 |
| `SpookyRecord::get_bool` | **9.84 ns** | ~101.7M reads/sec | 0 |
| `SpookyRecord::get_field` | **30.92 ns** | ~32.3M reads/sec | 1 |
| `SpookyRecordMut::get_str` | **9.53 ns** | ~104.9M reads/sec | 0 |
| `SpookyRecordMut::get_i64` | **9.02 ns** | ~110.9M reads/sec | 0 |
| `SpookyRecordMut::get_u64` | **9.08 ns** | ~110.1M reads/sec | 0 |
| `SpookyRecordMut::get_f64` | **11.97 ns** | ~83.6M reads/sec | 0 |
| `SpookyRecordMut::get_bool` | **9.10 ns** | ~109.9M reads/sec | 0 |
| `SpookyRecordMut::get_field` | **31.72 ns** | ~31.5M reads/sec | 1 |

### Setting Values

| Operation | Median | Throughput | Description |
|---|---|---|---|
| `set_i64` | **6.44 ns** | ~155.3M writes/sec | In-place overwrite |
| `set_u64` | **8.46 ns** | ~118.2M writes/sec | In-place overwrite |
| `set_f64` | **6.53 ns** | ~153.2M writes/sec | In-place overwrite |
| `set_bool` | **8.16 ns** | ~122.5M writes/sec | In-place overwrite |
| `set_str` (same len) | **13.17 ns** | ~75.9M writes/sec | In-place overwrite |
| `set_str` (diff len) | **27.88 ns** | ~35.9M writes/sec | Splice + fixup |
| `set_str_exact` | **11.82 ns** | ~84.6M writes/sec | Same-length guaranteed |
| `set_field` | **26.26 ns** | ~38.1M writes/sec | Generic path |
| `set_null` | **10.07 ns** | ~99.3M writes/sec | In-place overwrite |

### FieldSlot: Cached Access vs By-Name

FieldSlots eliminate the O(log n) binary search by caching the resolved field position. The slot stores the data offset, length, and type tag from the initial `resolve()` call. Subsequent `_at` accessors skip hashing and searching entirely — they index directly into the buffer.

A `generation` counter on `SpookyRecordMut` tracks layout changes. Fixed-width writes (`set_i64`, `set_bool`, same-length `set_str`) don't change layout, so slots remain valid. Structural mutations (`add_field`, `remove_field`, variable-length splice) bump the generation, invalidating all outstanding slots. Staleness is caught by `debug_assert` in `_at` methods — zero overhead in release builds.

| Operation | By Name | FieldSlot | Speedup |
|---|---|---|---|
| `get_i64` | **9.13 ns** | **1.48 ns** | 6.2× |
| `get_str` | **9.55 ns** | **3.87 ns** | 2.5× |
| `get_bool` | **9.10 ns** | **0.94 ns** | 9.7× |
| `get_f64` | **11.65 ns** | **1.00 ns** | 11.6× |
| `set_i64` | **8.84 ns** | **0.64 ns** | 13.8× |
| `set_str` (same len) | **9.74 ns** | **4.37 ns** | 2.2× |

### Buffer Reuse: Bulk Serialization

`serialize_into` and `from_spooky_value_into` reuse a caller-provided `Vec<u8>`, clearing it but retaining its heap allocation. This eliminates the per-record `Vec::new()` + allocation cost that dominates when serializing many records in sequence (sync ingestion, snapshot rebuild). The buffer naturally grows to the high-water mark and stays there.

| Operation | Fresh Alloc | Reused Buffer | Improvement |
|---|---|---|---|
| `serialize` | **528.30 ns** | **440.29 ns** | 17% faster |
| `from_spooky_value` | **519.85 ns** | **429.34 ns** | 17% faster |

### Field Migration

| Operation | Median | Throughput | Description |
|---|---|---|---|
| `add_field` | **191.18 ns** | ~5.2M ops/sec | Rebuild with sorted insertion |
| `remove_field` | **146.28 ns** | ~6.8M ops/sec | Rebuild without field |

### Throughput Summary

```
  ┌──────────────────────────────────────────────────────────────────────┐
  │ Operation              │ Speed              │ Category               │
  ├────────────────────────┼────────────────────┼────────────────────────┤
  │ FieldSlot reads        │ ~670M-1.06B ops/s  │ O(1) cached, 0 allocs  │
  │ FieldSlot writes       │ ~228M-1.56B ops/s  │ O(1) cached, 0 allocs  │
  │ Typed reads (by name)  │ ~86-111M ops/s     │ O(log n), 0 allocs     │
  │ In-place sets          │ ~118-155M ops/s    │ Zero-alloc overwrites  │
  │ String splice          │ ~36-85M ops/s      │ Buffer resize          │
  │ Add/Remove field       │ ~5-7M ops/s        │ Full rebuild           │
  │ Serialize (reuse buf)  │ ~2.3M recs/s       │ Buffer reuse           │
  │ Serialize (fresh)      │ ~1.9M recs/s       │ Allocates per record   │
  └──────────────────────────────────────────────────────────────────────┘
```

### Run Benchmarks

```bash
# All benchmarks
cargo bench

# Specific group
cargo bench --bench spooky_bench -- reading_values
cargo bench --bench spooky_bench -- fieldslot
cargo bench --bench spooky_bench -- buffer_reuse

# Quick smoke test
cargo bench --bench spooky_bench -- --test

# View HTML reports
open target/criterion/report/index.html
```

## Dependencies

| Crate | Purpose |
|---|---|
| `cbor4ii` | Fast, zero-copy CBOR encoding/decoding |
| `xxhash-rust` | Fast 64-bit hashing for field name lookups |
| `smol_str` | Small-string-optimized string type |
| `serde` | Serialization framework |
| `serde_json` | JSON support |

## License

MIT
