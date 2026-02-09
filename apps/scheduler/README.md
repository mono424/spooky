# Scheduler Service

> **Central Orchestration Hub for Spooky Architecture**

The Scheduler is a new Rust service that acts as the single point of coordination between SurrealDB and SSP (Spooky Sidecar Processor) instances. It replaces the direct SurrealDB-to-SSP connection model with a hub-and-spoke topology, providing centralized query routing, job scheduling, and event distribution.

## Architecture

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

## Key Features

### 🔄 Event Distribution

- **Single SurrealDB subscriber**: Only the Scheduler connects to SurrealDB for LIVE SELECT
- **Broadcast to all SSPs**: Record changes are efficiently distributed via NATS
- **In-memory replica**: Fast SSP bootstrapping without repeated DB queries

### ⚖️ Load Balancing

Three strategies for distributing queries across SSPs:

- **Round Robin**: Simple rotation
- **Least Queries**: Route to SSP with fewest active queries
- **Least Load**: Route based on CPU/memory metrics from heartbeats

### 📋 Job Scheduling

- Watch job tables for new work
- Dispatch jobs to available SSPs via queue groups
- Track job status and handle retries
- Automatic failover on SSP disconnect

### 🔌 Transport Abstraction

Channel-agnostic design with NATS as default:

- Broadcast messages to all SSPs
- Targeted messages to specific SSPs
- Queue-based load balancing
- Request/reply patterns

## NATS Integration

### Subjects

```
spooky.ingest.<table>           # Record update broadcasts
spooky.query.register           # Query registration (queue group)
spooky.query.unregister         # Query unregistration
spooky.job.execute              # Job dispatch (queue group)
spooky.job.status               # Job status updates
spooky.ssp.heartbeat            # SSP presence/health
spooky.ssp.<id>.bootstrap       # Targeted SSP bootstrap
spooky.ssp.<id>.direct          # Direct SSP messages
```

## Configuration

Create a `spooky.yml` file or use environment variables:

```yaml
scheduler:
  transport: nats
  nats:
    url: nats://localhost:4222
    credentials: /path/to/creds # Optional
  db:
    url: ws://localhost:8000
    namespace: spooky
    database: spooky
    username: root
    password: root
  load_balance: least_queries # round_robin | least_queries | least_load
  heartbeat_interval_ms: 5000
  heartbeat_timeout_ms: 15000
  bootstrap_chunk_size: 1000
  job_tables:
    - job
```

Environment variable overrides: `SPOOKY_SCHEDULER_<KEY>` (e.g., `SPOOKY_SCHEDULER_NATS_URL`)

## Usage

### Build

```bash
cargo build --release --bin scheduler
```

### Run

```bash
# With default config
./target/release/scheduler

# With custom config file
SPOOKY_CONFIG=custom.yml ./target/release/scheduler

# With environment overrides
SPOOKY_SCHEDULER_NATS_URL=nats://prod:4222 \
SPOOKY_SCHEDULER_DB_URL=ws://prod-db:8000 \
./target/release/scheduler
```

## Module Structure

```
src/
├── main.rs              # Entry point
├── lib.rs               # Core Scheduler struct
├── config.rs            # Configuration management
├── replica.rs           # In-memory DB replica
├── router.rs            # SSP pool & load balancing
├── job_scheduler.rs     # Job scheduling logic
└── transport/
    ├── mod.rs           # Transport trait
    └── nats.rs          # NATS implementation
```

## Implementation Phases

- ✅ **Phase 1**: Foundation (transport, replica, router) - **COMPLETE**
- 🚧 **Phase 2**: SSP Integration (NATS client, heartbeats, bootstrap)
- 🚧 **Phase 3**: Query Load Balancing (routing, ownership tracking)
- 🚧 **Phase 4**: Job Scheduling (dispatch, status, failover)
- 🚧 **Phase 5**: Resilience (disconnect handling, persistence, observability)

## Dependencies

- `tokio` - Async runtime
- `surrealdb` - Database client
- `async-nats` - NATS messaging
- `serde` / `serde_json` - Serialization
- `config` - Configuration management
- `tracing` - Logging and instrumentation
- `anyhow` - Error handling

## License

Same as the parent Spooky project.
