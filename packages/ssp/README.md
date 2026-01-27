# SSP – Spooky Stream Processor

**High-performance incremental materialized views for real-time applications.**

---

## Overview

SSP (Spooky Stream Processor) is an incremental view maintenance engine that powers the reactive layer between **SurrealDB** and your application via the **Spooky Sidecar**.

```
SurrealDB  ──LIVE──►  Sidecar  ──ingest──►  SSP Circuit  ──ViewUpdate──►  DB / Client
```

---

## Communication Flow

### The Full Pipeline

```mermaid
sequenceDiagram
    participant DB as SurrealDB
    participant SC as Sidecar (apps/ssp)
    participant DBSP as Circuit (packages/ssp)
    participant OUT as DB Persistence

    Note over DB,OUT: === INGEST FLOW ===

    DB->>SC: Change Event (LIVE SELECT)
    SC->>SC: prepare() - sanitize + hash
    SC->>DBSP: ingest_record(table, op, id, record, hash, is_optimistic)
    DBSP->>DBSP: View.process_ingest() for affected views
    DBSP-->>SC: Vec<ViewUpdate>

    alt Flat/Tree Format
        SC->>OUT: UPDATE _spooky_incantation SET hash, array
    else Streaming Format
        SC->>OUT: BEGIN TRANSACTION<br/>RELATE/UPDATE/DELETE...<br/>COMMIT
    end
```

### Deep Subquery Collection

The engine uses a recursive `collect` method to identify all dependent records, including those from nested subqueries (e.g., `Thread -> Author -> Role`). This ensures that changes to deep dependencies correctly trigger updates for the main view.

---

## Traffic Objects

### 1. IngestRequest (Sidecar Input)

What the sidecar receives from SurrealDB:

```rust
struct IngestRequest {
    table: String,        // "thread", "user", "comment"
    op: String,           // "CREATE", "UPDATE", "DELETE"
    id: String,           // "thread:abc123"
    record: Value,        // Full JSON record
}
```

### 2. Circuit.ingest_record() Parameters

```rust
circuit.ingest_record(
    table: &str,          // Table name
    op: &str,             // Operation type
    id: &str,             // Record ID (table:key format)
    record: Value,        // JSON record data
    hash: &str,           // Blake3 hash of record
    is_optimistic: bool,  // true = increment versions, false = keep versions
) -> Vec<ViewUpdate>
```

### 3. ViewUpdate (Engine Output)

The engine returns one of three formats:

#### Flat Format

```rust
ViewUpdate::Flat(MaterializedViewUpdate {
    query_id: "thread_list",
    result_hash: "d4a0562e39718e02...",
    result_data: [
        ("thread:abc123", 1),  // (record_id, version)
        ("user:xyz789", 1),
    ],
})
```

#### Streaming Format

```rust
ViewUpdate::Streaming(StreamingUpdate {
    view_id: "thread_list",
    records: [
        DeltaRecord { id: "thread:abc123", event: Created, version: 1 },
        DeltaRecord { id: "user:xyz789", event: Updated, version: 2 },
        DeltaRecord { id: "comment:old", event: Deleted, version: 0 },
    ],
})
```

### 4. DeltaEvent Types

| Event     | Meaning                  | DB Operation                                                 |
| --------- | ------------------------ | ------------------------------------------------------------ |
| `Created` | Record added to view     | `RELATE $from->_spooky_list_ref->$to SET version`            |
| `Updated` | Record content changed   | `UPDATE $from->_spooky_list_ref SET version WHERE out = $to` |
| `Deleted` | Record removed from view | `DELETE $from->_spooky_list_ref WHERE out = $to`             |

### 5. DB Persistence (Sidecar Output)

#### For Flat/Tree:

```sql
UPDATE <record>$id SET hash = <string>$hash, array = $array
-- Example:
UPDATE _spooky_incantation:thread_list SET
  hash = "d4a0562e39718e02...",
  array = [["thread:abc123", 1], ["user:xyz789", 1]]
```

#### For Streaming (Graph Edges):

```sql
BEGIN TRANSACTION;

-- Created
RELATE _spooky_incantation:thread_list->_spooky_list_ref->thread:abc123
  SET version = 1, clientId = $clientId;

-- Updated
UPDATE _spooky_incantation:thread_list->_spooky_list_ref
  SET version = 2 WHERE out = thread:abc123;

-- Deleted
DELETE _spooky_incantation:thread_list->_spooky_list_ref
  WHERE out = comment:old;

COMMIT TRANSACTION;
```

---

## Debug Logs

SSP outputs debug logs prefixed with `[SSP DEBUG]`. Here's what each log means:

### Ingestion Logs

```
[SSP DEBUG] DEBUG: Changed tables: ["thread", "user"]
```

Tables affected by the current batch ingest.

### View Processing Logs

```
[SSP DEBUG] DEBUG VIEW: id=thread_list is_first_run=true has_subquery_changes=false is_streaming=true
```

| Field                  | Meaning                                        |
| ---------------------- | ---------------------------------------------- |
| `id`                   | View identifier                                |
| `is_first_run`         | First time evaluating (full scan)              |
| `has_subquery_changes` | Subquery tables have changes (needs full scan) |
| `is_streaming`         | Using streaming format                         |

```
[SSP DEBUG] DEBUG VIEW: id=thread_list view_delta_empty=false has_cached_updates=true is_optimistic=true updated_ids_len=3
```

| Field                | Meaning                             |
| -------------------- | ----------------------------------- |
| `view_delta_empty`   | Whether the main delta is empty     |
| `has_cached_updates` | Cached records need version updates |
| `is_optimistic`      | Versions will be incremented        |
| `updated_ids_len`    | Number of records being updated     |

### Version Updates

```
[SSP DEBUG] DEBUG VIEW: Incrementing version for id=thread:abc123 old=1 new=2
```

Version bump when `is_optimistic=true`.

### Subquery Detection

```
[SSP DEBUG] DEBUG has_changes: view=thread_list subquery_tables=["user"] delta_tables=["thread"]
```

Checking if subquery tables were affected.

```
[SSP DEBUG] DEBUG has_changes: view=thread_list NO CHANGES FOUND
```

Subqueries unaffected, can use delta evaluation.

### Streaming Output

```
[SSP DEBUG] DEBUG STREAMING_EMIT: view=thread_list delta_records_count=2 records=[("thread:abc123", Created), ("user:xyz789", Created)]
```

Final streaming update being emitted.

### Updated Record Detection

```
[SSP DEBUG] DEBUG get_updated_records_streaming: view=thread_list table=thread found versioned record=thread:abc123
```

Found a cached record that needs version update.

---

## Output Formats

| Format        | Payload                       | Use Case                          |
| ------------- | ----------------------------- | --------------------------------- |
| **Flat**      | `[(id, version), ...]` + hash | Simple reconciliation, full state |
| **Streaming** | `[{id, event, version}, ...]` | Real-time UI, minimal bandwidth   |
| **Tree**      | Hierarchical (planned)        | Nested data structures            |

---

## Key Types Reference

### QueryPlan

```rust
pub struct QueryPlan {
    pub id: String,       // Unique view identifier
    pub root: Operator,   // Operator tree root
}
```

### Operator (subset)

```rust
pub enum Operator {
    Scan { table: String },
    Filter { input: Box<Operator>, predicate: Predicate },
    Project { input: Box<Operator>, projections: Vec<Projection> },
    Limit { input: Box<Operator>, limit: usize, order_by: Option<...> },
    // ... more operators
}
```

### ViewResultFormat

```rust
pub enum ViewResultFormat {
    Flat,       // Default - full snapshot
    Tree,       // Hierarchical
    Streaming,  // Delta events
}
```

---

## Version Semantics

| `is_optimistic` | Behavior                      | Use Case                       |
| --------------- | ----------------------------- | ------------------------------ |
| `true`          | Increment versions on changes | Local mutations (client-side)  |
| `false`         | Keep versions as-is           | Remote sync (server authority) |

---

## Module Structure

```
ssp/
├── src/
│   ├── lib.rs            # StreamProcessor trait, public API
│   ├── converter.rs      # sql → Operator tree
│   ├── sanitizer.rs      # Input normalization
│   ├── service.rs        # High-level helpers (prepare, register)
│   └── engine/
│       ├── circuit.rs    # Core coordinator (Database + Views)
│       ├── view.rs       # View logic, delta evaluation
│       ├── update.rs     # ViewUpdate, DeltaEvent, formatters
│       ├── operators/    # Operator definitions
│       └── types/        # SpookyValue, ZSet, FastMap
└── tests/
    ├── e2e_communication_test.rs   # Full pipeline validation
    ├── streaming_subquery_edge_test.rs
    └── ...
```

---

## API Reference

### Service Helpers (`ssp::service`)

These utilities handle input normalization and hashing before data reaches the circuit.

#### `prepare(record: Value) -> (SpookyValue, String)`

Normalizes and hashes a single record.

- **Input**: Raw JSON record
- **Returns**: `(NormalizedValue, HashString)`
- **Use Case**: Standard ingestion

#### `prepare_batch(records: Vec<Value>) -> Vec<(SpookyValue, String)>`

Normalizes and hashes a list of records.

- **Optimization**: Uses `rayon` for parallel processing (native only)
- **Use Case**: Bulk import / Initial load

#### `prepare_fast(record: Value) -> (SpookyValue, String)`

Hashes the record _without_ normalization processing.

- **Warning**: Input must be pre-normalized
- **Use Case**: High-throughput scenarios where data integrity is guaranteed upstream

### Core Library (`ssp::lib`)

The main entry points for interacting with the SSP engine.

#### `ingest_record(...) -> Vec<ViewUpdate>`

Ingests a single change event.

```rust
fn ingest_record(
    &mut self,
    table: &str,          // Table name
    op: &str,             // "CREATE", "UPDATE", "DELETE"
    id: &str,             // Record ID
    record: Value,        // Normalized record data
    hash: &str,           // Pre-computed hash
    is_optimistic: bool,  // true = increment versions
) -> Vec<ViewUpdate>
```

#### `ingest_batch(...) -> Vec<ViewUpdate>`

Ingests a batch of changes efficiently.

```rust
fn ingest_batch(
    &mut self,
    batch: Vec<(String, String, String, Value, String)>, // (table, op, id, record, hash)
    is_optimistic: bool,
) -> Vec<ViewUpdate>
```

- **Optimization**: Processes all global state updates first, then re-evaluates views once per batch.

---

## Quick Usage

```rust
use ssp::{Circuit, StreamProcessor};
use ssp::engine::update::ViewResultFormat;

// Create circuit
let mut circuit = Circuit::new();

// Register a streaming view
circuit.register_view(plan, params, Some(ViewResultFormat::Streaming));

// Ingest changes
let updates = circuit.ingest_record(
    "thread",           // table
    "CREATE",           // op
    "thread:abc123",    // id
    record,             // JSON
    &hash,              // blake3 hash
    true,               // is_optimistic
);

// Process updates
for update in updates {
    match update {
        ViewUpdate::Flat(m) => {
            // UPDATE _spooky_incantation SET hash, array
        }
        ViewUpdate::Streaming(s) => {
            // Batch all operations into single transaction
            let mut ops = Vec::new();
            for rec in s.records {
                match rec.event {
                    DeltaEvent::Created => ops.push("RELATE ..."),
                    DeltaEvent::Updated => ops.push("UPDATE ..."),
                    DeltaEvent::Deleted => ops.push("DELETE ..."),
                }
            }
            // Execute as one atomic transaction (O(1) round-trip)
            db.query("BEGIN TRANSACTION; ... COMMIT;");
        }
        _ => {}
    }
}
```

---

## Testing

```bash
# Run all tests
cargo test

# Run E2E communication test with verbose output
cargo test --test e2e_communication_test -- --nocapture

# Run streaming edge tests
cargo test --test streaming_subquery_edge_test -- --nocapture
```

---

## License

MIT © Spooky Project
