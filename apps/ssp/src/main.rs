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
    Circuit, MaterializedViewUpdate, ViewUpdate,
    engine::update::{StreamingUpdate, DeltaEvent, compute_flat_hash, ViewResultFormat},
};
use surrealdb::engine::remote::ws::{Client, Ws};
use surrealdb::opt::auth::Root;
use surrealdb::Surreal;
use surrealdb::types::RecordId;
use tracing::{info, error, debug, instrument};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use tokio::signal;

mod persistence;
mod background_saver;
use background_saver::BackgroundSaver;

#[derive(Clone)]
struct AppState {
    db: Surreal<Client>,
    processor: Arc<Mutex<Box<Circuit>>>,
    persistence_path: PathBuf,
    saver: Arc<BackgroundSaver>,
}

#[derive(Deserialize, Debug)]
struct IngestRequest {
    table: String,
    op: String,
    id: String,
    record: Value,
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
    let file_appender = tracing_appender::rolling::daily("logs", "ssp.log");
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

    info!("Starting ssp (PARALLEL MODE: array + edges)...");

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
    db.signin(Root { username: db_user.clone(), password: db_pass.clone() }).await.context("Failed to signin")?;
    db.use_ns(&db_ns).use_db(&db_db).await.context("Failed to select ns/db")?;

    info!("Connected to SurrealDB");

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

    // Get updates from BOTH modes
    let (flat_updates, streaming_updates) = {
        let mut circuit = state.processor.lock().unwrap();
        
        // We need to call ingest_record once - it returns updates based on the view's format
        // But views are registered with Streaming format, so we get StreamingUpdate
        let updates = circuit.ingest_record(
            &payload.table,
            &payload.op,
            &payload.id,
            clean_record.into(),
            &hash,
            true,
        );
        
        state.saver.trigger_save();
        
        // Separate into flat data and streaming data
        let mut flat: Vec<MaterializedViewUpdate> = Vec::new();
        let mut streaming: Vec<StreamingUpdate> = Vec::new();
        
        for update in updates {
            match update {
                ViewUpdate::Streaming(s) => {
                    // We need the FULL view state for flat mode, not just deltas
                    // This is a limitation - we'll handle it differently
                    streaming.push(s);
                }
                ViewUpdate::Flat(m) | ViewUpdate::Tree(m) => {
                    flat.push(m);
                }
            }
        }
        
        (flat, streaming)
    };

    // PARALLEL UPDATE: Both array AND edges
    // 
    // For flat updates (if any views are in flat mode)
    for update in &flat_updates {
        update_incantation_array(&state.db, update).await;
    }
    
    // For streaming updates - update BOTH array and edges
    for update in &streaming_updates {
        // Update edges (streaming way)
        update_incantation_edges(&state.db, update).await;
        
        // Also update array (flat way) - need to get full state from circuit
        update_incantation_array_from_streaming(&state.db, &state.processor, &update.view_id).await;
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

    debug!("Registering view {} (PARALLEL: array + edges)", data.plan.id);

    // Always register with Streaming mode (it gives us delta info)
    let update = {
        let mut circuit = state.processor.lock().unwrap();
        let res = circuit.register_view(
            data.plan.clone(), 
            data.safe_params, 
            Some(ViewResultFormat::Streaming)
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

    // Extract initial data from streaming update
    let (result_hash, result_data) = if let Some(ViewUpdate::Streaming(s)) = &update {
        let initial_data: Vec<(String, u64)> = s.records
            .iter()
            .filter(|r| r.event == DeltaEvent::Created)
            .map(|r| (r.id.clone(), r.version))
            .collect();
        let hash = compute_flat_hash(&initial_data);
        (hash, initial_data)
    } else {
        (compute_flat_hash(&[]), vec![])
    };

    // UPSERT incantation with array (for flat/old sync)
    let query = "UPSERT <record>$id SET hash = <string>$hash, array = $array, clientId = <string>$clientId, surrealQL = <string>$surrealQL, params = $params, ttl = <duration>$ttl, lastActiveAt = <datetime>$lastActiveAt";

    let db_res = state.db.query(query)
        .bind(("id", id_str.clone()))
        .bind(("hash", result_hash))
        .bind(("array", json!(result_data)))
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

    // ALSO create edges (for streaming/new sync)
    if let Some(ViewUpdate::Streaming(s)) = &update {
        debug!("Creating {} initial edges for view {}", s.records.len(), id_str);
        update_incantation_edges(&state.db, s).await;
    }

    debug!("View {} registered with BOTH array ({} items) and edges", id_str, result_data.len());

    StatusCode::OK.into_response()
}

#[instrument(skip(state))]
async fn unregister_view_handler(
    State(state): State<AppState>,
    Json(payload): Json<UnregisterViewRequest>,
) -> impl IntoResponse {
    debug!("Unregistering view {}", payload.id);
    
    {
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
    Json(json!("0.2.0-parallel"))
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

/// Update incantation array (flat mode)
async fn update_incantation_array(db: &Surreal<Client>, update: &MaterializedViewUpdate) {
    let query = "UPDATE <record>$id SET hash = <string>$hash, array = $array";
    let id_str = format_incantation_id(&update.query_id);
    
    debug!("Updating array for {} ({} items)", id_str, update.result_data.len());
    
    if let Err(e) = db.query(query)
        .bind(("id", id_str.clone()))
        .bind(("hash", update.result_hash.clone()))
        .bind(("array", json!(update.result_data)))
        .await 
    {
        error!("Failed to update array for {}: {}", id_str, e);
    }
}

/// Update incantation array from streaming update by getting full view state
async fn update_incantation_array_from_streaming(
    db: &Surreal<Client>,
    processor: &Arc<Mutex<Box<Circuit>>>,
    view_id: &str,
) {
    let id_str = format_incantation_id(view_id);
    
    // Get full view state from circuit's view cache
    let (hash, data) = {
        let circuit = processor.lock().unwrap();
        
        // Find the view by ID
        let view = circuit.views.iter().find(|v| v.plan.id == view_id);
        
        if let Some(v) = view {
            // Build array from view's version_map (Streaming mode source of truth)
            // Note: v.cache is empty in Streaming mode!
            let records: Vec<(String, u64)> = v.version_map
                .iter()
                .map(|(id, version)| (id.to_string(), *version))
                .collect();
            
            let hash = compute_flat_hash(&records);
            (hash, records)
        } else {
            debug!("View {} not found in circuit", view_id);
            return;
        }
    };
    
    let query = "UPDATE <record>$id SET hash = <string>$hash, array = $array";
    
    debug!("Updating array for {} ({} items)", id_str, data.len());
    
    if let Err(e) = db.query(query)
        .bind(("id", id_str.clone()))
        .bind(("hash", hash))
        .bind(("array", json!(data)))
        .await 
    {
        error!("Failed to update array for {}: {}", id_str, e);
    }
}

/// Update edges (streaming mode) - batched transaction
/// Update edges (streaming mode) - unbatched for stability
async fn update_incantation_edges(db: &Surreal<Client>, update: &StreamingUpdate) {
    if update.records.is_empty() {
        return;
    }

    let incantation_id_str = format_incantation_id(&update.view_id);
    
    // Validate from_id
    let Some(from_id) = parse_record_id(&incantation_id_str) else {
        error!("Invalid incantation ID format: {}", incantation_id_str);
        return;
    };

    debug!(
        "Processing {} edge operations for {} (unbatched)",
        update.records.len(),
        incantation_id_str
    );

    for record in &update.records {
        // Validate record ID
        if parse_record_id(&record.id).is_none() {
            error!("Invalid record ID format: {}", record.id);
            continue;
        }

        let query = match record.event {
            DeltaEvent::Created => {
                format!(
                    "RELATE $from->_spooky_list_ref->{} SET version = {}, clientId = (SELECT clientId FROM ONLY $from).clientId",
                    record.id, record.version
                )
            }
            DeltaEvent::Updated => {
                format!(
                    "UPDATE $from->_spooky_list_ref SET version = {} WHERE out = {}",
                    record.version, record.id
                )
            }
            DeltaEvent::Deleted => {
                format!(
                    "DELETE $from->_spooky_list_ref WHERE out = {}",
                    record.id
                )
            }
        };
        
        // Execute individually
        if let Err(e) = db.query(&query)
            .bind(("from", from_id.clone()))
            .await 
        {
            error!("Edge operation failed: {}", e);
        }
    }
}