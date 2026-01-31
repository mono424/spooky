# SSP Module Documentation

This document provides a detailed technical reference for the `ssp` (Spooky Stream Processor) module. It covers the core engine architecture, data types, operators, and evaluation logic, including time complexity analysis for critical paths.

---

## 1. Core Data Types (`ssp::engine::types`)

These fundamental types are used throughout the engine for data representation and processing.

### 1.1 `SpookyValue`
**Type**: `Enum`
**Location**: `src/engine/types/spooky_value.rs`

A custom value type optimized for memory efficiency and hashing stability. It wraps JSON-like structures but uses `FastMap` (FxHash) and `SmolStr` (Inline Strings) to reduce allocations.

```rust
pub enum SpookyValue {
    Null,
    Bool(bool),
    Number(f64),
    Str(SmolStr),          // Inline string optimization (<=23 bytes)
    Array(Vec<SpookyValue>),
    Object(FastMap<SmolStr, SpookyValue>), // Non-cryptographic fast hash map
}
```

### 1.2 `Operation`
**Type**: `Enum`
**Location**: `src/engine/types/circuit_types.rs`

Represents the type of mutation being applied to a record.

*   `Create`: Weight +1 (Additive). Changes Content. Changes Membership.
*   `Update`: Weight 0 (Neutral). Changes Content. **No Membership Change**.
*   `Delete`: Weight -1 (Subtractive). Changes Membership.

### 1.3 `Delta`
**Type**: `Struct`
**Location**: `src/engine/types/circuit_types.rs`

Represents a single atomic change to a ZSet (Difference Set).

*   `table`: `SmolStr`
*   `key`: `SmolStr` (Format: `table:id`)
*   `weight`: `i64` (-1, 0, 1)
*   `content_changed`: `bool` (True for Create/Update)

### 1.4 `Path`
**Type**: `Struct`
**Location**: `src/engine/types/path.rs`

Represents a dot-notation accessor path (e.g., `user.address.zip`). Contains a `Vec<SmolStr>` of segments.

### 1.5 Type Aliases (`src/engine/types/zset.rs`)
*   `Weight = i64`: Multiplicity definition.
*   `RowKey = SmolStr`: The normalized string key for rows.
*   `FastMap<K, V>`: `HashMap` using `FxHasher` (faster than SipHash but not DoS safe).
*   `ZSet = FastMap<RowKey, Weight>`: Core data structure for Differential Dataflow.

---

## 2. Operators & AST (`ssp::engine::operators`)

These types define the logical query plan executed by the view engine.

### 2.1 `Operator`
**Type**: `Enum`
**Location**: `src/engine/operators/operator.rs`

The nodes of the Query Plan execution tree.

1.  **`Scan { table: String }`**
    *   Source node. Reads from a database table.
2.  **`Filter { input: Box<Operator>, predicate: Predicate }`**
    *   Applies boolean logic to filter rows.
3.  **`Project { input: Box<Operator>, projections: Vec<Projection> }`**
    *   Transforms rows, selects fields, or runs subqueries.
4.  **`Join { left: Box<Operator>, right: Box<Operator>, on: JoinCondition }`**
    *   Performs an equi-join between two inputs.
5.  **`Limit { input: Box<Operator>, limit: usize, order_by: Option<Vec<OrderSpec>> }`**
    *   Limits results, optionally sorting first.

### 2.2 `Predicate`
**Type**: `Enum`
**Location**: `src/engine/operators/predicate.rs`

Boolean logic expressions.

*   `Eq`, `Neq`, `Gt`, `Gte`, `Lt`, `Lte`: Comparison operators.
*   `Prefix`: String prefix matching.
*   `And`, `Or`: Logical combinators.

### 2.3 `Projection`
**Type**: `Enum`
**Location**: `src/engine/operators/projection.rs`

*   `All`: Select all fields (`*`).
*   `Field { name: Path }`: Select specific field.
*   `Subquery { alias: String, plan: Box<Operator> }`: Nested query execution.

### 2.4 `OrderSpec`
**Type**: `Struct`
**Location**: `src/engine/operators/projection.rs`

*   `field`: `Path` to sort by.
*   `direction`: `String` ("ASC" or "DESC").

---

## 3. Storage & Context (`ssp::engine::circuit`)

The physical layer responsible for storing data and managing consistency.

### 3.1 `Table`
**Type**: `Struct`
**Location**: `src/engine/circuit.rs`

Physical storage unit for a collection of records.

*   `rows`: `FastMap<RowKey, SpookyValue>`
    *   Map of Record ID -> Data. Stores the *current state* of records.
*   `zset`: `ZSet`
    *   Differential index tracking presence/multiplicity.
    *   Used to determine if a record is "active" in the table context.

### 3.2 `Database`
**Type**: `Struct`
**Location**: `src/engine/circuit.rs`

A collection of tables. `FastMap<String, Table>`.

### 3.3 `Circuit`
**Type**: `Struct`
**Location**: `src/engine/circuit.rs`

The main engine container.

*   `db`: `Database`
*   `views`: `Vec<View>`
    *   List of registered materialized views.
    *   **Note**: Uses `Vec` + `swap_remove` for O(1) removal, instead of a Map.
*   `dependency_list`: `FastMap<TableName, SmallVec<[ViewIndex; 4]>>`
    *   Reverse index mapping tables to the views that depend on them.
    *   Optimized with `SmallVec` to avoid heap allocs for common cases (few views per table).

### 3.4 Data Transfer Objects (DTOs)
*   `BatchEntry`: Represents an incoming change (`table`, `op`, `id`, `data`).
*   `LoadRecord`: Represents an initial data load (`table`, `id`, `data`).
*   `ViewUpdateList`: `SmallVec<[ViewUpdate; 2]>` return type for ingestion.

---

## 4. View Logic (`ssp::engine::view`)

### 4.1 `View`
**Type**: `Struct`
**Location**: `src/engine/view.rs`

Represents a live Materialized View.

*   `plan`: `QueryPlan`
*   `cache`: `ZSet`
    *   Stores the *membership* of the view results.
    *   Keys are normalized `table:id` strings.
*   `last_hash`: `String` (BLAKE3)
    *   Used for change detection.
*   `format`: `ViewResultFormat`
*   **Cached Flags** (Not Serialized):
    *   `referenced_tables_cached`: Tables used by this view (for quick-reject).
    *   `is_simple_scan`: Optimization flag.
    *   `is_simple_filter`: Optimization flag.

### 4.2 `QueryPlan`
**Type**: `Struct`
**Location**: `src/engine/view.rs`

Wrapper for the root `Operator` and the view's unique `id`.

---

## 5. Output formats (`ssp::engine::update`)

### 5.1 `ViewResultFormat`
**Type**: `Enum`
**Location**: `src/engine/update.rs`

*   `Flat`: Returns full snapshot of all IDs in view + Hash.
*   `Tree`: (Placeholder) Same structured output.
*   `Streaming`: Returns **Deltas Only** (Created, Updated, Deleted events).

### 5.2 `ViewUpdate`
**Type**: `Enum`
**Location**: `src/engine/update.rs`

Unified return type containing either `MaterializedViewUpdate` (Snapshot) or `StreamingUpdate`.

### 5.3 `DeltaEvent`
**Type**: `Enum`
**Location**: `src/engine/update.rs`

*   `Created`: Record entered the view.
*   `Updated`: Record content changed, but still in view.
*   `Deleted`: Record left the view.

---

## 6. Logic & Algorithms (Complexity Analysis)

### 6.1 Ingestion: `ingest_single`
**Location**: `Circuit::ingest_single`

 Optimized path for real-time updates.

1.  **Storage Update**: **O(1)**.
    *   HashMap insertion into `Table.rows` and `Table.zset`.
2.  **Dependency Lookup**: **O(1)**.
    *   Hash lookup in `dependency_list`.
3.  **View Propagation**: **O(V * C)**.
    *   `V`: Number of views depending on this table (typically small).
    *   `C`: Cost of `process_delta` in the view.

### 6.2 View Processing: `process_delta`
**Location**: `View::process_delta`

1.  **Table Reference Check**: **O(T)** (T = tables in view, typically 1-3).
2.  **Fast Path** (Simple Scan/Filter): **O(1)**.
    *   Directly applies logic without full re-evaluation.
    *   Checks predicate matches (constant time for simple predicates).
    *   Updates `View.cache` (HashMap O(1)).
3.  **Slow Path** (Joins, etc.):
    *   Falls back to `process_batch` logic.

### 6.3 Batch Processing: `process_batch`
**Location**: `View::process_batch`

Used for bulk updates or complex views.

1.  **Compute Membership Delta**:
    *   **Incremental**: **O(D)** where D is batch size.
    *   **Full Scan (Fallback)**: **O(R)** where R is total database rows.
2.  **Categorize Changes**: **O(D)**.
    *   Iterates the computed delta to separate Additions/Removals.
3.  **Formatting**:
    *   **Streaming**: **O(D)** (Constructs event list).
    *   **Flat**: **O(N log N)** where N is view size. Requires **sorting** all record IDs to compute a deterministic state hash.

### 6.4 Evaluation: `eval_snapshot`
**Location**: `View::eval_snapshot`

Full re-calculation (non-incremental).

*   **Scan**: **O(N)**.
*   **Filter**: **O(N)**.
    *   **SIMD Optimization**: `apply_numeric_filter` processes 8x f64s per cycle.
*   **Join**: **O(L + R)**.
    *   Builds Hash Index on Right side: **O(R)**.
    *   Probes Index with Left side: **O(L)**.
*   **Limit/Order**: **O(N log N)** (QuickSort/IntroSort).

### 6.5 Subqueries: `evaluate_subqueries_for_parent_into`
**Location**: `View::evaluate_subqueries_for_parent_into`

Recursive evaluation for nested queries.

*   **Complexity**: **O(P * S)**.
    *   `P`: Number of parent rows.
    *   `S`: Cost of subquery execution.
*   **Warning**: This scales linearly with dataset size. Heavy usage can degrade performance.

---

## 7. Metadata & Versioning (`ssp::engine::metadata`)

### 7.1 `IngestStrategy`
**Type**: `Enum`
**Location**: `src/engine/metadata.rs`

*   `Optimistic`: Auto-increment version on every write.
*   `Explicit`: User provides version number.
*   `HashBased`: Version derived from data content.
*   `None`: Stateless.

### 7.2 `ViewMetadataState`
**Type**: `Struct`
**Location**: `src/engine/metadata.rs`

Stores version maps (`SmolStr` -> `u64`) and content hashes to support the chosen strategy.
