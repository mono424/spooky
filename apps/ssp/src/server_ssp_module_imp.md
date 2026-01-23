# Server Implementation Plan: main.rs Updates

## Overview

Update the server to align with the refactored `circuit.rs`, `view.rs`, and `update.rs` modules, plus add comprehensive observability (traces, spans, logs, metrics).

---

## Table of Contents

1. [Breaking Changes to Address](#1-breaking-changes-to-address)
2. [Code Cleanup](#2-code-cleanup)
3. [Performance Optimizations](#3-performance-optimizations)
4. [Observability Implementation](#4-observability-implementation)
5. [Full Implementation](#5-full-implementation)
6. [Migration Checklist](#6-migration-checklist)

---

## 1. Breaking Changes to Address

### 1.1 Import Changes

```rust
// OLD
use ssp::{
    engine::circuit::{Circuit, IngestBatch, BatchEntry, Operation},
    engine::update::{StreamingUpdate, DeltaEvent, ViewResultFormat, ViewUpdate},
    engine::metadata::VersionStrategy,
};

// NEW
use ssp::{
    engine::circuit::{Circuit, dto::BatchEntry, types::ViewUpdateList},
    engine::types::{Operation, Delta, BatchDeltas},
    engine::update::{StreamingUpdate, DeltaEvent, ViewResultFormat, ViewUpdate},
    // Note: IngestBatch and VersionStrategy may be removed/changed
};
```

### 1.2 API Changes

| Old API | New API | Notes |
|---------|---------|-------|
| `BatchEntry::new(table, op, id, data, hash)` | `BatchEntry::new(table, op, id, data)` | Hash removed |
| `entry.with_version(version)` | TBD - may need custom handling | Version strategy changed |
| `IngestBatch::new()` | Removed | Use `Vec<BatchEntry>` directly |
| `batch.with_strategy(...)` | Removed | Version handled differently |
| `circuit.ingest(batch, true)` | `circuit.ingest_batch(entries)` or `circuit.ingest_single(entry)` | Simplified API |

### 1.3 Ingest Handler Update

```rust
// OLD
let mut entry = BatchEntry::new(
    &payload.table,
    op,
    &payload.id,
    clean_record.into(),
    hash,  // ← REMOVED
);

let mut batch = IngestBatch::new();
if let Some(version) = payload.version {
    entry = entry.with_version(version);
    batch = batch.with_strategy(VersionStrategy::Explicit);
}
circuit.ingest(batch.entry(entry), true)

// NEW
let entry = BatchEntry::new(
    &payload.table,
    op,
    &payload.id,
    clean_record.into(),
);
circuit.ingest_single(entry)  // Returns ViewUpdateList (SmallVec)
```

---

## 2. Code Cleanup

### 2.1 Remove Unused Fields

```rust
// IngestRequest - remove unused fields
#[derive(Deserialize, Debug)]
struct IngestRequest {
    table: String,
    op: String,
    id: String,
    record: Value,
    // pub version: Option<u64>,  // Remove if not used
    // #[serde(default)]
    // _hash: String,             // Remove - hash computed internally
}
```

### 2.2 Simplify State Structure

```rust
// Consider using RwLock for better read concurrency
use tokio::sync::RwLock;

#[derive(Clone)]
struct AppState {
    db: SharedDb,
    circuit: Arc<RwLock<Circuit>>,  // RwLock instead of Mutex
    persistence_path: PathBuf,
    saver: Arc<BackgroundSaver>,
}
```

### 2.3 Extract Handler Logic

```rust
// Move business logic to service layer
mod service {
    pub mod ingest {
        pub fn process_single(...) -> ViewUpdateList { ... }
    }
    pub mod edge {
        pub async fn update_edges(...) { ... }
    }
}
```

---

## 3. Performance Optimizations

### 3.1 Use RwLock for Read-Heavy Operations

```rust
// Most operations are reads (view lookups)
// Only ingest needs write access
let circuit = Arc::new(RwLock::new(Circuit::new()));

// In handlers
async fn register_view_handler(...) {
    let mut circuit = state.circuit.write().await;  // Write lock
    // ...
}

async fn version_handler(...) {
    // No lock needed - doesn't access circuit
}
```

### 3.2 Batch Database Operations

```rust
// Already good! update_all_edges batches multiple updates
// Consider connection pooling if not already done
```

### 3.3 Pre-allocate Vectors

```rust
// When collecting streaming updates
let streaming_updates: Vec<&StreamingUpdate> = updates
    .iter()
    .filter_map(|u| match u {
        ViewUpdate::Streaming(s) => Some(s),
        _ => None,
    })
    .collect();

// Better: with capacity hint
let mut streaming_updates = Vec::with_capacity(updates.len());
for u in &updates {
    if let ViewUpdate::Streaming(s) = u {
        streaming_updates.push(s);
    }
}
```

---

## 4. Observability Implementation

### 4.1 Metrics Setup

```rust
// Add to open_telemetry.rs or new metrics.rs

use opentelemetry::{metrics::MeterProvider, KeyValue};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::metrics::{
    MeterProviderBuilder, PeriodicReader, SdkMeterProvider,
};
use std::time::Duration;

pub struct Metrics {
    pub ingest_counter: opentelemetry::metrics::Counter<u64>,
    pub ingest_duration: opentelemetry::metrics::Histogram<f64>,
    pub view_count: opentelemetry::metrics::UpDownCounter<i64>,
    pub edge_operations: opentelemetry::metrics::Counter<u64>,
    pub cache_size: opentelemetry::metrics::Gauge<u64>,
}

impl Metrics {
    pub fn new(meter_provider: &SdkMeterProvider) -> Self {
        let meter = meter_provider.meter("ssp");
        
        Self {
            ingest_counter: meter
                .u64_counter("ssp.ingest.count")
                .with_description("Number of ingest operations")
                .build(),
            ingest_duration: meter
                .f64_histogram("ssp.ingest.duration_ms")
                .with_description("Ingest operation duration in milliseconds")
                .build(),
            view_count: meter
                .i64_up_down_counter("ssp.views.count")
                .with_description("Number of registered views")
                .build(),
            edge_operations: meter
                .u64_counter("ssp.edges.operations")
                .with_description("Number of edge operations (create/update/delete)")
                .build(),
            cache_size: meter
                .u64_gauge("ssp.cache.size")
                .with_description("Total cache size across all views")
                .build(),
        }
    }
}

pub fn init_metrics() -> Result<(SdkMeterProvider, Metrics), anyhow::Error> {
    let otlp_endpoint = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
        .unwrap_or_else(|_| "http://localhost:18888".to_string());
    
    let exporter = opentelemetry_otlp::MetricExporter::builder()
        .with_tonic()
        .with_endpoint(&otlp_endpoint)
        .build()?;
    
    let reader = PeriodicReader::builder(exporter, opentelemetry_sdk::runtime::Tokio)
        .with_interval(Duration::from_secs(10))
        .build();
    
    let provider = MeterProviderBuilder::default()
        .with_reader(reader)
        .build();
    
    let metrics = Metrics::new(&provider);
    
    Ok((provider, metrics))
}
```

### 4.2 Enhanced Tracing Spans

```rust
use tracing::{info, error, debug, warn, instrument, Span, Level};
use tracing::field::Empty;

// Detailed span for ingest
#[instrument(
    name = "ingest",
    skip(state, payload),
    fields(
        table = %payload.table,
        op = %payload.op,
        id = %payload.id,
        views_affected = Empty,
        edges_updated = Empty,
        duration_ms = Empty,
    )
)]
async fn ingest_handler(
    State(state): State<AppState>,
    Json(payload): Json<IngestRequest>,
) -> impl IntoResponse {
    let start = std::time::Instant::now();
    let span = Span::current();
    
    // ... processing ...
    
    // Record results in span
    span.record("views_affected", updates.len());
    span.record("edges_updated", total_edges);
    span.record("duration_ms", start.elapsed().as_millis() as i64);
    
    // ...
}
```

### 4.3 Structured Logging

```rust
// Use structured fields instead of string formatting

// OLD
debug!("Registering view {}", data.plan.id);

// NEW
debug!(
    view_id = %data.plan.id,
    tables = ?data.plan.root.referenced_tables(),
    format = ?data.format,
    "Registering view"
);

// OLD
error!("Failed to upsert incantation metadata: {}", e);

// NEW
error!(
    error = %e,
    error_type = "database",
    view_id = %id_str,
    "Failed to upsert incantation metadata"
);
```

### 4.4 Custom Span Events

```rust
use tracing::event;

async fn ingest_handler(...) {
    // Start processing
    event!(Level::DEBUG, "Starting ingest processing");
    
    let updates = {
        let _guard = tracing::info_span!("circuit_lock").entered();
        let mut circuit = state.circuit.write().await;
        
        event!(Level::TRACE, "Acquired circuit lock");
        
        let _process_span = tracing::info_span!(
            "process_mutation",
            table = %payload.table,
        ).entered();
        
        circuit.ingest_single(entry)
    };
    
    event!(Level::DEBUG, views_updated = updates.len(), "Mutation processed");
    
    // Edge updates
    if !streaming_updates.is_empty() {
        let _edge_span = tracing::info_span!(
            "update_edges",
            view_count = streaming_updates.len(),
        ).entered();
        
        update_all_edges(&state.db, &streaming_updates).await;
    }
}
```

---

## 5. Full Implementation

### 5.1 Updated main.rs

```rust
use anyhow::Context;
use axum::{
    extract::{State, Json, Request},
    http::{StatusCode, header::AUTHORIZATION},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

// Updated imports for refactored engine
use ssp::{
    engine::circuit::{Circuit, dto::BatchEntry},
    engine::types::Operation,
    engine::update::{StreamingUpdate, DeltaEvent, ViewResultFormat, ViewUpdate},
};

use surrealdb::engine::remote::ws::{Client, Ws};
use surrealdb::opt::auth::Root;
use surrealdb::Surreal;
use surrealdb::types::RecordId;

use tracing::{info, error, debug, warn, instrument, Span, Level};
use tracing::field::Empty;
use tokio::signal;

mod persistence;
mod background_saver;
mod open_telemetry;
mod metrics;

use background_saver::BackgroundSaver;
use metrics::Metrics;

// ============================================================================
// Types
// ============================================================================

type SharedDb = Arc<Surreal<Client>>;

#[derive(Clone)]
struct AppState {
    db: SharedDb,
    circuit: Arc<RwLock<Circuit>>,
    persistence_path: PathBuf,
    saver: Arc<BackgroundSaver>,
    metrics: Arc<Metrics>,
}

#[derive(Deserialize, Debug)]
struct IngestRequest {
    table: String,
    op: String,
    id: String,
    record: Value,
}

#[derive(Deserialize, Debug)]
struct UnregisterViewRequest {
    id: String,
}

// ============================================================================
// Main
// ============================================================================

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    // Initialize observability
    open_telemetry::init_tracing().context("Failed to initialize tracing")?;
    let (meter_provider, metrics) = metrics::init_metrics().context("Failed to initialize metrics")?;
    let metrics = Arc::new(metrics);
    
    info!(version = env!("CARGO_PKG_VERSION"), "Starting SSP server");

    // Configuration
    let config = load_config();
    
    // Database connection
    let db = connect_database(&config).await?;
    
    // Circuit initialization
    let circuit = persistence::load_circuit(&config.persistence_path);
    let circuit = Arc::new(RwLock::new(circuit));
    
    // Record initial metrics
    {
        let c = circuit.read().await;
        metrics.view_count.add(c.views.len() as i64, &[]);
    }

    // Background saver
    let saver = Arc::new(BackgroundSaver::new(
        config.persistence_path.clone(),
        circuit.clone(),
        config.debounce_ms,
    ));
    
    tokio::spawn({
        let saver = saver.clone();
        async move { saver.run().await }
    });

    // App state
    let state = AppState {
        db,
        circuit,
        persistence_path: config.persistence_path,
        saver: saver.clone(),
        metrics,
    };

    // Router
    let app = Router::new()
        .route("/ingest", post(ingest_handler))
        .route("/view/register", post(register_view_handler))
        .route("/view/unregister", post(unregister_view_handler))
        .route("/reset", post(reset_handler))
        .route("/save", post(save_handler))
        .route("/health", get(health_handler))
        .route("/version", get(version_handler))
        .layer(middleware::from_fn(auth_middleware))
        .with_state(state);

    // Server
    let listener = tokio::net::TcpListener::bind(&config.listen_addr)
        .await
        .context("Failed to bind port")?;
    
    info!(addr = %config.listen_addr, "Server listening");
    
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(saver, meter_provider))
        .await
        .context("Server error")?;

    Ok(())
}

// ============================================================================
// Handlers
// ============================================================================

#[instrument(
    name = "ingest",
    skip(state, payload),
    fields(
        table = %payload.table,
        op = %payload.op,
        id = %payload.id,
        views_affected = Empty,
        edges_updated = Empty,
    )
)]
async fn ingest_handler(
    State(state): State<AppState>,
    Json(payload): Json<IngestRequest>,
) -> impl IntoResponse {
    let start = std::time::Instant::now();
    let span = Span::current();
    
    debug!(record = ?payload.record, "Processing ingest request");
    
    // Parse operation
    let op = match Operation::from_str(&payload.op) {
        Some(op) => op,
        None => {
            warn!(op = %payload.op, "Invalid operation type");
            return StatusCode::BAD_REQUEST;
        }
    };

    // Prepare record
    let (clean_record, _hash) = ssp::service::ingest::prepare(payload.record);
    
    // Create entry
    let entry = BatchEntry::new(&payload.table, op, &payload.id, clean_record.into());

    // Process mutation
    let updates = {
        let _lock_span = tracing::debug_span!("acquire_lock").entered();
        let mut circuit = state.circuit.write().await;
        
        let _process_span = tracing::debug_span!("process_mutation").entered();
        circuit.ingest_single(entry)
    };
    
    // Record metrics
    state.metrics.ingest_counter.add(1, &[
        opentelemetry::KeyValue::new("table", payload.table.clone()),
        opentelemetry::KeyValue::new("op", payload.op.clone()),
    ]);
    
    span.record("views_affected", updates.len());
    
    // Trigger persistence
    state.saver.trigger_save();

    // Process edge updates
    let streaming_updates: Vec<&StreamingUpdate> = updates
        .iter()
        .filter_map(|u| match u {
            ViewUpdate::Streaming(s) => Some(s),
            _ => None,
        })
        .collect();

    if !streaming_updates.is_empty() {
        let edge_count = streaming_updates.iter().map(|s| s.records.len()).sum::<usize>();
        span.record("edges_updated", edge_count);
        
        let _edge_span = tracing::debug_span!(
            "update_edges",
            view_count = streaming_updates.len(),
            edge_count = edge_count,
        ).entered();
        
        update_all_edges(&state.db, &streaming_updates, &state.metrics).await;
    }
    
    // Record duration
    let duration_ms = start.elapsed().as_secs_f64() * 1000.0;
    state.metrics.ingest_duration.record(duration_ms, &[]);
    
    debug!(duration_ms = duration_ms, "Ingest completed");
    
    StatusCode::OK
}

#[instrument(
    name = "register_view",
    skip(state, payload),
    fields(view_id = Empty, initial_records = Empty)
)]
async fn register_view_handler(
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
    let span = Span::current();
    
    // Prepare registration data
    let data = match ssp::service::view::prepare_registration(payload) {
        Ok(d) => d,
        Err(e) => {
            warn!(error = %e, "Invalid view registration request");
            return (StatusCode::BAD_REQUEST, e.to_string()).into_response();
        }
    };

    span.record("view_id", &data.plan.id);
    debug!(
        tables = ?data.plan.root.referenced_tables(),
        "Registering view"
    );

    // Register view
    let update = {
        let mut circuit = state.circuit.write().await;
        circuit.register_view(
            data.plan.clone(),
            data.safe_params,
            Some(ViewResultFormat::Streaming),
        )
    };
    
    state.saver.trigger_save();
    state.metrics.view_count.add(1, &[]);

    // Store metadata in SurrealDB
    let id_str = format_incantation_id(&data.metadata["id"].as_str().unwrap());
    
    if let Err(e) = store_view_metadata(&state.db, &id_str, &data.metadata).await {
        error!(error = %e, view_id = %id_str, "Failed to store view metadata");
        return (StatusCode::INTERNAL_SERVER_ERROR, "DB Error").into_response();
    }

    // Create initial edges
    if let Some(ViewUpdate::Streaming(s)) = &update {
        span.record("initial_records", s.records.len());
        debug!(edge_count = s.records.len(), "Creating initial edges");
        update_all_edges(&state.db, &[s], &state.metrics).await;
    }

    info!(view_id = %id_str, "View registered successfully");
    StatusCode::OK.into_response()
}

#[instrument(name = "unregister_view", skip(state), fields(view_id = %payload.id))]
async fn unregister_view_handler(
    State(state): State<AppState>,
    Json(payload): Json<UnregisterViewRequest>,
) -> impl IntoResponse {
    debug!("Unregistering view");
    
    {
        let mut circuit = state.circuit.write().await;
        circuit.unregister_view(&payload.id);
    }
    
    state.saver.trigger_save();
    state.metrics.view_count.add(-1, &[]);
    
    // Delete edges
    let id_str = format_incantation_id(&payload.id);
    if let Some(from_id) = parse_record_id(&id_str) {
        if let Err(e) = state.db
            .query("DELETE $from->_spooky_list_ref")
            .bind(("from", from_id))
            .await
        {
            error!(error = %e, "Failed to delete edges");
        }
    }
    
    info!("View unregistered successfully");
    StatusCode::OK
}

#[instrument(name = "reset", skip(state))]
async fn reset_handler(State(state): State<AppState>) -> impl IntoResponse {
    warn!("Resetting circuit state");
    
    let old_view_count = {
        let mut circuit = state.circuit.write().await;
        let count = circuit.views.len();
        *circuit = Circuit::new();
        count
    };
    
    // Update metrics
    state.metrics.view_count.add(-(old_view_count as i64), &[]);
    
    // Clean up persistence
    if state.persistence_path.exists() {
        let _ = std::fs::remove_file(&state.persistence_path);
    }
    state.saver.trigger_save();
    
    // Delete all edges
    if let Err(e) = state.db.query("DELETE _spooky_list_ref").await {
        error!(error = %e, "Failed to delete edges on reset");
    }
    
    info!("Circuit reset completed");
    StatusCode::OK
}

async fn save_handler(State(state): State<AppState>) -> impl IntoResponse {
    info!("Force saving state");
    state.saver.trigger_save();
    StatusCode::OK
}

async fn health_handler(State(state): State<AppState>) -> impl IntoResponse {
    let circuit = state.circuit.read().await;
    Json(json!({
        "status": "healthy",
        "views": circuit.views.len(),
        "tables": circuit.db.tables.len(),
    }))
}

async fn version_handler() -> impl IntoResponse {
    Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "mode": "streaming"
    }))
}

// ============================================================================
// Middleware
// ============================================================================

async fn auth_middleware(req: Request, next: Next) -> Response {
    let secret = std::env::var("SPOOKY_AUTH_SECRET").unwrap_or_default();
    
    let auth_valid = req.headers()
        .get(AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .map(|h| h == format!("Bearer {}", secret))
        .unwrap_or(false);

    if auth_valid {
        next.run(req).await
    } else {
        debug!("Unauthorized request");
        StatusCode::UNAUTHORIZED.into_response()
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

struct Config {
    persistence_path: PathBuf,
    listen_addr: String,
    debounce_ms: u64,
    db_addr: String,
    db_user: String,
    db_pass: String,
    db_ns: String,
    db_db: String,
}

fn load_config() -> Config {
    Config {
        persistence_path: PathBuf::from(
            std::env::var("SPOOKY_PERSISTENCE_FILE")
                .unwrap_or_else(|_| "data/spooky_state.json".to_string())
        ),
        listen_addr: std::env::var("LISTEN_ADDR")
            .unwrap_or_else(|_| "0.0.0.0:8667".to_string()),
        debounce_ms: std::env::var("SAVE_DEBOUNCE_MS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(2000),
        db_addr: std::env::var("SURREALDB_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:8000".to_string()),
        db_user: std::env::var("SURREALDB_USER")
            .unwrap_or_else(|_| "root".to_string()),
        db_pass: std::env::var("SURREALDB_PASS")
            .unwrap_or_else(|_| "root".to_string()),
        db_ns: std::env::var("SURREALDB_NS")
            .unwrap_or_else(|_| "test".to_string()),
        db_db: std::env::var("SURREALDB_DB")
            .unwrap_or_else(|_| "test".to_string()),
    }
}

#[instrument(name = "connect_db", skip(config))]
async fn connect_database(config: &Config) -> anyhow::Result<SharedDb> {
    info!(addr = %config.db_addr, "Connecting to SurrealDB");
    
    let db = Surreal::new::<Ws>(&config.db_addr)
        .await
        .context("Failed to connect to SurrealDB")?;
    
    db.signin(Root {
        username: &config.db_user,
        password: &config.db_pass,
    })
    .await
    .context("Failed to signin")?;
    
    db.use_ns(&config.db_ns)
        .use_db(&config.db_db)
        .await
        .context("Failed to select ns/db")?;
    
    info!("Connected to SurrealDB");
    Ok(Arc::new(db))
}

async fn store_view_metadata(
    db: &Surreal<Client>,
    id_str: &str,
    metadata: &Value,
) -> Result<(), surrealdb::Error> {
    let query = r#"
        UPSERT <record>$id SET 
            clientId = <string>$clientId, 
            surrealQL = <string>$surrealQL, 
            params = $params, 
            ttl = <duration>$ttl, 
            lastActiveAt = <datetime>$lastActiveAt
    "#;

    db.query(query)
        .bind(("id", id_str.to_string()))
        .bind(("clientId", metadata["clientId"].as_str().unwrap().to_string()))
        .bind(("surrealQL", metadata["surrealQL"].as_str().unwrap().to_string()))
        .bind(("params", metadata["safe_params"].clone()))
        .bind(("ttl", metadata["ttl"].as_str().unwrap().to_string()))
        .bind(("lastActiveAt", metadata["lastActiveAt"].as_str().unwrap().to_string()))
        .await?;

    Ok(())
}

#[instrument(
    name = "update_edges",
    skip(db, updates, metrics),
    fields(total_operations = Empty)
)]
async fn update_all_edges(
    db: &Surreal<Client>,
    updates: &[&StreamingUpdate],
    metrics: &Metrics,
) {
    if updates.is_empty() {
        return;
    }

    let span = Span::current();
    let mut all_statements: Vec<String> = Vec::new();
    let mut bindings: Vec<(String, RecordId)> = Vec::new();
    let mut edge_counts = EdgeCounts::default();

    for (idx, update) in updates.iter().enumerate() {
        if update.records.is_empty() {
            continue;
        }

        let incantation_id_str = format_incantation_id(&update.view_id);
        
        let Some(from_id) = parse_record_id(&incantation_id_str) else {
            error!(id = %incantation_id_str, "Invalid incantation ID format");
            continue;
        };

        let binding_name = format!("from{}", idx);
        bindings.push((binding_name.clone(), from_id));

        for record in &update.records {
            if parse_record_id(&record.id).is_none() {
                error!(id = %record.id, "Invalid record ID format");
                continue;
            }

            let stmt = match record.event {
                DeltaEvent::Created => {
                    edge_counts.created += 1;
                    format!(
                        "RELATE ${}->_spooky_list_ref->{} SET clientId = (SELECT clientId FROM ONLY ${}).clientId",
                        binding_name, record.id, binding_name
                    )
                }
                DeltaEvent::Updated => {
                    edge_counts.updated += 1;
                    format!(
                        "UPDATE ${}->_spooky_list_ref WHERE out = {}",
                        binding_name, record.id
                    )
                }
                DeltaEvent::Deleted => {
                    edge_counts.deleted += 1;
                    format!(
                        "DELETE ${}->_spooky_list_ref WHERE out = {}",
                        binding_name, record.id
                    )
                }
            };
            
            all_statements.push(stmt);
        }
    }

    if all_statements.is_empty() {
        return;
    }

    span.record("total_operations", all_statements.len());
    
    // Record metrics
    metrics.edge_operations.add(edge_counts.created, &[
        opentelemetry::KeyValue::new("operation", "create"),
    ]);
    metrics.edge_operations.add(edge_counts.updated, &[
        opentelemetry::KeyValue::new("operation", "update"),
    ]);
    metrics.edge_operations.add(edge_counts.deleted, &[
        opentelemetry::KeyValue::new("operation", "delete"),
    ]);

    debug!(
        creates = edge_counts.created,
        updates = edge_counts.updated,
        deletes = edge_counts.deleted,
        "Processing edge operations"
    );

    // Execute transaction
    let full_query = format!(
        "BEGIN TRANSACTION;\n{};\nCOMMIT TRANSACTION;",
        all_statements.join(";\n")
    );

    let mut query = db.query(&full_query);
    for (name, id) in bindings {
        query = query.bind((name, id));
    }

    match query.await {
        Ok(_) => {
            debug!("Edge operations completed successfully");
        }
        Err(e) => {
            error!(error = %e, "Batched edge update failed");
        }
    }
}

#[derive(Default)]
struct EdgeCounts {
    created: u64,
    updated: u64,
    deleted: u64,
}

fn parse_record_id(id: &str) -> Option<RecordId> {
    RecordId::parse_simple(id).ok()
}

fn format_incantation_id(id: &str) -> String {
    if id.starts_with("_spooky_incantation:") {
        id.to_string()
    } else {
        format!("_spooky_incantation:{}", id)
    }
}

async fn shutdown_signal(
    saver: Arc<BackgroundSaver>,
    meter_provider: opentelemetry_sdk::metrics::SdkMeterProvider,
) {
    let ctrl_c = async {
        signal::ctrl_c().await.expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("Failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    info!("Shutdown signal received");
    
    // Graceful shutdown sequence
    saver.signal_shutdown();
    
    // Allow time for final saves
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    
    // Shutdown telemetry
    if let Err(e) = meter_provider.shutdown() {
        error!(error = %e, "Failed to shutdown meter provider");
    }
    opentelemetry::global::shutdown_tracer_provider();
    
    info!("Shutdown complete");
}
```

### 5.2 Updated metrics.rs Module

```rust
// metrics.rs
use opentelemetry::{KeyValue, metrics::MeterProvider};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{
    metrics::{MeterProviderBuilder, PeriodicReader, SdkMeterProvider},
    Resource,
};
use std::time::Duration;

pub struct Metrics {
    pub ingest_counter: opentelemetry::metrics::Counter<u64>,
    pub ingest_duration: opentelemetry::metrics::Histogram<f64>,
    pub view_count: opentelemetry::metrics::UpDownCounter<i64>,
    pub edge_operations: opentelemetry::metrics::Counter<u64>,
}

impl Metrics {
    pub fn new(provider: &SdkMeterProvider) -> Self {
        let meter = provider.meter("ssp");
        
        Self {
            ingest_counter: meter
                .u64_counter("ssp_ingest_total")
                .with_description("Total number of ingest operations")
                .build(),
            ingest_duration: meter
                .f64_histogram("ssp_ingest_duration_milliseconds")
                .with_description("Ingest operation duration")
                .build(),
            view_count: meter
                .i64_up_down_counter("ssp_views_active")
                .with_description("Number of active views")
                .build(),
            edge_operations: meter
                .u64_counter("ssp_edge_operations_total")
                .with_description("Total edge operations by type")
                .build(),
        }
    }
}

pub fn init_metrics() -> Result<(SdkMeterProvider, Metrics), anyhow::Error> {
    let endpoint = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
        .unwrap_or_else(|_| "http://localhost:18888".to_string());
    
    let service_name = std::env::var("OTEL_SERVICE_NAME")
        .unwrap_or_else(|_| "ssp".to_string());
    
    let resource = Resource::new(vec![
        KeyValue::new("service.name", service_name),
    ]);
    
    let exporter = opentelemetry_otlp::MetricExporter::builder()
        .with_tonic()
        .with_endpoint(&endpoint)
        .build()?;
    
    let reader = PeriodicReader::builder(exporter, opentelemetry_sdk::runtime::Tokio)
        .with_interval(Duration::from_secs(15))
        .build();
    
    let provider = MeterProviderBuilder::default()
        .with_resource(resource)
        .with_reader(reader)
        .build();
    
    let metrics = Metrics::new(&provider);
    
    Ok((provider, metrics))
}
```

---

## 6. Migration Checklist

### Phase 1: Core Updates (Day 1)
- [ ] Update imports to match refactored engine
- [ ] Remove `IngestBatch` usage
- [ ] Update `BatchEntry::new()` calls (remove hash parameter)
- [ ] Change `circuit.ingest()` to `circuit.ingest_single()`
- [ ] Remove unused `IngestRequest` fields

### Phase 2: Observability (Day 2)
- [ ] Create `metrics.rs` module
- [ ] Update `open_telemetry.rs` for metrics
- [ ] Add `#[instrument]` to all handlers
- [ ] Add structured logging with fields
- [ ] Add span events for key operations

### Phase 3: Code Cleanup (Day 2-3)
- [ ] Extract config into struct
- [ ] Extract database connection logic
- [ ] Consider `RwLock` instead of `Mutex`
- [ ] Add health endpoint
- [ ] Improve error responses

### Phase 4: Testing (Day 3)
- [ ] Test ingest endpoint
- [ ] Test view registration
- [ ] Test edge operations
- [ ] Verify metrics in observability stack
- [ ] Verify traces in Jaeger/similar

---

## 7. Observability Summary

### Traces (Jaeger/Tempo)
- Span: `ingest` with fields: table, op, id, views_affected, edges_updated
- Span: `register_view` with fields: view_id, initial_records
- Span: `update_edges` with fields: total_operations
- Child spans: `acquire_lock`, `process_mutation`

### Logs (Loki)
- Structured JSON with fields
- Levels: ERROR, WARN, INFO, DEBUG, TRACE
- Fields: view_id, table, op, error, duration_ms

### Metrics (Prometheus/Grafana)
- `ssp_ingest_total{table, op}` - Counter
- `ssp_ingest_duration_milliseconds` - Histogram
- `ssp_views_active` - UpDownCounter
- `ssp_edge_operations_total{operation}` - Counter

---

*Document Version: 1.0*
*Estimated Time: 3 days*
*Status: Ready for Implementation*