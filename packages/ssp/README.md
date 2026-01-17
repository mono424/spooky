# SSP – Spooky Stream Processor

<div align="center">

**High-performance incremental materialized views for real-time applications.**

[![Rust](https://img.shields.io/badge/Rust-000000?style=for-the-badge&logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![SurrealDB](https://img.shields.io/badge/SurrealDB-F06292?style=for-the-badge)](https://surrealdb.com/)

</div>

---

## Overview

SSP (Spooky Stream Processor) is an incremental view maintenance engine. It converts **SurrealQL queries** into internal operator graphs and maintains **materialized views** that update in real-time as data changes—pushing only deltas instead of recomputing entire query results.

SSP powers the reactive layer of the Spooky ecosystem, sitting between **SurrealDB** (or any data source) and your application via the **Spooky Sidecar**.

---

## Architecture

```mermaid
flowchart TB
    subgraph External["External Systems"]
        SDB[(SurrealDB)]
        APP[Client Application]
    end
    
    subgraph Sidecar["Spooky Sidecar"]
        CDC[Change Data Capture]
        WS[WebSocket Layer]
        REG[View Registration]
    end
    
    subgraph SSP["SSP Engine"]
        direction TB
        SVC[Service Layer]
        CONV[Converter]
        CIRC[Circuit]
        subgraph Views["Registered Views"]
            V1[View 1]
            V2[View 2]
            VN[View N]
        end
        DB[(In-Memory Tables)]
    end
    
    SDB -->|LIVE Changes| CDC
    CDC -->|Records| SVC
    APP -->|SurrealQL| REG
    REG -->|Config| SVC
    SVC -->|Normalize + Hash| CONV
    CONV -->|QueryPlan| CIRC
    CIRC --> Views
    Views -->|Delta Updates| WS
    CIRC --> DB
    WS -->|Push| APP
    
    style SSP fill:#1a1a2e,stroke:#16213e,color:#fff
    style Sidecar fill:#0f3460,stroke:#16213e,color:#fff
    style External fill:#e94560,stroke:#16213e,color:#fff
```

---

## Data Flow

### 1. Record Ingestion Pipeline

```mermaid
sequenceDiagram
    participant SDB as SurrealDB
    participant SC as Spooky Sidecar
    participant SVC as Service Layer
    participant C as Circuit
    participant V as Views
    participant APP as Client
    
    SDB->>SC: LIVE Query Event (CREATE/UPDATE/DELETE)
    SC->>SVC: Forward Record
    SVC->>SVC: Normalize (sanitizer)
    SVC->>SVC: Hash (blake3)
    SVC->>C: ingest_batch(records)
    C->>C: Update Tables
    C->>V: Propagate Delta
    V->>V: Incremental Eval
    V-->>C: ViewUpdate (if changed)
    C-->>SC: Delta Updates
    SC-->>APP: Push via WebSocket
```

### 2. View Registration Pipeline

```mermaid
sequenceDiagram
    participant APP as Client
    participant SC as Spooky Sidecar
    participant SVC as Service Layer
    participant CONV as Converter
    participant C as Circuit
    participant V as New View
    
    APP->>SC: Register View (SurrealQL + params)
    SC->>SVC: view::prepare_registration()
    SVC->>CONV: convert_surql_to_dbsp()
    CONV-->>SVC: Operator Tree
    SVC-->>SC: QueryPlan + Metadata
    SC->>C: register_view(plan, params, format)
    C->>V: Create View Instance
    V->>V: Initial Full Scan
    V-->>C: Initial Snapshot
    C-->>SC: ViewUpdate (full result)
    SC-->>APP: Push Initial State
```

---

## Core Components

### Circuit

The central coordinator managing tables and views. Handles record ingestion and propagates changes to affected views.

```rust
use ssp::{Circuit, StreamProcessor};

let mut circuit = Circuit::new();

// Register a view
circuit.register_view(plan, params, Some(ViewResultFormat::Flat));

// Ingest data
let updates = circuit.ingest_batch(records, false);
```

### View

Maintains a single materialized view. Performs incremental delta evaluation when possible, falling back to full scans when necessary.

```mermaid
flowchart LR
    subgraph View["View Processing"]
        IN[Input Delta]
        CACHE[Version Map]
        EVAL{Can Increment?}
        DELTA[eval_delta_batch]
        SNAP[eval_snapshot]
        OUT[ViewUpdate]
    end
    
    IN --> EVAL
    EVAL -->|Yes| DELTA
    EVAL -->|No| SNAP
    DELTA --> CACHE
    SNAP --> CACHE
    CACHE --> OUT
    
    style View fill:#2d3436,stroke:#636e72,color:#fff
```

### QueryPlan & Operator Tree

A query is represented as a tree of operators:

```mermaid
flowchart TB
    ROOT[Project]
    FILTER[Filter]
    JOIN[Join]
    SCAN1[Scan: users]
    SCAN2[Scan: posts]
    
    ROOT --> FILTER
    FILTER --> JOIN
    JOIN --> SCAN1
    JOIN --> SCAN2
    
    style ROOT fill:#6c5ce7,stroke:#a29bfe,color:#fff
    style FILTER fill:#00b894,stroke:#55efc4,color:#fff
    style JOIN fill:#fdcb6e,stroke:#ffeaa7,color:#000
    style SCAN1 fill:#e17055,stroke:#fab1a0,color:#fff
    style SCAN2 fill:#e17055,stroke:#fab1a0,color:#fff
```

**Supported Operators:**

| Operator | Description |
|----------|-------------|
| `Scan` | Read from a table |
| `Filter` | Apply predicates |
| `Project` | Select/transform fields |
| `Join` | Combine tables (inner, left) |
| `OrderBy` | Sort results |
| `Limit` | Restrict result count |
| `Aggregate` | SUM, COUNT, AVG, MIN, MAX |

---

## Example: Real-Time Todo App

### 1. Register a View

```json
{
  "id": "user_todos_active",
  "clientId": "client_abc123",
  "surrealQL": "SELECT * FROM todos WHERE user_id = $user_id AND completed = false ORDER BY created_at DESC LIMIT 50",
  "params": { "user_id": "user:alice" },
  "ttl": "1h",
  "lastActiveAt": "2026-01-17T16:30:00Z"
}
```

The Converter transforms this into:

```rust
QueryPlan {
    id: "user_todos_active",
    root: Operator::Limit {
        count: 50,
        input: Box::new(Operator::OrderBy {
            specs: vec![OrderSpec { field: "created_at", desc: true }],
            input: Box::new(Operator::Filter {
                predicate: Predicate::And(vec![
                    Predicate::Eq { field: "user_id", value: "$user_id" },
                    Predicate::Eq { field: "completed", value: false }
                ]),
                input: Box::new(Operator::Scan { table: "todos" })
            })
        })
    }
}
```

### 2. Ingest a New Todo

```rust
let record = json!({
    "id": "todos:xyz123",
    "user_id": "user:alice",
    "title": "Buy milk",
    "completed": false,
    "created_at": "2026-01-17T16:35:00Z"
});

// Prepare and ingest
let (spooky_value, hash) = ssp::service::ingest::prepare(record);
let updates = circuit.ingest_record("todos", "CREATE", "todos:xyz123", record, &hash, false);
```

### 3. Receive Delta Update

```json
{
  "format": "streaming",
  "view_id": "user_todos_active",
  "records": [
    { "id": "todos:xyz123", "event": "created", "version": 1 }
  ]
}
```

---

## Output Formats

| Format | Description | Use Case |
|--------|-------------|----------|
| `Flat` | `[(id, version), ...]` with result hash | Simple reconciliation |
| `Streaming` | Delta events (created/updated/deleted) | Real-time UI updates |
| `Tree` | Hierarchical structure (planned) | Nested data display |

---

## Integration with Spooky Sidecar

```mermaid
flowchart LR
    subgraph SDB["SurrealDB"]
        LIVE[LIVE SELECT]
    end
    
    subgraph Sidecar["Spooky Sidecar (Rust)"]
        direction TB
        CDC[CDC Handler]
        SSP_LIB["ssp (library)"]
        WS_OUT[WebSocket Out]
    end
    
    subgraph Client["Client App"]
        WS_IN[WebSocket In]
        STORE[Local Store]
        UI[React/Vue/etc.]
    end
    
    LIVE -->|Change Events| CDC
    CDC --> SSP_LIB
    SSP_LIB -->|ViewUpdate| WS_OUT
    WS_OUT --> WS_IN
    WS_IN --> STORE
    STORE --> UI
    
    style SSP_LIB fill:#00cec9,stroke:#00b894,color:#000
```

The Sidecar uses SSP as a library:

```rust
// In spooky-sidecar
use ssp::{Circuit, StreamProcessor, service};

let mut circuit = Circuit::new();

// On view registration request
let reg_data = service::view::prepare_registration(config)?;
circuit.register_view(reg_data.plan, reg_data.safe_params, Some(format));

// On SurrealDB LIVE event
let (value, hash) = service::ingest::prepare(record);
let updates = circuit.ingest_record(table, op, id, record, &hash, is_optimistic);

// Broadcast updates to clients
for update in updates {
    websocket.send(update).await?;
}
```

---

## Performance Optimizations

| Optimization | Impact |
|--------------|--------|
| **Incremental Delta Evaluation** | Only recomputes affected rows |
| **Dependency Graph** | O(1) view lookup by affected table |
| **FxHash (rustc-hash)** | 2-3x faster internal hashing |
| **SmolStr** | Zero-alloc for short identifiers |
| **Blake3** | Fast cryptographic hashing for record fingerprints |
| **Rayon (optional)** | Parallel batch preparation on native targets |
| **mimalloc** | Optimized allocator for non-WASM builds |

---

## WASM Support

SSP compiles to WebAssembly for browser-based processing:

```toml
[target.'cfg(target_arch = "wasm32")'.dependencies]
getrandom = { version = "0.2", features = ["js"] }
web-sys = { version = "0.3", features = ["console"] }
```

Disable parallel features for WASM:

```bash
cargo build --target wasm32-unknown-unknown --no-default-features
```

---

## Module Structure

```
ssp/
├── src/
│   ├── lib.rs           # Public API: StreamProcessor trait
│   ├── converter.rs     # SurrealQL → Operator tree
│   ├── sanitizer.rs     # Input normalization
│   ├── service.rs       # High-level ingest/view helpers
│   ├── logging.rs       # Debug utilities
│   └── engine/
│       ├── circuit.rs   # Core coordinator
│       ├── view.rs      # View logic & incremental eval
│       ├── update.rs    # Output formatting (Flat/Streaming)
│       ├── operators/   # Operator definitions
│       ├── types/       # SpookyValue, ZSet, FastMap
│       └── eval/        # Predicate evaluation, hashing
└── tests/
    ├── benchmark.rs
    └── ...
```

---

## License

MIT © Spooky Project
