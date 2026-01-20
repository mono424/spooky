# DBSP Engine Refactored Architecture

## Overview

The engine is now cleanly separated into three core modules:

```
┌─────────────────────────────────────────────────────────────────────┐
│                           circuit.rs                                 │
│                    (Ingestion & Orchestration)                       │
│                                                                      │
│  BatchEntry → ingest_entries() → propagate_deltas() → View.process() │
└─────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────┐
│                            view.rs                                   │
│                    (Pure DBSP Delta Computation)                     │
│                                                                      │
│  eval_snapshot() / eval_delta_batch() → ZSet delta → RawViewResult   │
│                                                                      │
│  OUTPUTS: RawViewResult (format-agnostic)                           │
└─────────────────────────────────────────────────────────────────────┘
                          │                    │
                          ▼                    ▼
┌────────────────────────────────┐  ┌─────────────────────────────────┐
│         metadata.rs            │  │          update.rs               │
│   (Version/Hash Strategies)    │  │    (Output Formatting)           │
│                                │  │                                  │
│  - VersionStrategy::Optimistic │  │  RawViewResult → ViewUpdate      │
│  - VersionStrategy::Explicit   │  │                                  │
│  - VersionStrategy::HashBased  │  │  - Flat: [(id, version), hash]   │
│  - VersionStrategy::None       │  │  - Tree: (future hierarchical)   │
│                                │  │  - Streaming: DeltaRecord[]      │
│  ViewMetadataState             │  │                                  │
│  MetadataProcessor             │  │  build_update(raw, format)       │
└────────────────────────────────┘  └─────────────────────────────────┘
```

## Module Responsibilities

### 1. `circuit.rs` - Ingestion & Orchestration

**Purpose:** Entry point for data changes, orchestrates view updates.

**Key Types:**
- `Operation` - Create/Update/Delete enum
- `BatchEntry` - Single record change
- `IngestBatch` - Builder for batch operations
- `Circuit` - Main processor with views and dependency graph

**Key Methods:**
```rust
circuit.ingest(batch, is_optimistic)      // Builder API
circuit.ingest_entries(entries, is_opt)   // Direct entries
circuit.ingest_record(...)                // Single record (backward compat)
circuit.ingest_batch(...)                 // Tuple batch (backward compat)
```

### 2. `view.rs` - Pure DBSP Delta Computation

**Purpose:** Computes what changed in a view (pure logic, no formatting).

**Outputs:** `RawViewResult` - format-agnostic data:
```rust
pub struct RawViewResult {
    pub query_id: String,
    pub records: Vec<(String, u64)>,  // Full snapshot
    pub delta: ViewDelta,              // Changes only
    pub is_first_run: bool,
}
```

**Key Methods:**
```rust
view.process_ingest(deltas, db, is_optimistic) -> Option<ViewUpdate>
```

The view computes:
1. Delta (what records entered/left the view)
2. Updates (what records changed content but stayed)
3. Subquery changes (nested query results)

### 3. `metadata.rs` - Version/Hash Strategies (NEW)

**Purpose:** Pluggable source of truth for versioning.

**Strategies:**
```rust
pub enum VersionStrategy {
    Optimistic,  // Auto-increment on update (default)
    Explicit,    // Version provided during ingestion
    HashBased,   // Version derived from content hash
    None,        // No version tracking
}
```

**Key Types:**
```rust
// Per-record metadata (during ingestion)
pub struct RecordMeta {
    pub version: Option<u64>,   // Explicit version
    pub hash: Option<String>,   // Content hash
    pub custom: Option<SpookyValue>,
}

// Batch metadata
pub struct BatchMeta {
    pub records: FastMap<SmolStr, RecordMeta>,
    pub default_strategy: VersionStrategy,
}

// Persistent state
pub struct ViewMetadataState {
    pub versions: VersionMap,     // record_id -> version
    pub hashes: HashStore,        // record_id -> hash (for HashBased)
    pub strategy: VersionStrategy,
    pub last_result_hash: String,
}
```

**Usage Example (Explicit Version):**
```rust
let meta = BatchMeta::new()
    .with_strategy(VersionStrategy::Explicit)
    .add_record("user:1", RecordMeta::new().with_version(13));

circuit.ingest_with_meta(batch, meta, is_optimistic);
```

### 4. `update.rs` - Output Formatting

**Purpose:** Transforms `RawViewResult` into final output format.

**Formats:**
```rust
pub enum ViewResultFormat {
    Flat,       // [(id, version), ...] with hash
    Tree,       // Hierarchical (future)
    Streaming,  // DeltaRecord[] events
}
```

**Key Function:**
```rust
pub fn build_update(raw: RawViewResult, format: ViewResultFormat) -> ViewUpdate
```

**Output Types:**
```rust
// Flat/Tree
pub struct MaterializedViewUpdate {
    pub query_id: String,
    pub result_hash: String,
    pub result_data: Vec<(String, u64)>,
}

// Streaming
pub struct StreamingUpdate {
    pub view_id: String,
    pub records: Vec<DeltaRecord>,
}

pub struct DeltaRecord {
    pub id: String,
    pub event: DeltaEvent,  // Created/Updated/Deleted
    pub version: u64,
}
```

## Data Flow

### Standard Flow (Optimistic)

```
1. Ingest: circuit.ingest_record("users", "UPDATE", "user:1", data, hash, true)
                                                                        │
2. Circuit groups by table, creates delta                               │
                                                                        ▼
3. For each affected view:
   view.process_ingest(deltas, db, is_optimistic=true)
   │
   ├─ Compute ZSet delta (eval_delta_batch or eval_snapshot)
   ├─ Identify updates (records in view that changed)
   ├─ MetadataProcessor.compute_update_version() → increment version
   ├─ Build RawViewResult
   └─ build_update(raw, format) → ViewUpdate::Streaming/Flat/Tree
```

### Explicit Version Flow

```
1. Ingest with metadata:
   let meta = BatchMeta::new()
       .add_record("user:1", RecordMeta::new().with_version(13));
   
   circuit.ingest_with_meta(batch, meta, false);
                                        │
2. View processing:                     │
   view.process_ingest_with_meta(deltas, db, meta)
   │
   ├─ Compute delta (same as before)
   ├─ MetadataProcessor uses VersionStrategy::Explicit
   ├─ Uses provided version 13 instead of incrementing
   └─ Build ViewUpdate with explicit versions
```

### Hash-Based Version Flow (for Tree mode)

```
1. Configure view with HashBased strategy:
   circuit.register_view(plan, params, Some(ViewResultFormat::Tree))
   // View uses VersionStrategy::HashBased internally

2. On update:
   view.process_ingest(deltas, db, is_optimistic)
   │
   ├─ Compute delta
   ├─ For each record, compute content hash
   ├─ Compare hashes: if different → changed
   ├─ Store new hash in ViewMetadataState.hashes
   └─ Build RawViewResult with hash-derived versions
```

## Performance Considerations

1. **Hot Path Optimization:**
   - `view.rs` does pure computation (no formatting overhead)
   - `metadata.rs` methods are `#[inline]`
   - `update.rs` formatting is only called once at the end

2. **Memory:**
   - Flat/Tree: Uses `cache` ZSet (full snapshot)
   - Streaming: Uses `version_map` only (lighter)
   - HashBased: Adds `hashes` map (when needed)

3. **Allocation:**
   - `RawViewResult` is built once per view update
   - `build_update()` consumes it (no copy)
   - Pre-allocation with `with_capacity()` throughout

## Migration Guide

### From Old Code

```rust
// OLD: View handled everything
view.process_ingest(deltas, db, is_optimistic) -> Option<ViewUpdate>
// View computed delta + handled versioning + formatted output

// NEW: Same API, but internally separated
view.process_ingest(deltas, db, is_optimistic) -> Option<ViewUpdate>
// 1. View computes delta → RawViewResult
// 2. MetadataProcessor handles versions
// 3. build_update() formats output
```

### Adding Explicit Versions (NEW capability)

```rust
// Create metadata for explicit version control
let meta = BatchMeta::new()
    .with_strategy(VersionStrategy::Explicit)
    .add_record("record:1", RecordMeta::new().with_version(42));

// Ingest with metadata
circuit.ingest_with_meta(
    IngestBatch::new().update("table", "record:1", data, hash),
    meta,
    false,  // is_optimistic (ignored with Explicit strategy)
);
```

### Using Hash-Based Versioning (for Tree)

```rust
// Register view with Tree format (uses hash-based internally)
circuit.register_view(
    plan,
    params,
    Some(ViewResultFormat::Tree),
);

// Or configure explicitly
let view = View::new_with_metadata(
    plan,
    params,
    ViewMetadataState::new(VersionStrategy::HashBased),
);
```

## Files Changed

| File | Status | Changes |
|------|--------|---------|
| `mod.rs` | Updated | Added metadata module, updated exports |
| `update.rs` | Refactored | Added RawViewResult, ViewDelta, kept build_update |
| `metadata.rs` | **NEW** | VersionStrategy, RecordMeta, BatchMeta, MetadataProcessor, ViewMetadataState |
| `view.rs` | Needs Update | Use RawViewResult as output, delegate to update.rs |
| `circuit.rs` | Needs Update | Add `ingest_with_meta()` method |

## Next Steps

1. **Update view.rs** to:
   - Output `RawViewResult` instead of building `ViewUpdate` directly
   - Use `MetadataProcessor` for version computation
   - Use `ViewMetadataState` instead of raw `VersionMap`

2. **Update circuit.rs** to:
   - Add `ingest_with_meta()` method
   - Pass `BatchMeta` through to views

3. **Test** the new architecture:
   - Verify all existing tests pass
   - Add tests for explicit versioning
   - Add tests for hash-based versioning
