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
use tracing::{Span, debug, error, info, instrument};

// ... (imports)

// Expose these for use in main.rs and tests
pub mod background_saver;
pub mod metrics;
pub mod open_telemetry;
pub mod persistence;

use background_saver::BackgroundSaver;
use metrics::Metrics;

/// Shared database connection wrapped in Arc for true zero-copy sharing
pub type SharedDb = Arc<Surreal<Client>>;

#[derive(Clone)]
pub struct AppState {
    pub db: SharedDb,
    pub processor: Arc<RwLock<Circuit>>,
    pub persistence_path: PathBuf,
    pub saver: Arc<BackgroundSaver>,
    pub metrics: Arc<Metrics>,
}

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
        .context("Failed to select ns/db")?;

    info!("Connected to SurrealDB");
    Ok(Arc::new(db))
}

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

pub async fn run_server() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    // Initialize observability
    open_telemetry::init_tracing().context("Failed to initialize OpenTelemetry tracing")?;
    let (meter_provider, metrics) =
        metrics::init_metrics().context("Failed to initialize metrics")?;
    let metrics = Arc::new(metrics);

    info!("Starting ssp (streaming mode)...");

    let config = load_config();
    let db = connect_database(&config).await?;

    // Load Circuit
    let processor = persistence::load_circuit(&config.persistence_path);
    let processor_arc = Arc::new(RwLock::new(processor));

    // Initial view count metric
    {
        let guard = processor_arc.read().await;
        metrics.view_count.add(guard.views.len() as i64, &[]);
    }

    // Initialize Background Saver
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
    info!(addr = %config.listen_addr, "Listening");

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
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
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

// --- Handlers ---
use axum::body::Bytes;

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
    body: Bytes, // Hier steckt NUR dein JSON-Payload drin
) -> impl IntoResponse {
    let start = std::time::Instant::now();
    let span = Span::current();

    let payload_size = body.len();
    debug!(
        "Received Record ingest request: payload size: {} bytes",
        payload_size
    );

    // 2. Deserialisierung (von Bytes zu deinem Struct)
    let payload: IngestRequest = match serde_json::from_slice(&body) {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = %e, "Payload ist kein gültiges JSON");
            return StatusCode::BAD_REQUEST;
        }
    };

    // Parse operation
    let op = match Operation::from_str(&payload.op) {
        Some(op) => op,
        None => {
            tracing::warn!(op = %payload.op, "Invalid operation type");
            return StatusCode::BAD_REQUEST;
        }
    };

    let (clean_record, _hash) = ssp::service::ingest::prepare(payload.record);

    let updates = {
        // Use write lock for ingestion
        let mut circuit = state.processor.write().await;

        // i want it normalized not like soe table:id and other just id
        // i want it normalized not like soe table:id and other just id
        let record_id = clean_record
            .get("id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| payload.id.clone());

        debug!("payload: {:?}", clean_record);
        let entry = BatchEntry::new(&payload.table, op, record_id, clean_record.into());

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

    state.saver.trigger_save();

    // Collect all streaming updates and batch into single transaction
    let streaming_updates: Vec<&StreamingUpdate> = updates
        .iter()
        .filter_map(|u| {
            if let ViewUpdate::Streaming(s) = u {
                Some(s)
            } else {
                None
            }
        })
        .collect();

    if !streaming_updates.is_empty() {
        let edge_count = streaming_updates
            .iter()
            .map(|s| s.records.len())
            .sum::<usize>();
        span.record("edges_updated", edge_count);

        debug!("BEFORE EDGE UPDATE: updates: {:?}", streaming_updates);
        update_all_edges(&state.db, &streaming_updates, &state.metrics).await;
    }

    // Record duration
    let duration_ms = start.elapsed().as_secs_f64() * 1000.0;
    state.metrics.ingest_duration.record(duration_ms, &[]);

    StatusCode::OK
}

#[instrument(skip(payload), fields(level = %payload.level))]
async fn log_handler(Json(payload): Json<LogRequest>) -> impl IntoResponse {
    let msg = if let Some(data) = &payload.data {
        format!("{} | data: {}", payload.message, data)
    } else {
        payload.message.clone()
    };

    match payload.level.to_lowercase().as_str() {
        "error" => error!(remote = true, "{}", msg),
        "warn" => tracing::warn!(remote = true, "{}", msg),
        "debug" => debug!(remote = true, "{}", msg),
        "trace" => tracing::trace!(remote = true, "{}", msg),
        _ => info!(remote = true, "{}", msg), // Default to info
    }

    StatusCode::OK
}

#[instrument(skip(state), fields(view_id = Empty))]
async fn register_view_handler(
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
    let span = Span::current();

    let result = ssp::service::view::prepare_registration(payload);
    let data = match result {
        Ok(d) => d,
        Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    };

    span.record("view_id", &data.plan.id);

    // Extract metadata for cleanup check
    let m = &data.metadata;
    let raw_id = m.get("id").and_then(|v| v.as_str()).unwrap_or("unknown");
    let id_str = format_incantation_id(raw_id);

    // Register process with cleanup INSIDE the write lock to prevent race conditions (Issue 5)
    let update = {
        let mut circuit = state.processor.write().await;

        // Check if view exists and clean up old edges WHILE HOLDING LOCK
        let view_existed = circuit.views.iter().any(|v| v.plan.id == data.plan.id);

        if view_existed {
            info!(
                target: "ssp::edges",
                view_id = %id_str,
                "Re-registering view - deleting old edges first"
            );

            // Parse record ID safely
            if let Some(from_id) = parse_record_id(&id_str) {
                // We are holding the lock, so no one can be creating edges right now via ingest
                // BUT we need to be careful about not blocking the async runtime with DB calls if we can avoid it.
                // However, since we need atomicity regarding the "view existence", we must do this logic here.
                // Or we accept that delete happens, then register happens.
                // The race was:
                // 1. Check exists (read lock) -> True
                // 2. Delete edges (NO lock)
                // 3. Ingest happens (read lock) -> Sees view -> Creates edges
                // 4. Register happens (write lock)
                // 5. Create edges

                // If we hold write lock, Step 3 cannot happen.

                match state
                    .db
                    .query("DELETE $from->_spooky_list_ref RETURN BEFORE")
                    .bind(("from", from_id))
                    .await
                {
                    Ok(_) => debug!("Successfully cleaned up old edges for {}", id_str),
                    Err(e) => error!("Failed to cleanup old edges for {}: {}", id_str, e),
                }
            } else {
                error!("Failed to parse view ID for cleanup: {}", id_str);
            }
        }

        let res = circuit.register_view(
            data.plan.clone(),
            data.safe_params,
            Some(ViewResultFormat::Streaming),
        );
        state.saver.trigger_save();
        res
    };

    state.metrics.view_count.add(1, &[]);

    // i dont know which version is better at version-approach ad commit: 7ba7676
    let client_id_str = m
        .get("clientId")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let surql_str = m
        .get("surrealQL")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let ttl_str = m
        .get("ttl")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let last_active_str = m
        .get("lastActiveAt")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let params_val = m
        .get("safe_params")
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    // Store incantation metadata
    let query = "UPSERT <record>$id SET clientId = <string>$clientId, surrealQL = <string>$surrealQL, params = $params, ttl = <duration>$ttl, lastActiveAt = <datetime>$lastActiveAt";

    let db_res = state
        .db
        .query(query)
        .bind(("id", id_str.clone()))
        .bind(("clientId", client_id_str))
        .bind(("surrealQL", surql_str))
        .bind(("params", params_val))
        .bind(("ttl", ttl_str))
        .bind(("lastActiveAt", last_active_str))
        .await;

    if let Err(e) = db_res {
        error!("Failed to upsert incantation metadata: {}", e);
        return (StatusCode::INTERNAL_SERVER_ERROR, "DB Error").into_response();
    }

    // Create initial edges
    if let Some(ViewUpdate::Streaming(s)) = &update {
        debug!(
            "Creating {} initial edges for view {}",
            s.records.len(),
            id_str
        );
        update_incantation_edges(&state.db, s, &state.metrics).await;
    }

    StatusCode::OK.into_response()
}

#[instrument(skip(state), fields(view_id = %payload.id))]
async fn unregister_view_handler(
    State(state): State<AppState>,
    Json(payload): Json<UnregisterViewRequest>,
) -> impl IntoResponse {
    debug!("Unregistering view {}", payload.id);

    {
        let mut circuit = state.processor.write().await;
        circuit.unregister_view(&payload.id);
        state.saver.trigger_save();
    }

    state.metrics.view_count.add(-1, &[]);

    // Delete all edges for this incantation
    let id_str = format_incantation_id(&payload.id);
    if let Some(from_id) = parse_record_id(&id_str) {
        if let Err(e) = state
            .db
            .query("DELETE $from->_spooky_list_ref")
            .bind(("from", from_id))
            .await
        {
            error!("Failed to delete edges for view {}: {}", id_str, e);
        } else {
            debug!("Deleted all edges for view {}", id_str);
        }
    }

    StatusCode::OK
}

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

async fn save_handler(State(state): State<AppState>) -> impl IntoResponse {
    info!("Force saving state");
    state.saver.trigger_save();
    StatusCode::OK
}

async fn health_handler(State(state): State<AppState>) -> impl IntoResponse {
    let circuit = state.processor.read().await;
    Json(json!({
        "status": "healthy",
        "views": circuit.views.len(),
        "tables": circuit.db.tables.len(),
    }))
}

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
            "cache": cache_summary,
        }))
    } else {
        Json(json!({ "error": "View not found" }))
    }
}

async fn version_handler() -> impl IntoResponse {
    Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "mode": "streaming"
    }))
}

// --- Helper Functions ---

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

/// Update edges for multiple views in a SINGLE transaction
/// Optimizes: 3 views × 1 record = 1 transaction instead of 3
#[instrument(skip(db, updates, metrics), fields(total_operations = Empty))]
pub async fn update_all_edges<C: Connection>(
    db: &Surreal<C>,
    updates: &[&StreamingUpdate],
    metrics: &Metrics,
) {
    if updates.is_empty() {
        return;
    }

    // Tracing remove for performance?
    for update in updates.iter() {
        let created: Vec<_> = update
            .records
            .iter()
            .filter(|r| matches!(r.event, DeltaEvent::Created))
            .map(|r| r.id.as_str())
            .collect();

        tracing::info!(
            target: "ssp::edges",
            view_id = %update.view_id,
            created_count = created.len(),
            created_ids = ?created.iter().take(10).collect::<Vec<_>>(),
            "StreamingUpdate record IDs (these are used in SQL queries)"
        );
    }

    let span = Span::current();
    debug!(target: "ssp-server::update", "view_eges: {}, record: {}", updates.len(), updates.iter().map(|u| u.records.len()).sum::<usize>());
    let mut all_statements: Vec<String> = Vec::new();
    let mut bindings: Vec<(String, RecordId)> = Vec::new();

    let mut created_count = 0;
    let mut updated_count = 0;
    let mut deleted_count = 0;

    for (idx, update) in updates.iter().enumerate() {
        if update.records.is_empty() {
            continue;
        }

        let incantation_id_str = format_incantation_id(&update.view_id);
        debug!(target: "ssp::edges", "Incantation ID: {}", incantation_id_str);

        let Some(from_id) = parse_record_id(&incantation_id_str) else {
            error!("Invalid incantation ID format: {}", incantation_id_str);
            continue;
        };

        let binding_name = format!("from{}", idx);
        bindings.push((binding_name.clone(), from_id.clone()));

        for (r_idx, record) in update.records.iter().enumerate() {
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
            debug!(target: "ssp::edges", "Record: {:?}", record);

            // Fix Issues 1, 2, 3, 4, 8
            let stmt = match record.event {
                DeltaEvent::Created => {
                    created_count += 1;
                    // Logic:
                    // 1. Get target ID safely using LIMIT 1, selecting [0] to handle array/none.
                    // 2. Check if target exists (non-none) AND edge does NOT exist.
                    // 3. RELATE safely.
                    // 4. Set version using similar safe fetch.
                    format!(
                        "RELATE ${1}->_spooky_list_ref->(SELECT id FROM ONLY _spooky_version WHERE record_id = {0}) 
                            SET version = (SELECT version FROM ONLY _spooky_version WHERE record_id = {0}).version, 
                                clientId = (SELECT clientId FROM ONLY ${1}).clientId",
                        record.id,
                        binding_name,
                    )
                }
                DeltaEvent::Updated => {
                    updated_count += 1;
                    // Fix version update logic (Issue 4) - removing quotes for RecordId match
                    format!(
                        "UPDATE ${1}->_spooky_list_ref SET version = (SELECT VALUE version FROM _spooky_version WHERE record_id = {0} LIMIT 1)[0] WHERE out = (SELECT VALUE id FROM _spooky_version WHERE record_id = {0} LIMIT 1)[0]",
                        record.id, binding_name
                    )
                }
                DeltaEvent::Deleted => {
                    deleted_count += 1;
                    format!(
                        "DELETE ${1}->_spooky_list_ref WHERE out = (SELECT VALUE id FROM _spooky_version WHERE record_id = {0} LIMIT 1)[0]",
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
        "Processing {} edge operations across {} views",
        all_statements.len(),
        updates.len()
    );

    // Wrap ALL statements in ONE transaction
    let full_query = format!(
        "BEGIN TRANSACTION;\n{};\nCOMMIT TRANSACTION;",
        all_statements.join(";\n")
    );

    // Build query with all bindings
    let mut query = db.query(&full_query);
    let mut debug_query = full_query.clone();

    for (name, id) in bindings {
        // Create a string representation for debugging
        let id_str = format!("{:?}", id);
        debug_query = debug_query.replace(&format!("${}", name), &id_str);

        query = query.bind((name, id));
    }

    debug!(target: "ssp::edges::sql", debug_query);

    match query.await {
        Ok(_) => {
            debug!(
                "Completed {} edge operations across {} views",
                all_statements.len(),
                updates.len()
            );
        }
        Err(e) => {
            error!(
                target: "ssp::edges",
                error = %e,
                "Batched edge update transaction failed! Data may be out of sync. Consider retry or full sync."
            );
            // In a real production system, we might want to schedule a retry here or mark the view as "dirty".
            // For now, logging prominently (Issue 6).
        }
    }
}

/// Update edges for a single view (used by register_view_handler)
async fn update_incantation_edges<C: Connection>(
    db: &Surreal<C>,
    update: &StreamingUpdate,
    metrics: &Metrics,
) {
    update_all_edges(db, &[update], metrics).await;
}
