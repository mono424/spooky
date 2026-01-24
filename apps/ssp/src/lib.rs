use anyhow::Context;
use axum::{
    Router,
    extract::{Json, Request, State},
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
use surrealdb::{Surreal, types::ToSql};
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

        let entry = BatchEntry::new(&payload.table, op, &payload.id, clean_record.into());

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
    debug!("Registering view {}", data.plan.id);

    // Always register with Streaming mode
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

    let m = &data.metadata;
    let raw_id = m["id"].as_str().unwrap();
    let id_str = format_incantation_id(raw_id);

    let client_id_str = m["clientId"].as_str().unwrap().to_string();
    let surql_str = m["surrealQL"].as_str().unwrap().to_string();
    let ttl_str = m["ttl"].as_str().unwrap().to_string();
    let last_active_str = m["lastActiveAt"].as_str().unwrap().to_string();
    let params_val = m["safe_params"].clone();

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
async fn update_all_edges(db: &Surreal<Client>, updates: &[&StreamingUpdate], metrics: &Metrics) {
    if updates.is_empty() {
        return;
    }

    let span = Span::current();
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

        let Some(from_id) = parse_record_id(&incantation_id_str) else {
            error!("Invalid incantation ID format: {}", incantation_id_str);
            continue;
        };

        let binding_name = format!("from{}", idx);
        bindings.push((binding_name.clone(), from_id.clone()));

        debug!(
            "EDGE UPDATE: I_id_str: {}, I_id: {:?}, bindingname: {}",
            incantation_id_str, from_id, binding_name
        );

        for record in &update.records {
            if parse_record_id(&record.id).is_none() {
                error!("Invalid record ID format: {}", record.id);
                continue;
            }

            let stmt = match record.event {
                DeltaEvent::Created => {
                    created_count += 1;
                    format!(
                        "
                        LET $spooky_version = (SELECT id, version FROM ONLY _spooky_version WHERE record_id = {0});
                        LET $target = $spooky_version.id;
                        LET $clientid = (SELECT clientId FROM ONLY ${1}).clientId;
                        RELATE ${1}->_spooky_list_ref->$target SET version = ($spooky_version.version), clientId = $clientid;
                        ",
                        record.id,
                        binding_name,
                    )
                }
                DeltaEvent::Updated => {
                    updated_count += 1;
                    format!(
                        "UPDATE ${}->_spooky_list_ref WHERE out = {}",
                        binding_name, record.id
                    )
                }
                DeltaEvent::Deleted => {
                    deleted_count += 1;
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

    debug!("FULL_QUERY_DEBUG:\n{}", debug_query);

    match query.await {
        Ok(_) => {
            debug!(
                "Completed {} edge operations across {} views",
                all_statements.len(),
                updates.len()
            );
        }
        Err(e) => {
            error!("Batched edge update failed: {}", e);
        }
    }
}

/// Update edges for a single view (used by register_view_handler)
async fn update_incantation_edges(
    db: &Surreal<Client>,
    update: &StreamingUpdate,
    metrics: &Metrics,
) {
    update_all_edges(db, &[update], metrics).await;
}
