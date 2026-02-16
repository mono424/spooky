# SpookyRecord Optimization Plan: FieldSlot Cache & Buffer Reuse

## Objective

Achieve O(1) field reads on hot paths and near-zero-allocation serialization for bulk operations. These two changes target the dominant costs in the DBSP pipeline: repeated field lookups during change detection, and record serialization during sync ingestion.

---

## Phase 1: FieldSlot Cached Resolution

### Goal

Eliminate binary search (O(log n), ~10 ns) for repeatedly accessed fields by caching resolved positions. Target: 2–3 ns per access after initial resolution.

### 1.1 Define the `FieldSlot` Struct

Introduce a lightweight, `Copy`-able handle that stores everything needed for direct buffer access: the index position, data offset, data length, and type tag. Add a `generation` field for staleness detection.

### 1.2 Add Generation Counter to `SpookyRecordMut`

Add a `generation: u32` field to `SpookyRecordMut`, initialized to 0. Bump it on any operation that changes buffer layout: `add_field`, `remove_field`, `set_str` (different length splice), `set_field` (splice path). Fixed-width in-place writes (`set_i64`, `set_bool`, `set_str` same length) do **not** bump the generation.

### 1.3 Implement `resolve(name) → Option<FieldSlot>`

Performs a single `find_field` binary search, captures the current generation plus all index metadata into a `FieldSlot`, and returns it. This is the only slow path — called once per field per pipeline setup.

### 1.4 Implement `_at` Accessor Family

Add typed accessors that take a `FieldSlot` reference instead of a field name:

- `get_i64_at`, `get_u64_at`, `get_f64_at`, `get_bool_at`, `get_str_at`
- `set_i64_at`, `set_u64_at`, `set_f64_at`, `set_bool_at`

Each accessor validates the type tag, reads/writes directly at the cached offset, and asserts generation match in debug builds. No hashing, no search — just a bounds check and a memcpy.

### 1.5 Handle `set_str_at` Carefully

Same-length string writes are safe and don't invalidate the slot. Different-length writes require a splice which invalidates all slots. Two strategies:

- **Conservative**: `set_str_at` only accepts same-length writes, returns an error otherwise. Caller falls back to `set_str` + re-resolve.
- **Auto-invalidate**: `set_str_at` performs the splice if needed and returns a new `FieldSlot` with updated offsets and bumped generation. Other outstanding slots become stale.

Recommendation: start conservative. The DBSP hot path mostly reads fields and writes fixed-width values. String mutations on hot paths are uncommon.

### 1.6 Staleness Safety

In debug builds, `_at` accessors assert that `slot.generation == self.generation`. In release builds, the assert compiles away — zero overhead. This catches bugs during development where a slot is used after a structural mutation without re-resolving.

### 1.7 Testing

- Unit tests: resolve → read → mutate fixed-width → read again (slot still valid).
- Unit tests: resolve → structural mutation → attempt read (debug panic on stale slot).
- Benchmark: compare `get_i64("version")` vs `get_i64_at(&version_slot)` to confirm 3–5x improvement.
- Integration test: simulate DBSP change detection loop using FieldSlots for version + content_hash.

---

## Phase 2: Buffer Reuse for Bulk Serialization

### Goal

Eliminate per-record heap allocation when serializing many records in sequence (sync ingestion, snapshot rebuild). The output buffer is allocated once and reused across records.

### 2.1 Add `serialize_into(data, buf)` to `SpookyRecord`

New associated function that takes `&SpookyValue` and `&mut Vec<u8>`. Clears the buffer, then follows the same logic as `serialize` — sort references, write header, append field data via `write_field_into`, backfill index. The caller's `Vec` retains its heap allocation between calls.

### 2.2 Add `from_spooky_value_into(data, buf)` to `SpookyRecordMut`

Same pattern: takes an existing `Vec<u8>`, clears it, builds the record in-place, and returns `SpookyRecordMut` wrapping that buffer. The caller can pass in a buffer from a previous record that's no longer needed.

### 2.3 Amortized Allocation Pattern

The calling code maintains a single reusable buffer:

```
let mut buf = Vec::with_capacity(1024);
for record in incoming_stream {
    SpookyRecord::serialize_into(&record, &mut buf)?;
    store.put(key, &buf);
}
// One allocation, thousands of records
```

### 2.4 Capacity Heuristics

After clearing, the buffer retains its previous capacity. For workloads with variable record sizes, the buffer naturally grows to the high-water mark and stays there. No shrinking needed for typical DBSP batch sizes. If memory pressure becomes a concern, the caller can periodically `buf.shrink_to(reasonable_cap)`.

### 2.5 Testing

- Unit test: serialize two different records into the same buffer, verify both produce correct bytes.
- Benchmark: serialize 1000 records with `serialize` (fresh Vec each time) vs `serialize_into` (reused Vec). Expect 30–50% throughput improvement from eliminated allocations.

---

## Phase 3: FieldSlot Integration with DBSP Pipeline

### Goal

Wire FieldSlots into the actual change detection and view maintenance loops for end-to-end performance gains.

### 3.1 Define a `ResolvedSchema` Struct

For a given view/query, pre-resolve all fields that will be accessed during processing into a struct holding named FieldSlots. This is created once when the view is registered and reused for every record passing through that view.

### 3.2 Apply to Change Detection

The version comparison loop (check if incoming record differs from stored record) currently calls `get_i64("version")` and `get_str("content_hash")` per record. Replace with pre-resolved slots. At scale (millions of records per sync cycle), this saves ~7 ns × 2 fields × N records.

### 3.3 Apply to View Evaluation

Filter and projection operators that access specific fields per record can resolve slots once when the operator tree is compiled, then use `_at` accessors during evaluation. Slots must be re-resolved when a new record is loaded (different buffer), but the resolution is still faster than repeated by-name lookups on the same record if multiple operators access the same field.

### 3.4 Consider a `FieldSlotMap` Cache

For views that process many records with identical schemas (common in DBSP), maintain a `HashMap<u64, FieldSlot>` keyed by name hash. On each new record, check if the slot's generation matches; if not, re-resolve. This avoids even the resolution cost for records with stable schemas.

---

## Implementation Order & Priority

| Step | Effort | Impact | Priority |
|------|--------|--------|----------|
| 1.1–1.4 FieldSlot basics | Small (~50 lines) | High — O(1) hot path reads | **Do first** |
| 1.5–1.6 Staleness safety | Small (~15 lines) | Medium — prevents bugs | Do with 1.1 |
| 1.7 Testing + benchmarks | Small | Required | Do with 1.1 |
| 2.1–2.2 serialize_into | Small (~30 lines) | Medium — bulk serialization | **Do second** |
| 2.3–2.5 Integration + testing | Small | Required | Do with 2.1 |
| 3.1–3.2 DBSP integration | Medium | High — end-to-end gains | Do third |
| 3.3–3.4 View evaluation | Medium | Medium — depends on workload | Evaluate after 3.2 |

---

## Risks & Mitigations

**Stale FieldSlots used after mutation**: The generation counter catches this in debug builds. Document clearly which operations invalidate slots. Consider a lint or wrapper type that forces re-resolution after mutation.

**Over-caching with FieldSlotMap**: If record schemas vary widely within a view, the cache hit rate drops and the HashMap overhead isn't justified. Measure before committing to this — simple per-record `resolve` may be sufficient.

**Buffer reuse across threads**: `serialize_into` with a shared buffer requires the caller to manage ownership. For Rayon-parallel pipelines, use thread-local buffers rather than sharing. This is a caller concern, not a library concern.

**WASM compatibility**: FieldSlot is pure Rust with no platform dependencies. Buffer reuse works identically in WASM. No concerns here.