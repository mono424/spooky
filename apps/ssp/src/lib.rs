use anyhow::Context;
use axum::{
    Router,
    extract::{Json, Path, Request, State},
    http::{StatusCode, header::AUTHORIZATION},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

use ssp::{
    engine::circuit::{Circuit, dto::BatchEntry},
    engine::types::Operation,
    engine::update::{DeltaEvent, StreamingUpdate, ViewResultFormat, ViewUpdate},
};
use surrealdb::engine::remote::ws::{Client, Ws};
use surrealdb::opt::auth::Root;
use surrealdb::types::RecordId;
use surrealdb::{Connection, Surreal};
use tokio::signal;
use tracing::field::Empty;
use tracing::{Span, debug, error, info, instrument, warn};

// Expose modules for use in main.rs and tests
pub mod background_saver;
pub mod metrics;
pub mod open_telemetry;
pub mod persistence;

use background_saver::BackgroundSaver;
use metrics::Metrics;

/// Shared database connection wrapped in Arc for zero-copy sharing across tasks
pub type SharedDb = Arc<Surreal<Client>>;

#[derive(Clone)]
pub struct AppState {
    pub db: SharedDb,
    pub processor: Arc<RwLock<Circuit>>,
    pub persistence_path: PathBuf,
    pub saver: Arc<BackgroundSaver>,
    pub metrics: Arc<Metrics>,
}

// --- Request/Response DTOs ---

#[derive(Deserialize, Debug)]
pub struct LogRequest {
    message: String,
    #[serde(default)]
    level: String,
    #[serde(default)]
    data: Option<Value>,
}

#[derive(Deserialize, Debug)]
pub struct IngestRequest {
    pub table: String,
    pub op: String,
    pub id: String,
    pub record: Value,
}

#[derive(Deserialize, Debug)]
pub struct UnregisterViewRequest {
    pub id: String,
}

// --- Configuration ---

pub struct Config {
    pub persistence_path: PathBuf,
    pub listen_addr: String,
    pub debounce_ms: u64,
    pub db_addr: String,
    pub db_user: String,
    pub db_pass: String,
    pub db_ns: String,
    pub db_db: String,
}

pub fn load_config() -> Config {
    Config {
        persistence_path: PathBuf::from(
            std::env::var("SPOOKY_PERSISTENCE_FILE")
                .unwrap_or_else(|_| "data/spooky_state.json".to_string()),
        ),
        listen_addr: std::env::var("LISTEN_ADDR").unwrap_or_else(|_| "0.0.0.0:8667".to_string()),
        debounce_ms: std::env::var("SAVE_DEBOUNCE_MS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(2000),
        db_addr: std::env::var("SURREALDB_ADDR").unwrap_or_else(|_| "127.0.0.1:8000".to_string()),
        db_user: std::env::var("SURREALDB_USER").unwrap_or_else(|_| "root".to_string()),
        db_pass: std::env::var("SURREALDB_PASS").unwrap_or_else(|_| "root".to_string()),
        db_ns: std::env::var("SURREALDB_NS").unwrap_or_else(|_| "test".to_string()),
        db_db: std::env::var("SURREALDB_DB").unwrap_or_else(|_| "test".to_string()),
    }
}

// --- Database Connection ---

pub async fn connect_database(config: &Config) -> anyhow::Result<SharedDb> {
    info!(addr = %config.db_addr, "Connecting to SurrealDB");

    let db = Surreal::new::<Ws>(&config.db_addr)
        .await
        .context("Failed to connect to SurrealDB")?;

    db.signin(Root {
        username: config.db_user.clone(),
        password: config.db_pass.clone(),
    })
    .await
    .context("Failed to signin")?;

    db.use_ns(&config.db_ns)
        .use_db(&config.db_db)
        .await
        .context("Failed to select namespace/database")?;

    info!("Connected to SurrealDB successfully");
    Ok(Arc::new(db))
}

// --- Router Setup ---

pub fn create_app(state: AppState) -> Router {
    Router::new()
        .route("/ingest", post(ingest_handler))
        .route("/log", post(log_handler))
        .route("/debug/view/:view_id", get(debug_view_handler))
        .route("/view/register", post(register_view_handler))
        .route("/view/unregister", post(unregister_view_handler))
        .route("/reset", post(reset_handler))
        .route("/save", post(save_handler))
        .route("/health", get(health_handler))
        .route("/version", get(version_handler))
        .layer(middleware::from_fn(auth_middleware))
        .with_state(state)
}

// --- Server Lifecycle ---

pub async fn run_server() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    // Initialize observability
    open_telemetry::init_tracing().context("Failed to initialize OpenTelemetry tracing")?;
    let (meter_provider, metrics) =
        metrics::init_metrics().context("Failed to initialize metrics")?;
    let metrics = Arc::new(metrics);

    info!("Starting SSP sidecar (streaming mode)...");

    let config = load_config();
    let db = connect_database(&config).await?;

    // Load or initialize Circuit
    let processor = persistence::load_circuit(&config.persistence_path);
    let processor_arc = Arc::new(RwLock::new(processor));

    // Record initial view count metric
    {
        let guard = processor_arc.read().await;
        metrics.view_count.add(guard.views.len() as i64, &[]);
    }

    // Initialize background saver
    let saver = Arc::new(BackgroundSaver::new(
        config.persistence_path.clone(),
        processor_arc.clone(),
        config.debounce_ms,
    ));

    let saver_clone = saver.clone();
    tokio::spawn(async move {
        saver_clone.run().await;
    });

    let state = AppState {
        db,
        processor: processor_arc,
        persistence_path: config.persistence_path,
        saver: saver.clone(),
        metrics,
    };

    let app = create_app(state);

    let listener = tokio::net::TcpListener::bind(&config.listen_addr)
        .await
        .context("Failed to bind port")?;

    info!(addr = %config.listen_addr, "Listening for requests");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(saver, meter_provider))
        .await
        .context("Server error")?;

    opentelemetry::global::shutdown_tracer_provider();

    Ok(())
}

async fn shutdown_signal(
    saver: Arc<BackgroundSaver>,
    meter_provider: opentelemetry_sdk::metrics::SdkMeterProvider,
) {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
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

    info!("Signal received, starting graceful shutdown");
    saver.signal_shutdown();
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    if let Err(e) = meter_provider.shutdown() {
        error!(error = %e, "Failed to shutdown meter provider");
    }
}

// --- Middleware ---

async fn auth_middleware(req: Request, next: Next) -> Response {
    let auth_header = req.headers().get(AUTHORIZATION);
    let secret = std::env::var("SPOOKY_AUTH_SECRET").unwrap_or_default();

    match auth_header {
        Some(header) if header.to_str().unwrap_or_default() == format!("Bearer {}", secret) => {
            next.run(req).await
        }
        _ => StatusCode::UNAUTHORIZED.into_response(),
    }
}

// --- Request Handlers ---

/// Ingest handler - processes single record updates and propagates to affected views
#[instrument(
    skip(state, body),
    fields(
        table = Empty,
        op = Empty,
        id = Empty,
        payload_size_bytes = Empty,
        views_affected = Empty,
        edges_updated = Empty,
    )
)]
async fn ingest_handler(
    State(state): State<AppState>,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    let start = std::time::Instant::now();
    let span = Span::current();

    let payload_size = body.len();
    span.record("payload_size_bytes", payload_size);

    // Deserialize request
    let payload: IngestRequest = match serde_json::from_slice(&body) {
        Ok(p) => p,
        Err(e) => {
            error!(error = %e, "Invalid JSON payload");
            return StatusCode::BAD_REQUEST;
        }
    };

    span.record("table", &payload.table);
    span.record("op", &payload.op);
    span.record("id", &payload.id);

    // Parse operation
    let op = match Operation::from_str(&payload.op) {
        Some(op) => op,
        None => {
            warn!(op = %payload.op, "Invalid operation type");
            return StatusCode::BAD_REQUEST;
        }
    };

    // Prepare record data
    let (clean_record, _hash) = ssp::service::ingest::prepare(payload.record);

    // Process through circuit
    let updates = {
        let mut circuit = state.processor.write().await;
        let entry = BatchEntry::new(&payload.table, op, payload.id, clean_record.into());
        circuit.ingest_single(entry)
    };

    // Record metrics
    state.metrics.inc_ingest(
        1,
        &[
            opentelemetry::KeyValue::new("table", payload.table.clone()),
            opentelemetry::KeyValue::new("op", payload.op.clone()),
        ],
    );
    span.record("views_affected", updates.len());

    // Trigger background save
    state.saver.trigger_save();

    // Extract streaming updates for edge creation
    let streaming_updates: Vec<&StreamingUpdate> = updates
        .iter()
        .filter_map(|u| match u {
            ViewUpdate::Streaming(s) => Some(s),
            _ => None,
        })
        .collect();

    if !streaming_updates.is_empty() {
        // DIAGNOSTIC LOGGING - Remove after debugging
        #[cfg(debug_assertions)]
        {
            warn!(
                target: "ssp::debug::ingest",
                total_views = streaming_updates.len(),
                "Processing streaming updates"
            );

            for (idx, update) in streaming_updates.iter().enumerate() {
                warn!(
                    target: "ssp::debug::ingest",
                    view_idx = idx,
                    view_id = %update.view_id,
                    records_count = update.records.len(),
                    records_sample = ?update.records.iter().take(3).map(|r| &r.id).collect::<Vec<_>>(),
                    "View update details"
                );
            }
        }

        let edge_count: usize = streaming_updates.iter().map(|s| s.records.len()).sum();
        span.record("edges_updated", edge_count);

        // Update edges in database
        let circuit = state.processor.read().await;
        update_all_edges(&state.db, &streaming_updates, &state.metrics, &circuit).await;
    }

    // Record duration
    let duration_ms = start.elapsed().as_secs_f64() * 1000.0;
    state.metrics.ingest_duration.record(duration_ms, &[]);

    StatusCode::OK
}

/// Log handler - receives logs from client and forwards to tracing
#[instrument(skip(payload), fields(level = %payload.level))]
async fn log_handler(Json(payload): Json<LogRequest>) -> impl IntoResponse {
    let msg = if let Some(data) = &payload.data {
        format!("{} | data: {}", payload.message, data)
    } else {
        payload.message.clone()
    };

    match payload.level.to_lowercase().as_str() {
        "error" => error!(remote = true, "{}", msg),
        "warn" => warn!(remote = true, "{}", msg),
        "debug" => debug!(remote = true, "{}", msg),
        "trace" => tracing::trace!(remote = true, "{}", msg),
        _ => info!(remote = true, "{}", msg),
    }

    StatusCode::OK
}

/// Register view handler - creates a new view and initializes edges
#[instrument(skip(state), fields(view_id = Empty))]
async fn register_view_handler(
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
    let span = Span::current();

    // Parse and validate registration data
    let data = match ssp::service::view::prepare_registration(payload) {
        Ok(d) => d,
        Err(e) => {
            error!(error = %e, "Invalid view registration payload");
            return (StatusCode::BAD_REQUEST, e.to_string()).into_response();
        }
    };

    span.record("view_id", &data.plan.id);

    // Extract metadata
    let raw_id = data
        .metadata
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let incantation_id = format_incantation_id(raw_id);

    // Check if view exists and clean up old edges
    let view_existed = {
        let circuit = state.processor.read().await;
        circuit.views.iter().any(|v| v.plan.id == data.plan.id)
    };

    if view_existed {
        info!(
            target: "ssp::edges",
            view_id = %incantation_id,
            "View already existed - skipping registration"
        );

        return StatusCode::OK.into_response();
    }

    debug!("Registering view: {}", data.plan.id);

    // Register view with Streaming format
    let update = {
        let mut circuit = state.processor.write().await;
        let res = circuit.register_view(
            data.plan.clone(),
            data.safe_params,
            Some(ViewResultFormat::Streaming),
        );
        state.saver.trigger_save();
        res
    };

    state.metrics.view_count.add(1, &[]);

    // Extract metadata fields
    let client_id = data
        .metadata
        .get("clientId")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let surreal_ql = data
        .metadata
        .get("sql")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let ttl = data
        .metadata
        .get("ttl")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let last_active_at = data
        .metadata
        .get("lastActiveAt")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let params = data
        .metadata
        .get("safe_params")
        .cloned()
        .unwrap_or(Value::Null);

    // Store incantation metadata
    let query = "UPSERT <record>$id SET clientId = <string>$clientId, surql = <string>$surql, params = $params, ttl = <duration>$ttl, lastActiveAt = <datetime>$lastActiveAt";

    if let Err(e) = state
        .db
        .query(query)
        .bind(("id", incantation_id.clone()))
        .bind(("clientId", client_id))
        .bind(("surql", surreal_ql))
        .bind(("params", params))
        .bind(("ttl", ttl))
        .bind(("lastActiveAt", last_active_at))
        .await
    {
        error!("Failed to upsert incantation metadata: {}", e);
        return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
    }

    // Create initial edges
    if let Some(ViewUpdate::Streaming(s)) = &update {
        debug!(incantation_id);
        let circuit = state.processor.read().await;
        update_incantation_edges(&state.db, s, &state.metrics, &circuit).await;
    }

    StatusCode::OK.into_response()
}

/// Unregister view handler - removes view and deletes all associated edges
#[instrument(skip(state), fields(view_id = %payload.id))]
async fn unregister_view_handler(
    State(state): State<AppState>,
    Json(payload): Json<UnregisterViewRequest>,
) -> impl IntoResponse {
    debug!("Unregistering view: {}", payload.id);

    // Remove from circuit
    {
        let mut circuit = state.processor.write().await;
        circuit.unregister_view(&payload.id);
        state.saver.trigger_save();
    }

    state.metrics.view_count.add(-1, &[]);

    // Delete all edges for this incantation
    let incantation_id = format_incantation_id(&payload.id);
    if let Some(from_id) = parse_record_id(&incantation_id) {
        if let Err(e) = state
            .db
            .query("DELETE $from->_spooky_list_ref")
            .bind(("from", from_id))
            .await
        {
            error!("Failed to delete edges for view {}: {}", incantation_id, e);
        } else {
            debug!("Deleted all edges for view {}", incantation_id);
        }
    }

    StatusCode::OK
}

/// Reset handler - clears all circuit state and edges
async fn reset_handler(State(state): State<AppState>) -> impl IntoResponse {
    info!("Resetting circuit state");

    let old_view_count = {
        let mut circuit = state.processor.write().await;
        let count = circuit.views.len();
        *circuit = Circuit::new();
        if state.persistence_path.exists() {
            let _ = std::fs::remove_file(&state.persistence_path);
        }
        state.saver.trigger_save();
        count
    };

    state.metrics.view_count.add(-(old_view_count as i64), &[]);

    // Delete all edges
    if let Err(e) = state.db.query("DELETE _spooky_list_ref").await {
        error!("Failed to delete all edges on reset: {}", e);
    }

    StatusCode::OK
}

/// Force save handler - triggers immediate persistence
async fn save_handler(State(state): State<AppState>) -> impl IntoResponse {
    info!("Force saving state");
    state.saver.trigger_save();
    StatusCode::OK
}

/// Health check handler
async fn health_handler(State(state): State<AppState>) -> impl IntoResponse {
    let circuit = state.processor.read().await;
    Json(json!({
        "status": "healthy",
        "views": circuit.views.len(),
        "tables": circuit.db.tables.len(),
    }))
}

/// Debug view handler - returns cache state for a specific view
async fn debug_view_handler(
    State(state): State<AppState>,
    Path(view_id): Path<String>,
) -> impl IntoResponse {
    let circuit = state.processor.read().await;

    if let Some(view) = circuit.views.iter().find(|v| v.plan.id == view_id) {
        let cache_summary: Vec<_> = view
            .cache
            .iter()
            .map(|(k, &w)| json!({ "key": k, "weight": w }))
            .collect();

        Json(json!({
            "view_id": view_id,
            "cache_size": view.cache.len(),
            "last_hash": view.last_hash,
            "format": view.format,
            "cache": cache_summary,
        }))
    } else {
        Json(json!({ "error": "View not found" }))
    }
}

/// Version handler
async fn version_handler() -> impl IntoResponse {
    Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "mode": "streaming"
    }))
}

// --- Helper Functions ---

/// Parse a record ID string into SurrealDB RecordId
fn parse_record_id(id: &str) -> Option<RecordId> {
    RecordId::parse_simple(id).ok()
}

/// Format incantation ID with proper prefix
fn format_incantation_id(id: &str) -> String {
    if id.starts_with("_spooky_query:") {
        id.to_string()
    } else {
        format!("_spooky_query:{}", id)
    }
}

/// Update edges for multiple views in a SINGLE database transaction
///
/// This batches all edge operations across multiple views into one transaction,
/// significantly reducing database round-trips.
///
/// Example: 3 views × 1 record each = 1 transaction instead of 3
#[instrument(skip(db, updates, metrics), fields(total_operations = Empty))]
pub async fn update_all_edges<C: Connection>(
    db: &Surreal<C>,
    updates: &[&StreamingUpdate],
    metrics: &Metrics,
    circuit: &Circuit,
) {
    if updates.is_empty() {
        return;
    }

    let span = Span::current();
    let mut all_statements: Vec<String> = Vec::new();
    let mut bindings: Vec<(String, RecordId)> = Vec::new();

    let mut created_count = 0;
    let mut updated_count = 0;
    let mut deleted_count = 0;

    // Build SQL statements for each view's updates
    for (idx, update) in updates.iter().enumerate() {
        if update.records.is_empty() {
            continue;
        }

        let incantation_id = format_incantation_id(&update.view_id);

        let Some(from_id) = parse_record_id(&incantation_id) else {
            error!(
                incantation_id = %incantation_id,
                "Invalid incantation ID format - skipping view"
            );
            continue;
        };

        let binding_name = format!("from{}", idx);
        bindings.push((binding_name.clone(), from_id));

        // Process each record in the update
        for record in &update.records {
            if parse_record_id(&record.id).is_none() {
                error!(
                    target: "ssp::edges",
                    record_id = %record.id,
                    view_id = %update.view_id,
                    event = ?record.event,
                    "Invalid record ID format - skipping edge operation"
                );
                continue;
            }

            let record_id_parsed = parse_record_id(&record.id);
            if record_id_parsed.is_none() {
                continue;
            }
            let table_name = &record_id_parsed.unwrap().table;

            let record_verion = circuit
                .db
                .get_table(table_name)
                .and_then(|t| t.get_record_version(&record.id));
            println!("Record version: {:?}", record_verion.unwrap());

            tracing::debug!(
                target: "ssp::edges",
                record_version = record_verion,
            );

            let stmt = match record.event {
                DeltaEvent::Created => {
                    created_count += 1;
                    format!(
                        "RELATE ${1}->_spooky_list_ref->{0} SET version = {2}, clientId = (SELECT VALUE clientId FROM ${1} LIMIT 1)[0]",
                        record.id,
                        binding_name,
                        record_verion.unwrap_or(1)
                    )
                }
                DeltaEvent::Updated => {
                    updated_count += 1;
                    format!(
                        "UPDATE _spooky_list_ref SET version = {2} WHERE in = ${0} AND out = {1}",
                        binding_name,
                        record.id,
                        record_verion.unwrap_or(1)
                    )
                }
                DeltaEvent::Deleted => {
                    deleted_count += 1;
                    format!(
                        "DELETE ${1}->_spooky_list_ref WHERE out = {0}",
                        record.id, binding_name
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
    metrics.edge_operations.add(
        created_count,
        &[opentelemetry::KeyValue::new("operation", "create")],
    );
    metrics.edge_operations.add(
        updated_count,
        &[opentelemetry::KeyValue::new("operation", "update")],
    );
    metrics.edge_operations.add(
        deleted_count,
        &[opentelemetry::KeyValue::new("operation", "delete")],
    );

    debug!(
        created = created_count,
        updated = updated_count,
        deleted = deleted_count,
        views = updates.len(),
        "Processing edge operations"
    );

    // Wrap all statements in a single transaction
    let full_query = format!(
        "BEGIN TRANSACTION;\n{};\nCOMMIT TRANSACTION;",
        all_statements.join(";\n")
    );

    // Build query with bindings
    let mut query = db.query(&full_query);

    #[cfg(debug_assertions)]
    {
        let mut debug_query = full_query.clone();
        for (name, id) in &bindings {
            let id_str = format!("{:?}", id);
            debug_query = debug_query.replace(&format!("${}", name), &id_str);
        }
        debug!(target: "ssp::edges::sql", "{}", debug_query);
    }

    for (name, id) in bindings {
        query = query.bind((name, id));
    }

    // Execute transaction
    match query.await {
        Ok(_) => {
            debug!(
                target: "ssp::edges",
                operations = all_statements.len(),
                "Edge update transaction completed successfully"
            );
        }
        Err(e) => {
            error!(
                target: "ssp::edges",
                error = %e,
                operations = all_statements.len(),
                "Edge update transaction failed - data may be out of sync"
            );
        }
    }
}

/// Update edges for a single view (convenience wrapper for register_view_handler)
async fn update_incantation_edges<C: Connection>(
    db: &Surreal<C>,
    update: &StreamingUpdate,
    metrics: &Metrics,
    circuit: &Circuit,
) {
    update_all_edges(db, &[update], metrics, circuit).await;
}
