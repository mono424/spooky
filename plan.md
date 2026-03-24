# Scheduler Module Plan

## Overview

The **Scheduler** is a new central orchestration service that sits between SurrealDB and the SSP sidecars. It replaces the current direct SurrealDB-to-SSP connection model with a hub-and-spoke topology where the Scheduler is the single subscriber to SurrealDB events and the authoritative coordinator for all connected sidecars.

```
                    ┌─────────────────────┐
                    │      SurrealDB      │
                    └─────────┬───────────┘
                              │ LIVE SELECT / Events
                              ▼
                    ┌─────────────────────┐
                    │     Scheduler       │
                    │  ┌───────────────┐  │
                    │  │ DB Replica    │  │
                    │  │ (in-memory)   │  │
                    │  ├───────────────┤  │
                    │  │ Query Router  │  │
                    │  │ (load balance)│  │
                    │  ├───────────────┤  │
                    │  │ Job Scheduler │  │
                    │  └───────────────┘  │
                    └──┬──────┬───────┬───┘
                       │ NATS │       │ NATS
                       ▼      ▼       ▼
                    ┌─────┐┌─────┐┌─────┐
                    │ SSP ││ SSP ││ SSP │
                    │  1  ││  2  ││  N  │
                    └─────┘└─────┘└─────┘
```

## Goals

1. **Single point of DB event consumption** — The Scheduler subscribes to SurrealDB record events (via LIVE SELECT) and fans out updates to sidecars.
2. **Broadcast record updates** — Every record change is broadcast to all connected SSPs so they can update their local DBSP circuits.
3. **Load-balanced query registration** — When clients register queries, the Scheduler distributes them across SSPs to balance memory and CPU.
4. **Job scheduling** — The Scheduler owns job execution scheduling and pushes jobs to a selected SSP for execution.
5. **In-memory DB replica** — On startup, the Scheduler ingests all records from SurrealDB and keeps an in-memory replica for fast bootstrapping of new sidecars.
6. **Sidecar bootstrap** — When a new SSP connects, the Scheduler sends it the full dataset so it can build its DBSP state. *(Future: shared bucket/cache replaces this.)*
7. **Channel-agnostic transport** — Communication is abstracted behind a trait, with NATS as the default implementation leveraging its pub/sub, request/reply, and queue group features.

---

## Architecture

### Component Placement

```
apps/
  scheduler/           # New Rust binary crate
    Cargo.toml
    src/
      main.rs          # Entry point, config loading, startup sequence
      lib.rs           # Core Scheduler struct and lifecycle
      config.rs        # Configuration (env vars, sp00ky.yml)
      replica.rs       # In-memory DB replica (table → id → record)
      router.rs        # Query registration load balancer
      job_scheduler.rs # Job scheduling and dispatch
      transport/
        mod.rs         # Transport trait definition
        nats.rs        # NATS implementation
```

### Transport Trait

The Scheduler communicates with SSPs through a channel-agnostic transport layer.

```rust
#[async_trait]
pub trait Transport: Send + Sync + 'static {
    /// Broadcast a message to all connected SSPs.
    async fn broadcast(&self, subject: &str, payload: &[u8]) -> Result<()>;

    /// Send a message to one SSP (round-robin / least-loaded).
    async fn send_to(&self, ssp_id: &str, subject: &str, payload: &[u8]) -> Result<()>;

    /// Send to one SSP from a queue group (load-balanced).
    async fn queue_send(&self, subject: &str, payload: &[u8]) -> Result<()>;

    /// Request/reply to a specific SSP.
    async fn request(&self, ssp_id: &str, subject: &str, payload: &[u8]) -> Result<Vec<u8>>;

    /// Subscribe to messages from SSPs.
    async fn subscribe(&self, subject: &str) -> Result<Box<dyn Stream<Item = Message>>>;

    /// Track connected SSPs.
    async fn connected_ssps(&self) -> Result<Vec<SspInfo>>;
}
```

### NATS Implementation

The default `NatsTransport` maps naturally to NATS features:

| Scheduler Operation | NATS Feature |
|---|---|
| Broadcast record updates | `PUBLISH sp00ky.ingest.<table>` — all SSPs subscribe |
| Load-balanced query registration | `PUBLISH sp00ky.query.register` — SSPs in a **queue group** so only one receives it |
| Job dispatch | `REQUEST sp00ky.job.execute` — sent to queue group, one SSP picks it up and ACKs |
| Sidecar bootstrap | `REQUEST sp00ky.ssp.<id>.bootstrap` — targeted request/reply with chunked payload |
| SSP health/discovery | SSPs publish to `sp00ky.ssp.heartbeat` — Scheduler tracks connected pool |
| SSP disconnect detection | NATS connection lifecycle events or heartbeat timeout |

**NATS Subjects:**

```
sp00ky.ingest.<table>           # Record updates broadcast
sp00ky.query.register           # Query registration (queue group)
sp00ky.query.unregister         # Query unregistration
sp00ky.job.execute              # Job execution dispatch (queue group)
sp00ky.job.status               # Job status updates from SSPs
sp00ky.ssp.heartbeat            # SSP heartbeat / presence
sp00ky.ssp.<id>.bootstrap       # Targeted bootstrap for specific SSP
sp00ky.ssp.<id>.direct          # Direct messages to specific SSP
```

---

## Startup Sequence

```
1. Load configuration (sp00ky.yml, env vars)
2. Connect to NATS (or configured transport)
3. Connect to SurrealDB
4. Ingest all existing records into in-memory replica
   - For each table in schema: SELECT * FROM <table>
   - Store in HashMap<String, HashMap<RecordId, Value>>
5. Subscribe to SurrealDB LIVE SELECT for all tables
6. Start heartbeat listener for SSP discovery
7. Start job scheduling loop
8. Ready — accept SSP connections
```

## Key Flows

### Flow 1: Record Update Propagation

```
SurrealDB emits LIVE SELECT event (CREATE/UPDATE/DELETE on record)
  → Scheduler receives event
  → Scheduler updates in-memory replica
  → Scheduler broadcasts via transport:
      PUBLISH sp00ky.ingest.<table> { op, id, record }
  → All connected SSPs receive and process through their DBSP circuits
```

### Flow 2: Query Registration (Load-Balanced)

```
Client sends query registration to Scheduler (via SSP or directly)
  → Scheduler selects target SSP using load-balancing strategy:
      - Round-robin (default)
      - Least-queries (fewest registered queries)
      - Least-load (based on heartbeat CPU/memory metrics)
  → Scheduler sends via queue group or targeted:
      PUBLISH sp00ky.query.register { id, sql, params, clientId, ttl }
  → One SSP receives and registers the query in its DBSP circuit
  → Scheduler tracks which SSP owns which query (for rebalancing/failover)
```

### Flow 3: Job Execution

```
Scheduler detects new job record (via LIVE SELECT on job table)
  → Scheduler evaluates scheduling criteria:
      - Immediate execution (default for outbox jobs)
      - Scheduled execution (future: cron-like scheduling)
  → Scheduler dispatches to an SSP via queue group:
      REQUEST sp00ky.job.execute { jobId, path, payload, config }
  → SSP executes the job (HTTP call to backend)
  → SSP replies with result (success/failure)
  → Scheduler updates job record in SurrealDB
```

### Flow 4: New SSP Bootstrap

```
New SSP connects and publishes heartbeat to sp00ky.ssp.heartbeat
  → Scheduler detects new SSP (unknown ssp_id)
  → Scheduler initiates bootstrap:
      1. Pause broadcasting to new SSP (buffer updates)
      2. Stream full replica to SSP in chunks:
         REQUEST sp00ky.ssp.<id>.bootstrap { chunk_index, table, records }
      3. Send buffered updates that arrived during bootstrap
      4. Resume normal broadcasting
  → SSP confirms bootstrap complete
  → SSP enters normal operation, receives broadcasts
```

### Flow 5: SSP Disconnect / Failover

```
SSP heartbeat times out or NATS detects disconnect
  → Scheduler marks SSP as disconnected
  → Scheduler reassigns orphaned queries to remaining SSPs:
      - For each query owned by disconnected SSP:
        PUBLISH sp00ky.query.register (to queue group)
  → Scheduler reassigns in-flight jobs:
      - For each pending job on disconnected SSP:
        Re-dispatch to another SSP via queue group
```

---

## In-Memory Replica

The replica is a lightweight mirror of all SurrealDB records, used for:
- Fast SSP bootstrapping without querying SurrealDB
- Potential future use as a query cache

```rust
pub struct Replica {
    /// table_name → record_id → record_value
    tables: HashMap<String, HashMap<Thing, Value>>,
    /// Tracks the last update timestamp for consistency
    last_updated: Instant,
}

impl Replica {
    /// Full initial load from SurrealDB.
    pub async fn ingest_all(&mut self, db: &Surreal<Client>) -> Result<()>;
    /// Apply a single record event.
    pub fn apply(&mut self, table: &str, op: Op, id: &Thing, record: Option<Value>);
    /// Serialize all records for SSP bootstrap (chunked iterator).
    pub fn iter_chunks(&self, chunk_size: usize) -> impl Iterator<Item = ReplicaChunk>;
}
```

> **Future refactor note:** The bootstrap-via-message flow will be replaced by giving SSPs read access to a shared object store (S3/MinIO bucket) where the Scheduler periodically snapshots the replica. SSPs will pull the snapshot on connect instead of receiving it over the transport.

---

## Load Balancing Strategy

Query registration uses a pluggable strategy:

```rust
pub enum LoadBalanceStrategy {
    RoundRobin,
    LeastQueries,
    LeastLoad,
}
```

The Scheduler maintains an `SspPool`:

```rust
pub struct SspPool {
    ssps: HashMap<String, SspInfo>,
    strategy: LoadBalanceStrategy,
}

pub struct SspInfo {
    pub id: String,
    pub connected_at: Instant,
    pub last_heartbeat: Instant,
    pub query_count: usize,
    pub active_jobs: usize,
    pub cpu_usage: Option<f64>,
    pub memory_usage: Option<f64>,
}
```

---

## Configuration

Extends `sp00ky.yml` with a scheduler section:

```yaml
scheduler:
  transport: nats                          # Transport backend (default: nats)
  nats:
    url: nats://localhost:4222             # NATS server URL
    credentials: /path/to/creds            # Optional NATS credentials
  db:
    url: ws://localhost:8000               # SurrealDB WebSocket URL
    namespace: sp00ky
    database: sp00ky
    username: root
    password: root
  load_balance: least_queries              # round_robin | least_queries | least_load
  heartbeat_interval_ms: 5000             # SSP heartbeat expected interval
  heartbeat_timeout_ms: 15000             # SSP considered dead after this
  bootstrap_chunk_size: 1000              # Records per bootstrap chunk
  job_tables:                              # Tables to watch for job scheduling
    - job
```

Environment variable overrides follow the pattern `SP00KY_SCHEDULER_<KEY>` (e.g., `SP00KY_SCHEDULER_NATS_URL`).

---

## Changes to Existing Components

### SSP Sidecar (`apps/ssp`)

The SSP no longer connects directly to SurrealDB for LIVE SELECT. Instead:

1. **Add NATS client** — Subscribe to Scheduler subjects.
2. **Remove direct LIVE SELECT** — Records arrive via `sp00ky.ingest.<table>` subscription.
3. **Listen on query queue group** — Receive query registrations from Scheduler.
4. **Listen on job queue group** — Receive job dispatch from Scheduler.
5. **Publish heartbeats** — Periodic heartbeat with load metrics.
6. **Bootstrap handler** — Accept and apply replica chunks on connect.

> The SSP keeps its HTTP API for direct client communication (register/unregister views). However, query registration requests should be forwarded to the Scheduler for load balancing rather than handled locally.

### Job Runner (`packages/job-runner`)

The job runner remains within SSP but is now triggered by Scheduler dispatch rather than by direct ingest detection. The `mpsc` channel stays for internal queuing within the SSP; the Scheduler replaces the external trigger.

---

## Implementation Phases

### Phase 1: Foundation
- [ ] Create `apps/scheduler` crate with basic structure
- [ ] Implement `Transport` trait and `NatsTransport`
- [ ] Implement `Replica` with full SurrealDB ingest
- [ ] Implement LIVE SELECT subscription and replica updates
- [ ] Implement broadcast of record updates to NATS

### Phase 2: SSP Integration
- [ ] Add NATS client to SSP sidecar
- [ ] SSP subscribes to `sp00ky.ingest.<table>` for record updates
- [ ] SSP publishes heartbeats
- [ ] Scheduler tracks SSP pool from heartbeats
- [ ] Implement SSP bootstrap flow (full replica transfer on connect)

### Phase 3: Query Load Balancing
- [ ] Implement `SspPool` with load-balancing strategies
- [ ] Scheduler receives query registration requests
- [ ] Scheduler routes queries to SSPs via queue groups
- [ ] Track query-to-SSP ownership for failover

### Phase 4: Job Scheduling
- [ ] Scheduler watches job tables via LIVE SELECT
- [ ] Implement job dispatch to SSPs via queue group
- [ ] SSP replies with job results
- [ ] Scheduler updates job status in SurrealDB
- [ ] Handle job failover on SSP disconnect

### Phase 5: Resilience
- [ ] SSP disconnect detection and query reassignment
- [ ] Job retry on SSP failure
- [ ] Graceful shutdown with state persistence
- [ ] Metrics and observability (OpenTelemetry integration)

---

## Dependencies

New Rust crate dependencies for `apps/scheduler`:

```toml
[dependencies]
tokio = { version = "1", features = ["full"] }
surrealdb = "2.0"
async-nats = "0.37"                  # NATS client
serde = { version = "1", features = ["derive"] }
serde_json = "1"
async-trait = "0.1"
futures = "0.3"
tracing = "0.1"
tracing-subscriber = "0.3"
opentelemetry = "0.22"
```

## Open Questions

- **Client routing:** Should clients talk to the Scheduler directly for query registration, or continue through SSP which forwards to Scheduler?
- **Replica consistency:** How to handle the window between SurrealDB write and Scheduler replica update? Is eventual consistency acceptable?
- **Multi-scheduler HA:** Should we plan for multiple Scheduler instances behind NATS for high availability, or is single-instance acceptable for now?
- **Schema discovery:** Should the Scheduler auto-discover tables from SurrealDB or rely on explicit configuration in `sp00ky.yml`?
