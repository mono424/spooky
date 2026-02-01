# SSP Multi-Circuit Architecture Brainstorm

## Overview

This document explores the architecture for running multiple DBSP circuits managed by a sidecar service with a unified event queue processed by parallel workers.

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              SIDECAR SERVICE                                 │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                         EVENT QUEUE                                  │    │
│  │  ┌──────────┬──────────┬──────────┬──────────┬──────────┐          │    │
│  │  │ Ingest   │ Register │ Ingest   │ Ingest   │ Unregister│  ...    │    │
│  │  │ user:1   │ view_A   │ thread:5 │ user:2   │ view_B   │          │    │
│  │  └──────────┴──────────┴──────────┴──────────┴──────────┘          │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
│                                    │                                         │
│                    ┌───────────────┼───────────────┐                        │
│                    ▼               ▼               ▼                        │
│              ┌──────────┐   ┌──────────┐   ┌──────────┐                     │
│              │ Worker 1 │   │ Worker 2 │   │ Worker 3 │                     │
│              └────┬─────┘   └────┬─────┘   └────┬─────┘                     │
│                   │              │              │                            │
│         ┌─────────┴──────────────┴──────────────┴─────────┐                 │
│         ▼                        ▼                        ▼                 │
│  ┌─────────────┐         ┌─────────────┐         ┌─────────────┐           │
│  │  Circuit A  │         │  Circuit B  │         │  Circuit C  │           │
│  │  (Tenant 1) │         │  (Tenant 2) │         │ (Analytics) │           │
│  │             │         │             │         │             │           │
│  │ ┌─────────┐ │         │ ┌─────────┐ │         │ ┌─────────┐ │           │
│  │ │ Views   │ │         │ │ Views   │ │         │ │ Views   │ │           │
│  │ └─────────┘ │         │ └─────────┘ │         │ └─────────┘ │           │
│  └─────────────┘         └─────────────┘         └─────────────┘           │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
                    │                                    ▲
                    ▼                                    │
            ┌─────────────┐                      ┌─────────────┐
            │  SurrealDB  │                      │  Frontend   │
            │ LIVE SELECT │ ────────────────────►│  WebSocket  │
            └─────────────┘                      └─────────────┘
```

---

## Part 1: Event Queue Design

### 1.1 Event Types

```rust
#[derive(Clone, Debug)]
pub enum CircuitEvent {
    // --- Data Mutations ---
    Ingest {
        circuit_id: CircuitId,
        entry: BatchEntry,
        /// For ordering guarantees within same record
        sequence: u64,
    },
    
    IngestBatch {
        circuit_id: CircuitId,
        entries: Vec<BatchEntry>,
        /// Treat as atomic unit
        transaction_id: Option<TransactionId>,
    },
    
    // --- View Management ---
    RegisterView {
        circuit_id: CircuitId,
        plan: QueryPlan,
        params: Option<Value>,
        format: Option<ViewResultFormat>,
        /// Channel to send initial snapshot
        response_tx: oneshot::Sender<Option<ViewUpdate>>,
    },
    
    UnregisterView {
        circuit_id: CircuitId,
        view_id: String,
    },
    
    // --- Circuit Lifecycle ---
    CreateCircuit {
        circuit_id: CircuitId,
        config: CircuitConfig,
        response_tx: oneshot::Sender<Result<(), CircuitError>>,
    },
    
    DestroyCircuit {
        circuit_id: CircuitId,
        /// Wait for pending events to drain?
        graceful: bool,
    },
    
    // --- Control ---
    Flush {
        circuit_id: CircuitId,
        response_tx: oneshot::Sender<Vec<ViewUpdate>>,
    },
    
    Snapshot {
        circuit_id: CircuitId,
        response_tx: oneshot::Sender<Vec<u8>>,
    },
    
    // --- Health ---
    Ping {
        response_tx: oneshot::Sender<PongInfo>,
    },
}

pub type CircuitId = SmolStr;
pub type TransactionId = u64;
```

### 1.2 Queue Implementation Options

| Option | Pros | Cons | Use Case |
|--------|------|------|----------|
| `tokio::sync::mpsc` | Simple, no deps, backpressure | Single consumer | Low-medium throughput |
| `crossbeam::channel` | Fast, multi-producer | No async, bounded only | CPU-bound workers |
| `flume` | Async + sync, fast | Extra dependency | Mixed async/sync |
| `async-channel` | Pure async, simple | Slower than crossbeam | Fully async pipeline |
| Custom ring buffer | Zero-copy, predictable | Complex, fixed size | Ultra-low latency |

**Recommendation**: Start with `flume` for flexibility:

```rust
use flume::{Sender, Receiver, bounded};

pub struct EventQueue {
    tx: Sender<CircuitEvent>,
    rx: Receiver<CircuitEvent>,
}

impl EventQueue {
    pub fn new(capacity: usize) -> Self {
        let (tx, rx) = bounded(capacity);
        Self { tx, rx }
    }
    
    /// Clone sender for producers (SurrealDB listeners, HTTP handlers)
    pub fn sender(&self) -> Sender<CircuitEvent> {
        self.tx.clone()
    }
    
    /// Get receiver for worker pool
    pub fn receiver(&self) -> Receiver<CircuitEvent> {
        self.rx.clone()  // flume allows cloning receivers for work-stealing
    }
}
```

### 1.3 Ordering Guarantees

**Critical Question**: What ordering do you need?

| Guarantee Level | Implementation | Performance | Use Case |
|-----------------|----------------|-------------|----------|
| **None** | Workers grab any event | Highest | Independent events |
| **Per-circuit** | Shard queue by circuit_id | High | Multi-tenant isolation |
| **Per-record** | Shard by (circuit_id, table, id) | Medium | Record consistency |
| **Per-table** | Shard by (circuit_id, table) | Medium | Table-level consistency |
| **Global** | Single queue, single consumer | Lowest | Strong consistency |

**Per-circuit ordering** is likely what you want:

```rust
pub struct ShardedEventQueue {
    /// One queue per circuit
    queues: DashMap<CircuitId, Sender<CircuitEvent>>,
    /// Fallback for circuit creation events
    global_queue: Sender<CircuitEvent>,
}

impl ShardedEventQueue {
    pub fn send(&self, event: CircuitEvent) -> Result<(), SendError> {
        match &event {
            CircuitEvent::CreateCircuit { .. } => {
                self.global_queue.send(event)
            }
            _ => {
                let circuit_id = event.circuit_id();
                if let Some(queue) = self.queues.get(circuit_id) {
                    queue.send(event)
                } else {
                    // Circuit doesn't exist - queue to global for error handling
                    self.global_queue.send(event)
                }
            }
        }
    }
}
```

### 1.4 Backpressure Strategies

When queue fills up:

```rust
pub enum BackpressureStrategy {
    /// Block producer until space available
    Block,
    
    /// Return error immediately
    DropWithError,
    
    /// Drop oldest events (lossy)
    DropOldest,
    
    /// Coalesce duplicate operations
    Coalesce {
        /// How long to wait for more events
        window: Duration,
    },
    
    /// Spill to disk
    Spill {
        path: PathBuf,
    },
}

impl EventQueue {
    pub async fn send_with_backpressure(
        &self, 
        event: CircuitEvent,
        strategy: BackpressureStrategy,
    ) -> Result<(), QueueError> {
        match strategy {
            BackpressureStrategy::Block => {
                self.tx.send_async(event).await?;
            }
            BackpressureStrategy::DropWithError => {
                self.tx.try_send(event)?;
            }
            BackpressureStrategy::Coalesce { window } => {
                // Buffer events, coalesce, then send
                self.coalesce_and_send(event, window).await?;
            }
            // ...
        }
        Ok(())
    }
}
```

---

## Part 2: Worker Pool Design

### 2.1 Worker Architecture

```rust
pub struct WorkerPool {
    workers: Vec<JoinHandle<()>>,
    circuits: Arc<DashMap<CircuitId, Arc<RwLock<Circuit>>>>,
    shutdown: CancellationToken,
}

impl WorkerPool {
    pub fn spawn(
        num_workers: usize,
        queue: EventQueue,
        update_tx: broadcast::Sender<ViewUpdate>,
    ) -> Self {
        let circuits = Arc::new(DashMap::new());
        let shutdown = CancellationToken::new();
        
        let workers = (0..num_workers)
            .map(|id| {
                let rx = queue.receiver();
                let circuits = circuits.clone();
                let update_tx = update_tx.clone();
                let shutdown = shutdown.clone();
                
                tokio::spawn(async move {
                    Worker::new(id, rx, circuits, update_tx, shutdown)
                        .run()
                        .await;
                })
            })
            .collect();
        
        Self { workers, circuits, shutdown }
    }
    
    pub async fn shutdown(self) {
        self.shutdown.cancel();
        for worker in self.workers {
            let _ = worker.await;
        }
    }
}

struct Worker {
    id: usize,
    rx: Receiver<CircuitEvent>,
    circuits: Arc<DashMap<CircuitId, Arc<RwLock<Circuit>>>>,
    update_tx: broadcast::Sender<ViewUpdate>,
    shutdown: CancellationToken,
}

impl Worker {
    async fn run(self) {
        loop {
            tokio::select! {
                _ = self.shutdown.cancelled() => {
                    tracing::info!(worker_id = self.id, "Worker shutting down");
                    break;
                }
                
                event = self.rx.recv_async() => {
                    match event {
                        Ok(event) => self.process_event(event).await,
                        Err(_) => break, // Channel closed
                    }
                }
            }
        }
    }
    
    async fn process_event(&self, event: CircuitEvent) {
        let start = Instant::now();
        
        match event {
            CircuitEvent::Ingest { circuit_id, entry, .. } => {
                self.handle_ingest(circuit_id, entry).await;
            }
            CircuitEvent::RegisterView { circuit_id, plan, params, format, response_tx } => {
                self.handle_register_view(circuit_id, plan, params, format, response_tx).await;
            }
            // ... other event handlers
        }
        
        metrics::histogram!("ssp.worker.event_duration", start.elapsed());
    }
    
    async fn handle_ingest(&self, circuit_id: CircuitId, entry: BatchEntry) {
        let Some(circuit_lock) = self.circuits.get(&circuit_id) else {
            tracing::warn!(%circuit_id, "Ingest for unknown circuit");
            return;
        };
        
        // Acquire write lock
        let mut circuit = circuit_lock.write().await;
        let updates = circuit.ingest_single(entry);
        drop(circuit); // Release lock before broadcasting
        
        // Broadcast updates to subscribers
        for update in updates {
            let _ = self.update_tx.send(update);
        }
    }
}
```

### 2.2 Lock Contention Strategies

**Problem**: Multiple workers might try to access the same circuit simultaneously.

| Strategy | Implementation | Pros | Cons |
|----------|----------------|------|------|
| **Coarse RwLock** | `Arc<RwLock<Circuit>>` | Simple | High contention |
| **Actor per circuit** | Each circuit has dedicated task | No locks | More memory, routing overhead |
| **Lock-free queue per circuit** | Circuit has internal queue | Low latency | Complex |
| **Affinity routing** | Worker N handles circuit N | Cache locality | Uneven load |

**Actor-per-circuit** is cleanest for your use case:

```rust
pub struct CircuitActor {
    circuit: Circuit,
    rx: mpsc::Receiver<CircuitCommand>,
    update_tx: broadcast::Sender<ViewUpdate>,
}

pub enum CircuitCommand {
    Ingest(BatchEntry, Option<oneshot::Sender<Vec<ViewUpdate>>>),
    RegisterView(QueryPlan, Option<Value>, oneshot::Sender<Option<ViewUpdate>>),
    UnregisterView(String),
    Flush(oneshot::Sender<Vec<ViewUpdate>>),
    Snapshot(oneshot::Sender<Vec<u8>>),
    Shutdown,
}

impl CircuitActor {
    pub fn spawn(
        circuit_id: CircuitId,
        config: CircuitConfig,
        update_tx: broadcast::Sender<ViewUpdate>,
    ) -> CircuitHandle {
        let (tx, rx) = mpsc::channel(config.queue_size);
        let circuit = Circuit::new();
        
        let actor = Self { circuit, rx, update_tx };
        let handle = tokio::spawn(actor.run());
        
        CircuitHandle { tx, handle, circuit_id }
    }
    
    async fn run(mut self) {
        while let Some(cmd) = self.rx.recv().await {
            match cmd {
                CircuitCommand::Ingest(entry, response) => {
                    let updates = self.circuit.ingest_single(entry);
                    for update in &updates {
                        let _ = self.update_tx.send(update.clone());
                    }
                    if let Some(tx) = response {
                        let _ = tx.send(updates);
                    }
                }
                CircuitCommand::Shutdown => break,
                // ... other commands
            }
        }
    }
}

/// Handle to send commands to a circuit actor
pub struct CircuitHandle {
    tx: mpsc::Sender<CircuitCommand>,
    handle: JoinHandle<()>,
    circuit_id: CircuitId,
}

impl CircuitHandle {
    pub async fn ingest(&self, entry: BatchEntry) -> Result<(), SendError> {
        self.tx.send(CircuitCommand::Ingest(entry, None)).await
    }
    
    pub async fn ingest_sync(&self, entry: BatchEntry) -> Result<Vec<ViewUpdate>, Error> {
        let (tx, rx) = oneshot::channel();
        self.tx.send(CircuitCommand::Ingest(entry, Some(tx))).await?;
        rx.await.map_err(Into::into)
    }
}
```

### 2.3 Worker Affinity

For cache efficiency, route events to consistent workers:

```rust
impl ShardedEventQueue {
    pub fn send_with_affinity(&self, event: CircuitEvent) {
        let circuit_id = event.circuit_id();
        
        // Hash circuit_id to worker index
        let worker_idx = self.hash_to_worker(circuit_id);
        
        // Send to that worker's queue
        self.worker_queues[worker_idx].send(event);
    }
    
    fn hash_to_worker(&self, circuit_id: &CircuitId) -> usize {
        use std::hash::{Hash, Hasher};
        let mut hasher = ahash::AHasher::default();
        circuit_id.hash(&mut hasher);
        (hasher.finish() as usize) % self.worker_queues.len()
    }
}
```

---

## Part 3: Multi-Circuit Management

### 3.1 Circuit Registry

```rust
pub struct CircuitRegistry {
    /// Active circuits
    circuits: DashMap<CircuitId, CircuitHandle>,
    
    /// Circuit configurations
    configs: DashMap<CircuitId, CircuitConfig>,
    
    /// Broadcast channel for view updates
    update_tx: broadcast::Sender<ViewUpdate>,
    
    /// Metrics
    metrics: CircuitMetrics,
}

#[derive(Clone, Debug)]
pub struct CircuitConfig {
    /// Human-readable name
    pub name: String,
    
    /// Maximum views per circuit
    pub max_views: usize,
    
    /// Queue size for circuit actor
    pub queue_size: usize,
    
    /// Tables to pre-create
    pub tables: Vec<String>,
    
    /// Update frequency tier
    pub tier: UpdateTier,
    
    /// Tenant isolation
    pub tenant_id: Option<TenantId>,
}

#[derive(Clone, Debug, Default)]
pub enum UpdateTier {
    /// Process every mutation immediately
    Realtime,
    
    /// Batch mutations, flush every N ms
    #[default]
    Batched { interval_ms: u64 },
    
    /// Only flush on explicit request
    Manual,
}

impl CircuitRegistry {
    pub async fn create_circuit(
        &self,
        circuit_id: CircuitId,
        config: CircuitConfig,
    ) -> Result<(), CircuitError> {
        // Check if already exists
        if self.circuits.contains_key(&circuit_id) {
            return Err(CircuitError::AlreadyExists(circuit_id));
        }
        
        // Spawn actor
        let handle = CircuitActor::spawn(
            circuit_id.clone(),
            config.clone(),
            self.update_tx.clone(),
        );
        
        self.circuits.insert(circuit_id.clone(), handle);
        self.configs.insert(circuit_id, config);
        
        self.metrics.circuits_created.increment(1);
        
        Ok(())
    }
    
    pub async fn destroy_circuit(
        &self,
        circuit_id: &CircuitId,
        graceful: bool,
    ) -> Result<(), CircuitError> {
        let (_, handle) = self.circuits
            .remove(circuit_id)
            .ok_or_else(|| CircuitError::NotFound(circuit_id.clone()))?;
        
        if graceful {
            // Send shutdown command and wait
            let _ = handle.tx.send(CircuitCommand::Shutdown).await;
            let _ = handle.handle.await;
        } else {
            // Abort immediately
            handle.handle.abort();
        }
        
        self.configs.remove(circuit_id);
        self.metrics.circuits_destroyed.increment(1);
        
        Ok(())
    }
    
    pub fn get(&self, circuit_id: &CircuitId) -> Option<CircuitHandle> {
        self.circuits.get(circuit_id).map(|r| r.clone())
    }
    
    pub fn list(&self) -> Vec<CircuitId> {
        self.circuits.iter().map(|r| r.key().clone()).collect()
    }
}
```

### 3.2 Circuit Routing

How does an incoming event know which circuit to target?

```rust
pub struct CircuitRouter {
    registry: Arc<CircuitRegistry>,
    
    /// Routing strategies
    strategies: Vec<Box<dyn RoutingStrategy>>,
    
    /// Default circuit for unrouted events
    default_circuit: Option<CircuitId>,
}

pub trait RoutingStrategy: Send + Sync {
    fn route(&self, event: &CircuitEvent) -> Option<CircuitId>;
}

/// Route by tenant ID in event metadata
pub struct TenantRoutingStrategy;

impl RoutingStrategy for TenantRoutingStrategy {
    fn route(&self, event: &CircuitEvent) -> Option<CircuitId> {
        event.metadata()
            .and_then(|m| m.get("tenant_id"))
            .map(|t| CircuitId::from(format!("tenant:{}", t)))
    }
}

/// Route by table name
pub struct TableRoutingStrategy {
    table_to_circuit: HashMap<String, CircuitId>,
}

impl RoutingStrategy for TableRoutingStrategy {
    fn route(&self, event: &CircuitEvent) -> Option<CircuitId> {
        match event {
            CircuitEvent::Ingest { entry, .. } => {
                self.table_to_circuit.get(entry.table.as_str()).cloned()
            }
            _ => None,
        }
    }
}

impl CircuitRouter {
    pub fn route(&self, event: CircuitEvent) -> Result<CircuitId, RoutingError> {
        // Try each strategy in order
        for strategy in &self.strategies {
            if let Some(circuit_id) = strategy.route(&event) {
                // Verify circuit exists
                if self.registry.circuits.contains_key(&circuit_id) {
                    return Ok(circuit_id);
                }
            }
        }
        
        // Fall back to default
        self.default_circuit
            .clone()
            .ok_or(RoutingError::NoRouteFound)
    }
}
```

### 3.3 Cross-Circuit Queries

**Hard Problem**: What if a view needs data from multiple circuits?

Options:

1. **Disallow**: Views can only reference tables within their circuit
2. **Materialized Federation**: Each circuit maintains a read-only copy of external tables
3. **Query-time Federation**: Scatter-gather across circuits at query time
4. **Shared Base Layer**: Common base tables, circuit-specific derived views

**Option 2 (Materialized Federation)** is probably safest:

```rust
pub struct FederatedCircuit {
    /// Local circuit with full write access
    local: Circuit,
    
    /// Read-only snapshots from other circuits
    foreign_tables: HashMap<CircuitId, HashMap<TableName, ForeignTable>>,
    
    /// Subscriptions to foreign circuit updates
    subscriptions: Vec<ForeignSubscription>,
}

pub struct ForeignTable {
    /// Snapshot of foreign table data
    rows: FastMap<RowKey, SpookyValue>,
    
    /// Last sync timestamp
    last_sync: Instant,
    
    /// Staleness tolerance
    max_staleness: Duration,
}

impl FederatedCircuit {
    /// Sync foreign table from another circuit
    pub async fn sync_foreign_table(
        &mut self,
        source_circuit: &CircuitHandle,
        table_name: &str,
    ) -> Result<(), SyncError> {
        // Request table snapshot from source circuit
        let snapshot = source_circuit.get_table_snapshot(table_name).await?;
        
        // Update local copy
        self.foreign_tables
            .entry(source_circuit.circuit_id.clone())
            .or_default()
            .insert(
                SmolStr::new(table_name),
                ForeignTable {
                    rows: snapshot,
                    last_sync: Instant::now(),
                    max_staleness: Duration::from_secs(5),
                },
            );
        
        Ok(())
    }
}
```

---

## Part 4: Sidecar Service Architecture

### 4.1 Full Sidecar Structure

```rust
pub struct Sidecar {
    /// Circuit management
    registry: Arc<CircuitRegistry>,
    
    /// Event routing
    router: Arc<CircuitRouter>,
    
    /// HTTP/WebSocket server
    server: Server,
    
    /// SurrealDB connection
    db: Surreal<Client>,
    
    /// LIVE SELECT subscriptions
    subscriptions: Vec<LiveSelectHandle>,
    
    /// Metrics & tracing
    metrics: SidecarMetrics,
    
    /// Graceful shutdown
    shutdown: CancellationToken,
}

impl Sidecar {
    pub async fn run(config: SidecarConfig) -> Result<(), SidecarError> {
        // 1. Connect to SurrealDB
        let db = Surreal::new::<Ws>(&config.surreal_url).await?;
        db.use_ns(&config.namespace).use_db(&config.database).await?;
        
        // 2. Initialize circuit registry
        let (update_tx, _) = broadcast::channel(10_000);
        let registry = Arc::new(CircuitRegistry::new(update_tx.clone()));
        
        // 3. Create default circuits
        for circuit_config in &config.circuits {
            registry.create_circuit(
                circuit_config.id.clone(),
                circuit_config.clone(),
            ).await?;
        }
        
        // 4. Set up router
        let router = Arc::new(CircuitRouter::new(registry.clone(), config.routing));
        
        // 5. Start HTTP/WebSocket server
        let server = Server::new(registry.clone(), router.clone(), update_tx.clone());
        
        // 6. Start LIVE SELECT subscriptions
        let subscriptions = Self::setup_live_selects(
            &db,
            &config.watched_tables,
            router.clone(),
        ).await?;
        
        // 7. Run until shutdown
        let shutdown = CancellationToken::new();
        
        tokio::select! {
            _ = server.run() => {}
            _ = shutdown.cancelled() => {}
        }
        
        // 8. Graceful shutdown
        for sub in subscriptions {
            sub.close().await;
        }
        
        Ok(())
    }
    
    async fn setup_live_selects(
        db: &Surreal<Client>,
        tables: &[String],
        router: Arc<CircuitRouter>,
    ) -> Result<Vec<LiveSelectHandle>, Error> {
        let mut handles = Vec::new();
        
        for table in tables {
            let mut stream = db
                .select(Resource::from(table.as_str()))
                .live()
                .await?;
            
            let router = router.clone();
            let table = table.clone();
            
            let handle = tokio::spawn(async move {
                while let Some(notification) = stream.next().await {
                    match notification {
                        Ok(notification) => {
                            let entry = BatchEntry::from_surreal_notification(
                                &table,
                                notification,
                            );
                            
                            // Route to appropriate circuit
                            if let Ok(circuit_id) = router.route_entry(&entry) {
                                if let Some(circuit) = router.registry.get(&circuit_id) {
                                    let _ = circuit.ingest(entry).await;
                                }
                            }
                        }
                        Err(e) => {
                            tracing::error!(table = %table, error = %e, "LIVE SELECT error");
                        }
                    }
                }
            });
            
            handles.push(LiveSelectHandle { handle, table: table.clone() });
        }
        
        Ok(handles)
    }
}
```

### 4.2 HTTP API Design

```rust
// Routes
pub fn routes(state: AppState) -> Router {
    Router::new()
        // Circuit management
        .route("/circuits", get(list_circuits).post(create_circuit))
        .route("/circuits/:id", get(get_circuit).delete(destroy_circuit))
        .route("/circuits/:id/snapshot", get(snapshot_circuit))
        
        // View management
        .route("/circuits/:id/views", get(list_views).post(register_view))
        .route("/circuits/:id/views/:view_id", get(get_view).delete(unregister_view))
        
        // Data ingestion
        .route("/circuits/:id/ingest", post(ingest_single))
        .route("/circuits/:id/ingest/batch", post(ingest_batch))
        .route("/circuits/:id/flush", post(flush_circuit))
        
        // WebSocket for real-time updates
        .route("/ws", get(websocket_handler))
        
        // Health & metrics
        .route("/health", get(health_check))
        .route("/metrics", get(prometheus_metrics))
        
        .with_state(state)
}

// Handlers
async fn ingest_single(
    State(state): State<AppState>,
    Path(circuit_id): Path<CircuitId>,
    Json(entry): Json<BatchEntry>,
) -> Result<Json<Vec<ViewUpdate>>, ApiError> {
    let circuit = state.registry
        .get(&circuit_id)
        .ok_or(ApiError::CircuitNotFound)?;
    
    let updates = circuit.ingest_sync(entry).await?;
    
    Ok(Json(updates))
}

async fn register_view(
    State(state): State<AppState>,
    Path(circuit_id): Path<CircuitId>,
    Json(request): Json<RegisterViewRequest>,
) -> Result<Json<Option<ViewUpdate>>, ApiError> {
    let circuit = state.registry
        .get(&circuit_id)
        .ok_or(ApiError::CircuitNotFound)?;
    
    let initial = circuit.register_view(
        request.plan,
        request.params,
        request.format,
    ).await?;
    
    Ok(Json(initial))
}
```

### 4.3 WebSocket Protocol

```rust
#[derive(Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum WsClientMessage {
    /// Subscribe to view updates
    Subscribe {
        circuit_id: CircuitId,
        view_ids: Vec<String>,
    },
    
    /// Unsubscribe from view updates
    Unsubscribe {
        circuit_id: CircuitId,
        view_ids: Vec<String>,
    },
    
    /// Ingest data (alternative to HTTP)
    Ingest {
        circuit_id: CircuitId,
        entry: BatchEntry,
    },
    
    /// Ping for keepalive
    Ping {
        timestamp: u64,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum WsServerMessage {
    /// View update notification
    ViewUpdate {
        circuit_id: CircuitId,
        update: ViewUpdate,
    },
    
    /// Subscription confirmed
    Subscribed {
        circuit_id: CircuitId,
        view_ids: Vec<String>,
    },
    
    /// Error
    Error {
        code: String,
        message: String,
    },
    
    /// Pong response
    Pong {
        timestamp: u64,
        server_time: u64,
    },
}

async fn websocket_handler(
    State(state): State<AppState>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_websocket(socket, state))
}

async fn handle_websocket(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();
    
    // Subscribe to broadcast channel
    let mut update_rx = state.update_tx.subscribe();
    
    // Track this client's subscriptions
    let subscriptions: Arc<DashSet<(CircuitId, String)>> = Arc::new(DashSet::new());
    
    // Spawn task to forward updates to client
    let subs = subscriptions.clone();
    let send_task = tokio::spawn(async move {
        while let Ok(update) = update_rx.recv().await {
            // Check if client is subscribed to this view
            let key = (update.circuit_id.clone(), update.view_id.clone());
            if subs.contains(&key) {
                let msg = WsServerMessage::ViewUpdate {
                    circuit_id: update.circuit_id,
                    update,
                };
                let json = serde_json::to_string(&msg).unwrap();
                if sender.send(Message::Text(json)).await.is_err() {
                    break;
                }
            }
        }
    });
    
    // Handle incoming messages
    while let Some(msg) = receiver.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                if let Ok(client_msg) = serde_json::from_str::<WsClientMessage>(&text) {
                    match client_msg {
                        WsClientMessage::Subscribe { circuit_id, view_ids } => {
                            for view_id in view_ids {
                                subscriptions.insert((circuit_id.clone(), view_id));
                            }
                        }
                        WsClientMessage::Unsubscribe { circuit_id, view_ids } => {
                            for view_id in view_ids {
                                subscriptions.remove(&(circuit_id.clone(), view_id));
                            }
                        }
                        // ... handle other messages
                    }
                }
            }
            Ok(Message::Close(_)) => break,
            _ => {}
        }
    }
    
    send_task.abort();
}
```

---

## Part 5: Considerations & Gotchas

### 5.1 Memory Management

| Concern | Risk | Mitigation |
|---------|------|------------|
| Circuit accumulation | Memory leak | Max circuit limit, LRU eviction |
| View cache growth | OOM | Per-circuit memory budget |
| Event queue growth | Backpressure failure | Bounded queues, monitoring |
| Foreign table copies | Duplication | Lazy loading, TTL eviction |

```rust
pub struct MemoryBudget {
    /// Max total memory for all circuits
    pub total_limit: usize,
    
    /// Max memory per circuit
    pub per_circuit_limit: usize,
    
    /// Max views per circuit
    pub max_views_per_circuit: usize,
    
    /// Max events in queue
    pub max_queue_size: usize,
}

impl CircuitRegistry {
    pub fn check_memory_budget(&self) -> MemoryReport {
        let mut total = 0;
        let mut per_circuit = HashMap::new();
        
        for entry in self.circuits.iter() {
            let circuit_id = entry.key();
            let handle = entry.value();
            
            // Request memory usage from circuit actor
            let usage = handle.get_memory_usage_sync();
            total += usage;
            per_circuit.insert(circuit_id.clone(), usage);
        }
        
        MemoryReport { total, per_circuit }
    }
    
    pub async fn enforce_memory_budget(&self, budget: &MemoryBudget) {
        let report = self.check_memory_budget();
        
        if report.total > budget.total_limit {
            // Evict least recently used circuits
            self.evict_lru_circuits(report.total - budget.total_limit).await;
        }
    }
}
```

### 5.2 Failure Modes

| Failure | Impact | Recovery |
|---------|--------|----------|
| Worker panic | Circuit events lost | Supervisor restarts worker |
| Circuit actor panic | Circuit state lost | Reload from snapshot |
| Queue overflow | Events dropped | Backpressure, monitoring |
| SurrealDB disconnect | No new events | Reconnect, replay from last sequence |
| WebSocket disconnect | Client misses updates | Client reconnects, requests snapshot |

```rust
/// Supervisor that restarts failed workers
pub struct WorkerSupervisor {
    workers: Vec<(JoinHandle<()>, CancellationToken)>,
    queue: EventQueue,
    circuits: Arc<CircuitRegistry>,
    restart_policy: RestartPolicy,
}

#[derive(Clone)]
pub struct RestartPolicy {
    pub max_restarts: usize,
    pub restart_window: Duration,
    pub backoff: ExponentialBackoff,
}

impl WorkerSupervisor {
    pub async fn run(&mut self) {
        loop {
            // Check for failed workers
            for (i, (handle, token)) in self.workers.iter_mut().enumerate() {
                if handle.is_finished() {
                    match handle.await {
                        Ok(()) => {
                            tracing::info!(worker_id = i, "Worker completed normally");
                        }
                        Err(e) => {
                            tracing::error!(worker_id = i, error = %e, "Worker panicked");
                            
                            // Restart with backoff
                            if self.should_restart(i) {
                                let new_token = CancellationToken::new();
                                let new_handle = self.spawn_worker(i, new_token.clone());
                                *handle = new_handle;
                                *token = new_token;
                            }
                        }
                    }
                }
            }
            
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
}
```

### 5.3 Testing Strategies

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    /// Test event ordering within a circuit
    #[tokio::test]
    async fn test_event_ordering() {
        let sidecar = TestSidecar::new().await;
        
        // Send events that depend on ordering
        let events = vec![
            BatchEntry::create("users", "user:1", json!({"v": 1})),
            BatchEntry::update("users", "user:1", json!({"v": 2})),
            BatchEntry::update("users", "user:1", json!({"v": 3})),
        ];
        
        for entry in events {
            sidecar.ingest("default", entry).await.unwrap();
        }
        
        // Verify final state
        let snapshot = sidecar.get_table_snapshot("default", "users").await;
        assert_eq!(snapshot["user:1"]["v"], 3);
    }
    
    /// Test concurrent circuit access
    #[tokio::test]
    async fn test_concurrent_circuits() {
        let sidecar = TestSidecar::new().await;
        
        // Create multiple circuits
        for i in 0..10 {
            sidecar.create_circuit(format!("circuit:{}", i)).await.unwrap();
        }
        
        // Hammer them concurrently
        let tasks: Vec<_> = (0..100)
            .map(|i| {
                let sidecar = sidecar.clone();
                tokio::spawn(async move {
                    let circuit_id = format!("circuit:{}", i % 10);
                    let entry = BatchEntry::create("test", format!("id:{}", i), json!({}));
                    sidecar.ingest(&circuit_id, entry).await
                })
            })
            .collect();
        
        for task in tasks {
            task.await.unwrap().unwrap();
        }
    }
    
    /// Test circuit isolation
    #[tokio::test]
    async fn test_circuit_isolation() {
        let sidecar = TestSidecar::new().await;
        
        sidecar.create_circuit("circuit:a").await.unwrap();
        sidecar.create_circuit("circuit:b").await.unwrap();
        
        // Insert into A
        sidecar.ingest("circuit:a", BatchEntry::create("users", "user:1", json!({}))).await.unwrap();
        
        // Verify B doesn't see it
        let snapshot_b = sidecar.get_table_snapshot("circuit:b", "users").await;
        assert!(snapshot_b.is_empty());
    }
    
    /// Test worker failure recovery
    #[tokio::test]
    async fn test_worker_panic_recovery() {
        let sidecar = TestSidecar::new().await;
        
        // Inject a panic-inducing event
        sidecar.inject_panic_event().await;
        
        // Wait for supervisor to restart worker
        tokio::time::sleep(Duration::from_secs(2)).await;
        
        // Verify system still works
        let result = sidecar.ingest("default", BatchEntry::create("test", "id:1", json!({}))).await;
        assert!(result.is_ok());
    }
}
```

### 5.4 Monitoring & Observability

```rust
pub struct SidecarMetrics {
    // Circuit metrics
    pub circuits_active: Gauge,
    pub circuits_created_total: Counter,
    pub circuits_destroyed_total: Counter,
    
    // Event metrics
    pub events_received_total: Counter,
    pub events_processed_total: Counter,
    pub events_dropped_total: Counter,
    pub event_processing_duration: Histogram,
    
    // Queue metrics
    pub queue_depth: Gauge,
    pub queue_capacity: Gauge,
    pub backpressure_events: Counter,
    
    // Worker metrics
    pub workers_active: Gauge,
    pub worker_restarts_total: Counter,
    
    // View metrics
    pub views_active: Gauge,
    pub view_updates_total: Counter,
    pub view_evaluation_duration: Histogram,
    
    // WebSocket metrics
    pub websocket_connections: Gauge,
    pub websocket_messages_sent: Counter,
    pub websocket_messages_received: Counter,
}

impl SidecarMetrics {
    pub fn register(registry: &prometheus::Registry) -> Self {
        Self {
            circuits_active: register_gauge!(
                registry,
                "ssp_circuits_active",
                "Number of active circuits"
            ),
            events_received_total: register_counter!(
                registry,
                "ssp_events_received_total",
                "Total events received"
            ),
            // ... etc
        }
    }
}
```

---

## Part 6: Multi-Circuit Ingestion Strategies

This is the critical architectural decision: **When a record arrives, which circuit(s) should receive it?**

### Strategy Overview

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                        INGESTION STRATEGIES                                  │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  Strategy A: BROADCAST                                                       │
│  ┌──────────┐                                                               │
│  │  Record  │───┬──► Circuit A                                              │
│  │  user:1  │   ├──► Circuit B                                              │
│  └──────────┘   └──► Circuit C                                              │
│  Every circuit receives every record                                         │
│                                                                              │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  Strategy B: ROUTED (Pre-computed)                                          │
│  ┌──────────┐     ┌────────┐                                                │
│  │  Record  │────►│ Router │───► Circuit B only                             │
│  │  user:1  │     └────────┘                                                │
│  └──────────┘     (knows user:1 → Circuit B)                                │
│                                                                              │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  Strategy C: DEPENDENCY-BASED (View Analysis)                               │
│  ┌──────────┐     ┌───────────────┐                                         │
│  │  Record  │────►│ Dependency    │───┬──► Circuit A (has view on users)    │
│  │  user:1  │     │ Index         │   └──► Circuit C (has view on users)    │
│  └──────────┘     └───────────────┘                                         │
│  (Circuit B has no views on 'users' table → skip)                           │
│                                                                              │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  Strategy D: META-CIRCUIT (DBSP for routing)                                │
│  ┌──────────┐     ┌─────────────┐     ┌──────────────┐                      │
│  │  Record  │────►│ Meta-Circuit│────►│ Affected     │──► Circuit A         │
│  │  user:1  │     │ (DBSP)      │     │ Circuits     │──► Circuit C         │
│  └──────────┘     └─────────────┘     └──────────────┘                      │
│  Uses DBSP itself to compute routing                                         │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

### Strategy A: Broadcast Ingestion

**Every record goes to every circuit.**

```rust
pub struct BroadcastIngestion {
    circuits: Vec<CircuitHandle>,
}

impl BroadcastIngestion {
    pub async fn ingest(&self, entry: BatchEntry) -> Vec<Result<Vec<ViewUpdate>, Error>> {
        let futures: Vec<_> = self.circuits
            .iter()
            .map(|circuit| circuit.ingest(entry.clone()))
            .collect();
        
        futures::future::join_all(futures).await
    }
}
```

| Pros | Cons |
|------|------|
| ✅ Simplest implementation | ❌ N× redundant storage (every circuit stores every record) |
| ✅ No routing logic needed | ❌ N× redundant computation (every circuit processes every delta) |
| ✅ Views always have data they need | ❌ O(circuits × records) memory |
| ✅ No stale data issues | ❌ Doesn't scale beyond ~5 circuits |
| ✅ Consistent state across circuits | ❌ Wasted work if circuit has no views on that table |

**When to use:**
- Few circuits (≤5)
- All circuits need most tables
- Simplicity > efficiency
- Development/prototyping phase

**When NOT to use:**
- Many circuits (tenant-per-circuit)
- Circuits have disjoint data needs
- Memory/CPU constrained

---

### Strategy B: Routed Ingestion (Pre-computed Affinity)

**Records are tagged with their target circuit(s) at the source.**

```rust
pub struct RoutedIngestion {
    circuits: HashMap<CircuitId, CircuitHandle>,
}

#[derive(Clone)]
pub struct RoutedEntry {
    pub entry: BatchEntry,
    pub target_circuits: SmallVec<[CircuitId; 2]>,
}

impl RoutedIngestion {
    pub async fn ingest(&self, routed: RoutedEntry) -> Vec<ViewUpdate> {
        let mut all_updates = Vec::new();
        
        for circuit_id in &routed.target_circuits {
            if let Some(circuit) = self.circuits.get(circuit_id) {
                match circuit.ingest(routed.entry.clone()).await {
                    Ok(updates) => all_updates.extend(updates),
                    Err(e) => tracing::error!(%circuit_id, %e, "Ingest failed"),
                }
            }
        }
        
        all_updates
    }
}

// Example: Tenant-based routing
impl BatchEntry {
    pub fn with_tenant_routing(mut self, tenant_id: &str) -> RoutedEntry {
        RoutedEntry {
            entry: self,
            target_circuits: smallvec![CircuitId::from(format!("tenant:{}", tenant_id))],
        }
    }
}
```

| Pros | Cons |
|------|------|
| ✅ No redundant processing | ❌ Routing must be determined at source |
| ✅ Scales to many circuits | ❌ Source must know circuit topology |
| ✅ Clear data ownership | ❌ Routing changes require source updates |
| ✅ Easy to reason about | ❌ Cross-circuit queries impossible |

**When to use:**
- Tenant isolation (tenant_id in every record)
- Data has natural partitioning key
- Source system can tag records
- No cross-partition queries needed

**When NOT to use:**
- Records don't have clear affinity
- Views span multiple partitions
- Routing logic is complex/dynamic

---

### Strategy C: Dependency-Based Ingestion (View Analysis)

**Analyze which circuits have views that reference the record's table.**

This is the sweet spot for your use case.

```rust
pub struct DependencyBasedIngestion {
    circuits: HashMap<CircuitId, CircuitHandle>,
    
    /// Global index: table → circuits that have views on it
    table_to_circuits: RwLock<HashMap<TableName, HashSet<CircuitId>>>,
}

impl DependencyBasedIngestion {
    /// Called when a view is registered
    pub fn on_view_registered(&self, circuit_id: &CircuitId, view: &View) {
        let tables = view.plan.root.referenced_tables();
        let mut index = self.table_to_circuits.write().unwrap();
        
        for table in tables {
            index.entry(SmolStr::new(table))
                .or_default()
                .insert(circuit_id.clone());
        }
    }
    
    /// Called when a view is unregistered
    pub fn on_view_unregistered(&self, circuit_id: &CircuitId, view: &View) {
        let tables = view.plan.root.referenced_tables();
        let mut index = self.table_to_circuits.write().unwrap();
        
        for table in tables {
            if let Some(circuits) = index.get_mut(&SmolStr::new(table)) {
                circuits.remove(circuit_id);
                if circuits.is_empty() {
                    index.remove(&SmolStr::new(table));
                }
            }
        }
    }
    
    /// Smart ingestion: only send to circuits that care
    pub async fn ingest(&self, entry: BatchEntry) -> Vec<ViewUpdate> {
        let target_circuits = {
            let index = self.table_to_circuits.read().unwrap();
            index.get(&entry.table)
                .map(|set| set.iter().cloned().collect::<Vec<_>>())
                .unwrap_or_default()
        };
        
        if target_circuits.is_empty() {
            tracing::debug!(table = %entry.table, "No circuits care about this table");
            return Vec::new();
        }
        
        let mut all_updates = Vec::new();
        
        for circuit_id in target_circuits {
            if let Some(circuit) = self.circuits.get(&circuit_id) {
                if let Ok(updates) = circuit.ingest(entry.clone()).await {
                    all_updates.extend(updates);
                }
            }
        }
        
        all_updates
    }
}
```

**Extended: Row-Level Filtering**

Go deeper - analyze view predicates to skip circuits that won't match:

```rust
pub struct PredicateAwareIngestion {
    circuits: HashMap<CircuitId, CircuitHandle>,
    
    /// table → [(circuit_id, view_id, predicate)]
    predicates: RwLock<HashMap<TableName, Vec<(CircuitId, String, CompiledPredicate)>>>,
}

#[derive(Clone)]
pub struct CompiledPredicate {
    /// Fast check if record MIGHT match (may have false positives)
    pub bloom_filter: Option<BloomFilter>,
    
    /// Exact field constraints extracted from WHERE clause
    pub field_constraints: Vec<FieldConstraint>,
}

#[derive(Clone)]
pub enum FieldConstraint {
    Equals { field: String, value: SpookyValue },
    In { field: String, values: HashSet<SpookyValue> },
    Range { field: String, min: Option<SpookyValue>, max: Option<SpookyValue> },
    // Can't optimize: always matches
    Opaque,
}

impl CompiledPredicate {
    /// Fast rejection check - false means definitely won't match
    pub fn might_match(&self, record: &SpookyValue) -> bool {
        for constraint in &self.field_constraints {
            match constraint {
                FieldConstraint::Equals { field, value } => {
                    if let Some(record_value) = record.get(field) {
                        if record_value != value {
                            return false; // Definitely won't match
                        }
                    }
                }
                FieldConstraint::In { field, values } => {
                    if let Some(record_value) = record.get(field) {
                        if !values.contains(record_value) {
                            return false;
                        }
                    }
                }
                FieldConstraint::Opaque => {
                    // Can't determine, assume might match
                }
                // ... other constraints
            }
        }
        true
    }
}

impl PredicateAwareIngestion {
    pub async fn ingest(&self, entry: BatchEntry) -> Vec<ViewUpdate> {
        let candidate_circuits = {
            let predicates = self.predicates.read().unwrap();
            
            predicates.get(&entry.table)
                .map(|views| {
                    views.iter()
                        .filter(|(_, _, pred)| pred.might_match(&entry.data))
                        .map(|(circuit_id, _, _)| circuit_id.clone())
                        .collect::<HashSet<_>>()
                })
                .unwrap_or_default()
        };
        
        // Only ingest to circuits where at least one view might care
        let mut all_updates = Vec::new();
        
        for circuit_id in candidate_circuits {
            if let Some(circuit) = self.circuits.get(&circuit_id) {
                if let Ok(updates) = circuit.ingest(entry.clone()).await {
                    all_updates.extend(updates);
                }
            }
        }
        
        all_updates
    }
}
```

| Pros | Cons |
|------|------|
| ✅ Automatic - no source changes | ⚠️ Index maintenance overhead |
| ✅ Scales with view count | ⚠️ Complex predicate extraction |
| ✅ No redundant processing | ⚠️ May have false positives (safe, not efficient) |
| ✅ Works with dynamic views | ❌ Doesn't help if all circuits have views on same tables |
| ✅ Row-level filtering possible | ❌ Predicate analysis doesn't work for all queries |

**When to use:**
- Circuits have different view sets
- Views have selective predicates (WHERE tenant_id = X)
- Dynamic view registration/unregistration
- Want automatic optimization

**When NOT to use:**
- All circuits have views on all tables
- Predicates are too complex to analyze
- Index maintenance cost > broadcast cost

---

### Strategy D: Meta-Circuit Routing (DBSP for DBSP)

**Use a DBSP circuit to compute which circuits should receive each record.**

This is the most sophisticated approach - using your own technology to solve the routing problem.

```rust
/// A circuit that outputs routing decisions
pub struct MetaCircuit {
    circuit: Circuit,
    
    /// View that outputs: record_key → [circuit_ids]
    routing_view_id: String,
}

impl MetaCircuit {
    pub fn new() -> Self {
        let mut circuit = Circuit::new();
        
        // Register a view that computes routing
        // This view joins:
        // - circuit_views: (circuit_id, view_id, referenced_tables)
        // - view_predicates: (view_id, predicate_info)
        // Output: For each (table, predicate), which circuits care
        
        let routing_plan = QueryPlan {
            id: "routing".into(),
            root: Operator::Project {
                input: Box::new(Operator::Scan { 
                    table: "circuit_views".into(),
                    alias: None,
                }),
                fields: vec!["circuit_id".into(), "table_name".into()],
            },
        };
        
        circuit.register_view(routing_plan, None, None);
        
        Self {
            circuit,
            routing_view_id: "routing".into(),
        }
    }
    
    /// Register a circuit's view dependencies
    pub fn register_circuit_view(
        &mut self,
        circuit_id: &CircuitId,
        view_id: &str,
        referenced_tables: &[String],
    ) {
        for table in referenced_tables {
            self.circuit.ingest_single(BatchEntry::create(
                "circuit_views",
                format!("{}:{}:{}", circuit_id, view_id, table),
                json!({
                    "circuit_id": circuit_id,
                    "view_id": view_id,
                    "table_name": table,
                }).into(),
            ));
        }
    }
    
    /// Query which circuits need a record
    pub fn get_target_circuits(&self, table: &str) -> Vec<CircuitId> {
        // Query the routing view
        // In practice, you'd maintain this as a cached lookup
        self.circuit.views
            .iter()
            .find(|v| v.plan.id == self.routing_view_id)
            .map(|v| {
                v.cache.iter()
                    .filter(|row| row.get("table_name").and_then(|t| t.as_str()) == Some(table))
                    .filter_map(|row| row.get("circuit_id").and_then(|c| c.as_str()))
                    .map(|s| CircuitId::from(s))
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Full system with meta-circuit routing
pub struct MetaRoutedSystem {
    meta: MetaCircuit,
    circuits: HashMap<CircuitId, CircuitHandle>,
    
    /// Cached routing table (updated incrementally by meta-circuit)
    routing_cache: Arc<RwLock<HashMap<TableName, Vec<CircuitId>>>>,
}

impl MetaRoutedSystem {
    pub async fn ingest(&self, entry: BatchEntry) -> Vec<ViewUpdate> {
        // Fast path: check cache
        let targets = {
            let cache = self.routing_cache.read().unwrap();
            cache.get(&entry.table).cloned()
        };
        
        let targets = match targets {
            Some(t) => t,
            None => {
                // Cache miss: query meta-circuit
                let t = self.meta.get_target_circuits(&entry.table);
                
                // Update cache
                let mut cache = self.routing_cache.write().unwrap();
                cache.insert(entry.table.clone(), t.clone());
                
                t
            }
        };
        
        // Route to targets
        let mut updates = Vec::new();
        for circuit_id in targets {
            if let Some(circuit) = self.circuits.get(&circuit_id) {
                if let Ok(u) = circuit.ingest(entry.clone()).await {
                    updates.extend(u);
                }
            }
        }
        
        updates
    }
    
    /// When views change, meta-circuit updates routing
    pub fn on_view_registered(
        &mut self,
        circuit_id: &CircuitId,
        view: &View,
    ) {
        let tables = view.plan.root.referenced_tables();
        self.meta.register_circuit_view(circuit_id, &view.plan.id, &tables);
        
        // Invalidate cache for affected tables
        let mut cache = self.routing_cache.write().unwrap();
        for table in tables {
            cache.remove(&SmolStr::new(table));
        }
    }
}
```

**Advanced: Predicate-Based Meta-Routing**

The meta-circuit can even analyze predicates:

```rust
/// Meta-circuit view that computes fine-grained routing
/// 
/// Tables:
/// - circuit_views(circuit_id, view_id, table_name)
/// - view_predicates(view_id, field, operator, value)
/// 
/// Output view:
/// For each (table, field, value) combination, which circuits care
pub struct PredicateMetaCircuit {
    circuit: Circuit,
}

impl PredicateMetaCircuit {
    /// Register predicate info extracted from a view
    pub fn register_predicate(
        &mut self,
        circuit_id: &CircuitId,
        view_id: &str,
        table: &str,
        field: &str,
        op: &str,
        value: &SpookyValue,
    ) {
        self.circuit.ingest_single(BatchEntry::create(
            "view_predicates",
            format!("{}:{}:{}:{}", circuit_id, view_id, table, field),
            json!({
                "circuit_id": circuit_id,
                "view_id": view_id,
                "table": table,
                "field": field,
                "op": op,
                "value": value,
            }).into(),
        ));
    }
    
    /// Get circuits that might care about this specific record
    pub fn get_targets_for_record(
        &self,
        table: &str,
        record: &SpookyValue,
    ) -> Vec<CircuitId> {
        // Query meta-circuit with record values
        // Returns circuits where predicate MIGHT match
        // (Conservative: may return circuits that won't actually match)
        todo!()
    }
}
```

| Pros | Cons |
|------|------|
| ✅ Self-consistent: uses DBSP for everything | ❌ Most complex implementation |
| ✅ Incremental routing updates | ❌ Extra circuit overhead |
| ✅ Can handle complex routing logic | ❌ Chicken-and-egg bootstrap |
| ✅ Routing is queryable/debuggable | ❌ May be overkill |
| ✅ Elegant conceptual model | ❌ Two-phase commit issues |

**When to use:**
- You want routing logic to be declarative SQL
- Routing rules are complex and change frequently
- You want to debug routing with queries
- You're building a routing abstraction for others

**When NOT to use:**
- Simple routing (tenant-based)
- Performance critical (extra hop)
- Team isn't comfortable with meta-programming

---

### Strategy Comparison Matrix

| Aspect | Broadcast | Routed | Dependency-Based | Meta-Circuit |
|--------|-----------|--------|------------------|--------------|
| **Implementation** | Trivial | Simple | Medium | Complex |
| **Source changes** | None | Required | None | None |
| **Redundant work** | Maximum | None | Minimal | Minimal |
| **Memory overhead** | N× tables | 1× tables | 1× tables + index | 1× + meta overhead |
| **Dynamic views** | ✅ | ❌ | ✅ | ✅ |
| **Row-level filtering** | ❌ | ✅ (if source knows) | ✅ (with predicates) | ✅ |
| **Cross-circuit queries** | Easy | Hard | Medium | Medium |
| **Debugging** | Simple | Simple | Medium | Complex |
| **Best for** | Prototypes, few circuits | Tenant isolation | General use | Complex routing |

---

### Recommended Hybrid Approach

For your SSP sidecar, I recommend **Dependency-Based with Broadcast Fallback**:

```rust
pub struct HybridIngestion {
    circuits: HashMap<CircuitId, CircuitHandle>,
    
    /// Table → circuits with views on it
    dependency_index: RwLock<HashMap<TableName, HashSet<CircuitId>>>,
    
    /// Tables that should always broadcast (shared/global data)
    broadcast_tables: HashSet<TableName>,
    
    /// Fallback: broadcast if no dependency info
    fallback_to_broadcast: bool,
}

impl HybridIngestion {
    pub async fn ingest(&self, entry: BatchEntry) -> Vec<ViewUpdate> {
        // 1. Check if this is a broadcast table
        if self.broadcast_tables.contains(&entry.table) {
            return self.broadcast(entry).await;
        }
        
        // 2. Check dependency index
        let targets = {
            let index = self.dependency_index.read().unwrap();
            index.get(&entry.table).cloned()
        };
        
        match targets {
            Some(circuit_ids) if !circuit_ids.is_empty() => {
                // Route to specific circuits
                self.route_to(entry, circuit_ids).await
            }
            _ if self.fallback_to_broadcast => {
                // No info, broadcast to be safe
                tracing::warn!(
                    table = %entry.table,
                    "No dependency info, falling back to broadcast"
                );
                self.broadcast(entry).await
            }
            _ => {
                // No circuits care
                tracing::debug!(table = %entry.table, "No circuits registered for table");
                Vec::new()
            }
        }
    }
    
    async fn broadcast(&self, entry: BatchEntry) -> Vec<ViewUpdate> {
        let futures: Vec<_> = self.circuits
            .values()
            .map(|c| c.ingest(entry.clone()))
            .collect();
        
        futures::future::join_all(futures)
            .await
            .into_iter()
            .filter_map(|r| r.ok())
            .flatten()
            .collect()
    }
    
    async fn route_to(
        &self,
        entry: BatchEntry,
        circuit_ids: HashSet<CircuitId>,
    ) -> Vec<ViewUpdate> {
        let mut updates = Vec::new();
        
        for circuit_id in circuit_ids {
            if let Some(circuit) = self.circuits.get(&circuit_id) {
                if let Ok(u) = circuit.ingest(entry.clone()).await {
                    updates.extend(u);
                }
            }
        }
        
        updates
    }
}
```

**Configuration:**

```rust
pub struct IngestionConfig {
    /// Tables that always go to all circuits (e.g., "settings", "config")
    pub broadcast_tables: Vec<String>,
    
    /// If true, unknown tables broadcast; if false, they're dropped
    pub fallback_to_broadcast: bool,
    
    /// Enable row-level predicate filtering
    pub enable_predicate_filtering: bool,
    
    /// Max circuits to route to before switching to parallel broadcast
    pub parallel_threshold: usize,
}
```

---

### Decision Flowchart

```
                    ┌─────────────────────────┐
                    │   New record arrives    │
                    └───────────┬─────────────┘
                                │
                    ┌───────────▼─────────────┐
                    │ Is table in broadcast   │
                    │ list?                   │
                    └───────────┬─────────────┘
                          │           │
                         Yes          No
                          │           │
              ┌───────────▼───┐       │
              │   Broadcast   │       │
              │   to ALL      │       │
              │   circuits    │       │
              └───────────────┘       │
                                      │
                    ┌─────────────────▼───────┐
                    │ Check dependency index: │
                    │ which circuits have     │
                    │ views on this table?    │
                    └───────────┬─────────────┘
                          │           │
                      Found         Not found
                          │           │
                          │     ┌─────▼──────────────┐
                          │     │ fallback_to_       │
                          │     │ broadcast enabled? │
                          │     └─────┬──────────────┘
                          │       │         │
                          │      Yes        No
                          │       │         │
                          │       │    ┌────▼────┐
                          │       │    │  Drop   │
                          │       │    │ record  │
                          │       │    └─────────┘
                          │       │
              ┌───────────▼───────▼───┐
              │ Filter by predicate   │
              │ (if enabled)          │
              └───────────┬───────────┘
                          │
              ┌───────────▼───────────┐
              │ Route to matching     │
              │ circuits only         │
              └───────────────────────┘
```

---

---

## Part 7: Circuit Configuration Strategies

Beyond routing, how you **partition views across circuits** matters significantly.

### Configuration A: Single Circuit, All Views

```
┌─────────────────────────────────────────────┐
│              Circuit: default               │
│  ┌─────────────────────────────────────┐   │
│  │ Views:                               │   │
│  │  - user_list                         │   │
│  │  - thread_detail                     │   │
│  │  - comment_feed                      │   │
│  │  - analytics_dashboard               │   │
│  │  - admin_panel                       │   │
│  └─────────────────────────────────────┘   │
│                                             │
│  Tables: users, threads, comments, stats   │
└─────────────────────────────────────────────┘
```

| Pros | Cons |
|------|------|
| ✅ Simplest mental model | ❌ No isolation |
| ✅ Views can share computation | ❌ Slow view blocks fast views |
| ✅ No routing needed | ❌ Single point of failure |
| ✅ Cross-view queries trivial | ❌ Memory grows unbounded |
| ✅ Your current architecture | ❌ Can't scale horizontally |

**Best for:** Small-medium apps, development, single-tenant

---

### Configuration B: Circuit Per Tenant

```
┌─────────────────────┐ ┌─────────────────────┐ ┌─────────────────────┐
│ Circuit: tenant_A   │ │ Circuit: tenant_B   │ │ Circuit: tenant_C   │
│ ┌─────────────────┐ │ │ ┌─────────────────┐ │ │ ┌─────────────────┐ │
│ │ Views:          │ │ │ │ Views:          │ │ │ │ Views:          │ │
│ │  - user_list    │ │ │ │  - user_list    │ │ │ │  - user_list    │ │
│ │  - thread_feed  │ │ │ │  - thread_feed  │ │ │ │  - thread_feed  │ │
│ └─────────────────┘ │ │ └─────────────────┘ │ │ └─────────────────┘ │
│                     │ │                     │ │                     │
│ Tables: (isolated)  │ │ Tables: (isolated)  │ │ Tables: (isolated)  │
└─────────────────────┘ └─────────────────────┘ └─────────────────────┘
```

| Pros | Cons |
|------|------|
| ✅ Complete isolation | ❌ Duplicate view definitions |
| ✅ Tenant A can't affect B | ❌ N× memory for N tenants |
| ✅ Easy per-tenant billing | ❌ Cross-tenant queries impossible |
| ✅ Can scale horizontally | ❌ Circuit creation overhead |
| ✅ Different SLAs per tenant | ❌ Need routing logic |

**Best for:** Multi-tenant SaaS, enterprise isolation requirements

**Implementation considerations:**

```rust
pub struct TenantCircuitManager {
    circuits: HashMap<TenantId, CircuitHandle>,
    
    /// Template for new tenant circuits
    view_templates: Vec<QueryPlan>,
    
    /// Lazy creation vs eager
    creation_strategy: TenantCreationStrategy,
}

pub enum TenantCreationStrategy {
    /// Create circuit when first record arrives
    Lazy,
    
    /// Create circuit when tenant is provisioned
    Eager,
    
    /// Pool of pre-created circuits
    Pooled { pool_size: usize },
}

impl TenantCircuitManager {
    pub async fn ensure_tenant_circuit(&mut self, tenant_id: &TenantId) -> &CircuitHandle {
        if !self.circuits.contains_key(tenant_id) {
            let circuit = self.create_tenant_circuit(tenant_id).await;
            self.circuits.insert(tenant_id.clone(), circuit);
        }
        self.circuits.get(tenant_id).unwrap()
    }
    
    async fn create_tenant_circuit(&self, tenant_id: &TenantId) -> CircuitHandle {
        let circuit_id = CircuitId::from(format!("tenant:{}", tenant_id));
        let handle = CircuitActor::spawn(circuit_id, CircuitConfig::default());
        
        // Register all view templates for this tenant
        for template in &self.view_templates {
            let plan = template.clone().with_tenant_param(tenant_id);
            handle.register_view(plan, None).await.ok();
        }
        
        handle
    }
}
```

---

### Configuration C: Circuit Per Update Frequency

```
┌─────────────────────────────────────────────────────────────────────────┐
│                                                                          │
│  ┌─────────────────────┐                                                │
│  │ Circuit: realtime   │  step() on EVERY mutation                      │
│  │ ┌─────────────────┐ │  Latency: <10ms                                │
│  │ │ - cursor_pos    │ │                                                │
│  │ │ - typing_ind    │ │                                                │
│  │ │ - presence      │ │                                                │
│  │ └─────────────────┘ │                                                │
│  └─────────────────────┘                                                │
│                                                                          │
│  ┌─────────────────────┐                                                │
│  │ Circuit: normal     │  step() every 100ms OR 50 mutations            │
│  │ ┌─────────────────┐ │  Latency: <200ms                               │
│  │ │ - message_feed  │ │                                                │
│  │ │ - thread_list   │ │                                                │
│  │ │ - user_profile  │ │                                                │
│  │ └─────────────────┘ │                                                │
│  └─────────────────────┘                                                │
│                                                                          │
│  ┌─────────────────────┐                                                │
│  │ Circuit: analytics  │  step() every 5 seconds                        │
│  │ ┌─────────────────┐ │  Latency: <10s                                 │
│  │ │ - daily_stats   │ │                                                │
│  │ │ - user_metrics  │ │                                                │
│  │ │ - trend_report  │ │                                                │
│  │ └─────────────────┘ │                                                │
│  └─────────────────────┘                                                │
│                                                                          │
└─────────────────────────────────────────────────────────────────────────┘
```

| Pros | Cons |
|------|------|
| ✅ Optimized for different SLAs | ⚠️ Same record may need multiple circuits |
| ✅ Analytics don't slow realtime | ⚠️ Need broadcast or smart routing |
| ✅ Can batch expensive views | ❌ More complex than single circuit |
| ✅ Memory efficient (shared tables) | ❌ Cross-tier view dependencies tricky |

**Best for:** Mixed workloads, apps with both realtime and analytics needs

**Implementation:**

```rust
pub struct FrequencyTieredCircuits {
    realtime: CircuitHandle,
    normal: CircuitHandle,
    analytics: CircuitHandle,
    
    /// Deferred batches for non-realtime circuits
    normal_batch: Mutex<Vec<BatchEntry>>,
    analytics_batch: Mutex<Vec<BatchEntry>>,
    
    /// Flush intervals
    normal_interval: Duration,
    analytics_interval: Duration,
}

impl FrequencyTieredCircuits {
    pub async fn ingest(&self, entry: BatchEntry, tier: UpdateTier) {
        match tier {
            UpdateTier::Realtime => {
                // Immediate processing
                self.realtime.ingest(entry).await.ok();
            }
            UpdateTier::Normal => {
                // Queue for batch
                self.normal_batch.lock().unwrap().push(entry);
            }
            UpdateTier::Analytics => {
                // Queue for batch
                self.analytics_batch.lock().unwrap().push(entry);
            }
        }
    }
    
    /// Called by timer task
    pub async fn flush_normal(&self) {
        let batch = {
            let mut guard = self.normal_batch.lock().unwrap();
            std::mem::take(&mut *guard)
        };
        
        if !batch.is_empty() {
            self.normal.ingest_batch(batch).await.ok();
        }
    }
    
    /// Called by timer task
    pub async fn flush_analytics(&self) {
        let batch = {
            let mut guard = self.analytics_batch.lock().unwrap();
            std::mem::take(&mut *guard)
        };
        
        if !batch.is_empty() {
            self.analytics.ingest_batch(batch).await.ok();
        }
    }
}

/// Background task for timed flushes
async fn flush_task(circuits: Arc<FrequencyTieredCircuits>) {
    let mut normal_interval = tokio::time::interval(circuits.normal_interval);
    let mut analytics_interval = tokio::time::interval(circuits.analytics_interval);
    
    loop {
        tokio::select! {
            _ = normal_interval.tick() => {
                circuits.flush_normal().await;
            }
            _ = analytics_interval.tick() => {
                circuits.flush_analytics().await;
            }
        }
    }
}
```

---

### Configuration D: Circuit Per View (Extreme Isolation)

```
┌──────────────┐ ┌──────────────┐ ┌──────────────┐ ┌──────────────┐
│Circuit:view_1│ │Circuit:view_2│ │Circuit:view_3│ │Circuit:view_4│
│ ┌──────────┐ │ │ ┌──────────┐ │ │ ┌──────────┐ │ │ ┌──────────┐ │
│ │user_list │ │ │ │thread_   │ │ │ │comment_  │ │ │ │analytics │ │
│ │          │ │ │ │detail    │ │ │ │feed      │ │ │ │dashboard │ │
│ └──────────┘ │ │ └──────────┘ │ │ └──────────┘ │ │ └──────────┘ │
└──────────────┘ └──────────────┘ └──────────────┘ └──────────────┘
```

| Pros | Cons |
|------|------|
| ✅ Maximum isolation | ❌ N× table duplication |
| ✅ Views can't affect each other | ❌ No shared computation |
| ✅ Easy to reason about | ❌ Memory explosion |
| ✅ Simple failure domains | ❌ Expensive cross-view queries |

**Best for:** Almost never. Only if views have completely disjoint data needs and zero shared computation.

---

### Configuration E: Hybrid (Recommended)

Combine strategies based on actual requirements:

```
┌─────────────────────────────────────────────────────────────────────────┐
│                                                                          │
│  ┌─────────────────────────────────┐  ← Shared base tables              │
│  │      Circuit: shared_base       │    (users, config, settings)       │
│  │  ┌───────────────────────────┐  │                                    │
│  │  │ Base views only:          │  │                                    │
│  │  │  - active_users           │  │                                    │
│  │  │  - global_settings        │  │                                    │
│  │  └───────────────────────────┘  │                                    │
│  └─────────────────────────────────┘                                    │
│                    │                                                     │
│          ┌────────┴────────┐                                            │
│          ▼                 ▼                                            │
│  ┌───────────────┐ ┌───────────────┐  ← Tenant-specific circuits        │
│  │Circuit:tenant_A│ │Circuit:tenant_B│   (inherit from shared_base)     │
│  │ ┌───────────┐ │ │ ┌───────────┐ │                                    │
│  │ │- my_feed  │ │ │ │- my_feed  │ │                                    │
│  │ │- my_stats │ │ │ │- my_stats │ │                                    │
│  │ └───────────┘ │ │ └───────────┘ │                                    │
│  └───────────────┘ └───────────────┘                                    │
│                                                                          │
│  ┌─────────────────────────────────┐  ← Global analytics (batched)      │
│  │    Circuit: analytics           │                                    │
│  │  ┌───────────────────────────┐  │                                    │
│  │  │  - platform_stats         │  │                                    │
│  │  │  - trend_analysis         │  │                                    │
│  │  └───────────────────────────┘  │                                    │
│  └─────────────────────────────────┘                                    │
│                                                                          │
└─────────────────────────────────────────────────────────────────────────┘
```

**Implementation:**

```rust
pub struct HybridCircuitSystem {
    /// Shared base circuit (always receives all records)
    shared_base: CircuitHandle,
    
    /// Per-tenant circuits (receive tenant-specific records)
    tenant_circuits: HashMap<TenantId, CircuitHandle>,
    
    /// Analytics circuit (receives all records, batched)
    analytics: CircuitHandle,
    analytics_batch: Mutex<Vec<BatchEntry>>,
    
    /// Configuration
    config: HybridConfig,
}

pub struct HybridConfig {
    /// Tables that go to shared_base only
    pub shared_tables: HashSet<TableName>,
    
    /// Tables that go to tenant circuits
    pub tenant_tables: HashSet<TableName>,
    
    /// Tables that go to analytics (can overlap)
    pub analytics_tables: HashSet<TableName>,
    
    /// How to extract tenant from record
    pub tenant_extractor: Box<dyn Fn(&BatchEntry) -> Option<TenantId> + Send + Sync>,
}

impl HybridCircuitSystem {
    pub async fn ingest(&self, entry: BatchEntry) -> Vec<ViewUpdate> {
        let mut all_updates = Vec::new();
        
        // 1. Shared base (if applicable)
        if self.config.shared_tables.contains(&entry.table) {
            if let Ok(updates) = self.shared_base.ingest(entry.clone()).await {
                all_updates.extend(updates);
            }
        }
        
        // 2. Tenant circuit (if applicable)
        if self.config.tenant_tables.contains(&entry.table) {
            if let Some(tenant_id) = (self.config.tenant_extractor)(&entry) {
                if let Some(circuit) = self.tenant_circuits.get(&tenant_id) {
                    if let Ok(updates) = circuit.ingest(entry.clone()).await {
                        all_updates.extend(updates);
                    }
                }
            }
        }
        
        // 3. Analytics (batched)
        if self.config.analytics_tables.contains(&entry.table) {
            self.analytics_batch.lock().unwrap().push(entry);
        }
        
        all_updates
    }
}
```

---

### Configuration Comparison

| Configuration | Memory | Latency | Isolation | Complexity | Use Case |
|---------------|--------|---------|-----------|------------|----------|
| Single Circuit | 1× | Uniform | None | Lowest | Dev, small apps |
| Per Tenant | N× | Uniform | Complete | Medium | Multi-tenant SaaS |
| Per Frequency | 1× | Varied | By tier | Medium | Mixed workloads |
| Per View | N× | Best | Complete | High | Never (usually) |
| Hybrid | 1-2× | Optimized | Flexible | Highest | Production systems |

---

## Part 8: Implementation Phases

### Phase 1: Foundation (Week 1-2)

- [ ] Event queue with `flume`
- [ ] Single worker processing events
- [ ] Circuit registry (single circuit)
- [ ] Basic HTTP API for ingest/register
- [ ] **Dependency index for routing** ← Start here

### Phase 2: Parallelism (Week 3-4)

- [ ] Worker pool with configurable size
- [ ] Circuit actor pattern
- [ ] Per-circuit event routing
- [ ] Lock-free event dispatch

### Phase 3: Multi-Circuit (Week 5-6)

- [ ] Multiple circuit support
- [ ] Circuit creation/destruction API
- [ ] Routing strategies (dependency-based)
- [ ] Circuit isolation tests

### Phase 4: Production Readiness (Week 7-8)

- [ ] WebSocket real-time updates
- [ ] Metrics & monitoring
- [ ] Graceful shutdown
- [ ] Snapshot/restore
- [ ] Integration tests

### Phase 5: Advanced (As Needed)

- [ ] Frequency-tiered circuits
- [ ] Predicate-based filtering
- [ ] Cross-circuit queries (if needed)
- [ ] Tenant isolation (if needed)

---

## Part 9: Summary & Recommendations

### Key Decisions

| Decision | Recommendation | Rationale |
|----------|----------------|-----------|
| **Ingestion strategy** | Dependency-based | Automatic, no source changes, scales with views |
| **Circuit configuration** | Start single, add tiers if needed | Don't over-engineer early |
| **Worker model** | Actor-per-circuit | No locks, natural isolation |
| **Queue** | `flume` with per-circuit sharding | Balance simplicity and performance |
| **Cross-circuit** | Avoid, or materialized federation | Keep circuits independent |

### Start Here

1. **Keep your single circuit** for now
2. **Add dependency index** to track table→view mappings (you already have `dependency_list`)
3. **Wrap in actor** with command mailbox
4. **Add event queue** in front of actor
5. **Only then** consider multiple circuits

### Don't Do (Yet)

- ❌ Meta-circuit routing (overkill)
- ❌ Circuit-per-view (memory explosion)
- ❌ Complex predicate extraction (premature optimization)
- ❌ Cross-circuit JOINs (architectural complexity)

### Architecture Evolution Path

```
Current                    Phase 1                     Phase 2
───────                    ───────                     ───────
                           
┌──────────┐              ┌──────────────┐            ┌──────────────┐
│  Direct  │              │ Event Queue  │            │ Event Queue  │
│  Calls   │      →       │      │       │     →      │      │       │
└────┬─────┘              │      ▼       │            │      ▼       │
     │                    │ ┌──────────┐ │            │ ┌─────────┐  │
     ▼                    │ │  Single  │ │            │ │ Router  │  │
┌──────────┐              │ │  Circuit │ │            │ └────┬────┘  │
│ Circuit  │              │ │  Actor   │ │            │      │       │
└──────────┘              │ └──────────┘ │            │ ┌────┴────┐  │
                          └──────────────┘            │ ▼         ▼  │
                                                      │┌────┐ ┌────┐│
                                                      ││ C1 │ │ C2 ││
                                                      │└────┘ └────┘│
                                                      └─────────────┘
```

### Questions to Answer Before Multi-Circuit

1. **Do you need tenant isolation?** → Per-tenant circuits
2. **Do you have mixed latency requirements?** → Frequency-tiered
3. **Are views on disjoint tables?** → Maybe worth splitting
4. **Can a single circuit handle your load?** → Probably yes, benchmark first

If you answered "no" to all of these, **stay with single circuit**.


---

## Summary

| Component | Recommendation | Complexity |
|-----------|----------------|------------|
| Event Queue | `flume` with per-circuit sharding | Low |
| Workers | Actor-per-circuit pattern | Medium |
| Lock Strategy | No locks - actor mailbox | Low |
| Circuit Routing | Strategy pattern, tenant-aware | Medium |
| Cross-Circuit | Materialized federation (if needed) | High |
| Failure Recovery | Supervisor + snapshots | Medium |

The key insight: **Actor-per-circuit** eliminates most synchronization complexity. Each circuit is owned by exactly one async task, receiving commands through a channel. This gives you:

- No locks
- Natural backpressure
- Easy reasoning about state
- Simple failure isolation

Start with this pattern and only add complexity (work-stealing, sharding, federation) when you have concrete evidence it's needed.
