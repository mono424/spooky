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
use std::sync::{Arc, Mutex};
use ssp::{
    engine::circuit::{Circuit, IngestBatch, BatchEntry, Operation},
    engine::update::{StreamingUpdate, DeltaEvent, ViewResultFormat, ViewUpdate},
    engine::metadata::VersionStrategy,
};
use surrealdb::engine::remote::ws::{Client, Ws};
use surrealdb::opt::auth::Root;
use surrealdb::Surreal;
use surrealdb::types::RecordId;
use tracing::{info, error, debug, instrument};
//use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use tokio::signal;

mod persistence;
mod background_saver;
use background_saver::BackgroundSaver;

mod open_telemetry;

/// Shared database connection wrapped in Arc for true zero-copy sharing
type SharedDb = Arc<Surreal<Client>>;

#[derive(Clone)]
struct AppState {
    db: SharedDb,  // Arc-wrapped for efficient cloning
    processor: Arc<Mutex<Box<Circuit>>>,
    persistence_path: PathBuf,
    saver: Arc<BackgroundSaver>,
}

#[derive(Deserialize, Debug)]
struct LogRequest {
    message: String,
    #[serde(default)]
    level: String,
    #[serde(default)]
    data: Option<Value>,
}

#[derive(Deserialize, Debug)]
struct IngestRequest {
    table: String,
    op: String,
    id: String,
    record: Value,
    pub version: Option<u64>, // Added version
    #[serde(default)]
    _hash: String, 
}

#[derive(Deserialize, Debug)]
struct UnregisterViewRequest {
    id: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    // Setup logging
    /*let file_appender = tracing_appender::rolling::daily("logs", "ssp.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "ssp=debug,axum=info".into());

    let stdout_layer = tracing_subscriber::fmt::layer().with_ansi(true).pretty();
    let file_layer = tracing_subscriber::fmt::layer().with_writer(non_blocking).json();

    tracing_subscriber::registry()
        .with(env_filter)
        .with(stdout_layer)
        .with(file_layer)
        .init();
    */
    open_telemetry::init_tracing().context("Failed to initialize OpenTelemetry tracing")?;
    info!("Starting ssp (streaming mode)...");

    // Persistence Config
    let persistence_file = std::env::var("SPOOKY_PERSISTENCE_FILE")
        .unwrap_or_else(|_| "data/spooky_state.json".to_string());
    let persistence_path = PathBuf::from(persistence_file);

    // Auth Config
    let _auth_secret = std::env::var("SPOOKY_AUTH_SECRET").expect("SPOOKY_AUTH_SECRET must be set");

    // SurrealDB Config
    let db_addr = std::env::var("SURREALDB_ADDR").unwrap_or_else(|_| "127.0.0.1:8000".to_string());
    let db_user = std::env::var("SURREALDB_USER").unwrap_or_else(|_| "root".to_string());
    let db_pass = std::env::var("SURREALDB_PASS").unwrap_or_else(|_| "root".to_string());
    let db_ns = std::env::var("SURREALDB_NS").unwrap_or_else(|_| "test".to_string());
    let db_db = std::env::var("SURREALDB_DB").unwrap_or_else(|_| "test".to_string());

    info!("Connecting to SurrealDB at {}", db_addr);

    let db = Surreal::new::<Ws>(db_addr).await.context("Failed to connect to SurrealDB")?;
    db.signin(Root { username: db_user, password: db_pass }).await.context("Failed to signin")?;
    db.use_ns(&db_ns).use_db(&db_db).await.context("Failed to select ns/db")?;

    info!("Connected to SurrealDB (single persistent connection)");

    // Wrap in Arc for zero-copy sharing across handlers
    let db = Arc::new(db);

    // Load Circuit
    let processor = persistence::load_circuit(&persistence_path);
    let processor_arc = Arc::new(Mutex::new(Box::new(processor)));

    // Initialize Background Saver
    let debounce_ms = 2000;
    let saver = Arc::new(BackgroundSaver::new(
        persistence_path.clone(),
        processor_arc.clone(),
        debounce_ms,
    ));
    
    let saver_clone = saver.clone();
    tokio::spawn(async move {
        saver_clone.run().await;
    });

    let state = AppState {
        db,
        processor: processor_arc,
        persistence_path,
        saver: saver.clone(),
    };

    let app = Router::new()
        .route("/ingest", post(ingest_handler))
        .route("/log", post(log_handler))
        .route("/view/register", post(register_view_handler))
        .route("/view/unregister", post(unregister_view_handler))
        .route("/reset", post(reset_handler))
        .route("/save", post(save_handler))
        .route("/version", get(version_handler))
        .layer(middleware::from_fn(auth_middleware))
        .with_state(state);

    let listener_addr = "0.0.0.0:8667";
    let listener = tokio::net::TcpListener::bind(listener_addr).await.context("Failed to bind port")?;
    info!("Listening on {}", listener_addr);
    
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(saver))
        .await
        .context("Server error")?;
        
    opentelemetry::global::shutdown_tracer_provider();

    Ok(())
}

async fn shutdown_signal(saver: Arc<BackgroundSaver>) {
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

#[instrument(skip(state), fields(table = %payload.table, op = %payload.op, id = %payload.id))]
async fn ingest_handler(
    State(state): State<AppState>,
    Json(payload): Json<IngestRequest>,
) -> impl IntoResponse {
    debug!("Received ingest request");

    let (clean_record, hash) = ssp::service::ingest::prepare(payload.record);

    let updates = {
        let _span = ssp::logging::get_module_span().entered();
        let mut circuit = state.processor.lock().unwrap();

        let op = Operation::from_str(&payload.op).unwrap_or(Operation::Create);

        let mut entry = BatchEntry::new(
             &payload.table,
             op,
             &payload.id,
             clean_record.into(),
             hash,
        );

        let mut batch = IngestBatch::new();

        if let Some(version) = payload.version {
             entry = entry.with_version(version);
             batch = batch.with_strategy(VersionStrategy::Explicit);
        }

        circuit.ingest(batch.entry(entry), true)
    };
    state.saver.trigger_save();

    // Collect all streaming updates and batch into single transaction
    let streaming_updates: Vec<&StreamingUpdate> = updates
        .iter()
        .filter_map(|u| if let ViewUpdate::Streaming(s) = u { Some(s) } else { None })
        .collect();

    if !streaming_updates.is_empty() {
        update_all_edges(&state.db, &streaming_updates).await;
    }

    StatusCode::OK
}

#[instrument(skip(payload), fields(level = %payload.level))]
async fn log_handler(
    Json(payload): Json<LogRequest>,
) -> impl IntoResponse {
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

#[instrument(skip(state))]
async fn register_view_handler(
    State(state): State<AppState>,
    Json(payload): Json<Value>, 
) -> impl IntoResponse {
    let result = ssp::service::view::prepare_registration(payload);
    let data = match result {
        Ok(d) => d,
        Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    };

    debug!("Registering view {}", data.plan.id);

    // Always register with Streaming mode
    let update = {
        let _span = ssp::logging::get_module_span().entered();
        let mut circuit = state.processor.lock().unwrap();
        let res = circuit.register_view(
            data.plan.clone(), 
            data.safe_params, 
            Some(ViewResultFormat::Streaming),
        );
        state.saver.trigger_save();
        res
    };

    let m = &data.metadata;
    let raw_id = m["id"].as_str().unwrap();
    let id_str = format_incantation_id(raw_id);

    let client_id_str = m["clientId"].as_str().unwrap().to_string();
    let surql_str = m["surrealQL"].as_str().unwrap().to_string();
    let ttl_str = m["ttl"].as_str().unwrap().to_string();
    let last_active_str = m["lastActiveAt"].as_str().unwrap().to_string();
    let params_val = m["safe_params"].clone();

    // Store incantation metadata (no array/hash - streaming uses edges)
    let query = "UPSERT <record>$id SET clientId = <string>$clientId, surrealQL = <string>$surrealQL, params = $params, ttl = <duration>$ttl, lastActiveAt = <datetime>$lastActiveAt";

    let db_res = state.db.query(query)
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
        debug!("Creating {} initial edges for view {}", s.records.len(), id_str);
        update_incantation_edges(&state.db, s).await;
    }

    debug!("View {} registered with {} edges", id_str, 
        update.as_ref().map(|u| if let ViewUpdate::Streaming(s) = u { s.records.len() } else { 0 }).unwrap_or(0)
    );

    StatusCode::OK.into_response()
}

#[instrument(skip(state))]
async fn unregister_view_handler(
    State(state): State<AppState>,
    Json(payload): Json<UnregisterViewRequest>,
) -> impl IntoResponse {
    debug!("Unregistering view {}", payload.id);
    
    {
        let _span = ssp::logging::get_module_span().entered();
        let mut circuit = state.processor.lock().unwrap();
        circuit.unregister_view(&payload.id);
        state.saver.trigger_save();
    }
    
    // Delete all edges for this incantation
    let id_str = format_incantation_id(&payload.id);
    if let Some(from_id) = parse_record_id(&id_str) {
        if let Err(e) = state.db
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
    
    {
        let _span = ssp::logging::get_module_span().entered();
        let mut circuit = state.processor.lock().unwrap();
        *circuit = Box::new(Circuit::new());
        if state.persistence_path.exists() {
            let _ = std::fs::remove_file(&state.persistence_path);
        }
        state.saver.trigger_save();
    }
    
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

async fn version_handler() -> impl IntoResponse {
    Json(json!("0.3.2-streaming-batched"))
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
async fn update_all_edges(db: &Surreal<Client>, updates: &[&StreamingUpdate]) {
    if updates.is_empty() {
        return;
    }

    let mut all_statements: Vec<String> = Vec::new();
    let mut bindings: Vec<(String, RecordId)> = Vec::new();
    
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
        bindings.push((binding_name.clone(), from_id));

        for record in &update.records {
            if parse_record_id(&record.id).is_none() {
                error!("Invalid record ID format: {}", record.id);
                continue;
            }

            let stmt = match record.event {
                DeltaEvent::Created => {
                    format!(
                        "RELATE ${}->_spooky_list_ref->{} SET version = {}, clientId = (SELECT clientId FROM ONLY ${}).clientId",
                        binding_name, record.id, record.version, binding_name
                    )
                }
                DeltaEvent::Updated => {
                    format!(
                        "UPDATE ${}->_spooky_list_ref SET version = {} WHERE out = {}",
                        binding_name, record.version, record.id
                    )
                }
                DeltaEvent::Deleted => {
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

    debug!(
        "Processing {} edge operations across {} views in single transaction",
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
    for (name, id) in bindings {
        query = query.bind((name, id));
    }

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
async fn update_incantation_edges(db: &Surreal<Client>, update: &StreamingUpdate) {
    update_all_edges(db, &[update]).await;
}