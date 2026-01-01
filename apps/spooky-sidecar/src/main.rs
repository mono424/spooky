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
use spooky_stream_processor::{Circuit, MaterializedViewUpdate};
use surrealdb::engine::remote::ws::{Client, Ws};
use surrealdb::opt::auth::Root;
use surrealdb::Surreal;
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
    // Load .env file if it exists
    dotenvy::dotenv().ok();

    // Setup logging
    let file_appender = tracing_appender::rolling::daily("logs", "spooky-sidecar.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "spooky_sidecar=debug,axum=info".into());

    let stdout_layer = tracing_subscriber::fmt::layer().with_ansi(true).pretty();
    let file_layer = tracing_subscriber::fmt::layer().with_writer(non_blocking).json();

    tracing_subscriber::registry()
        .with(env_filter)
        .with(stdout_layer)
        .with(file_layer)
        .init();

    info!("Starting spooky-sidecar...");

    // Persistence Config
    let persistence_file = std::env::var("SPOOKY_PERSISTENCE_FILE").unwrap_or_else(|_| "data/spooky_state.json".to_string());
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
    db.signin(Root { username: &db_user, password: &db_pass }).await.context("Failed to signin")?;
    db.use_ns(&db_ns).use_db(&db_db).await.context("Failed to select ns/db")?;

    info!("Connected to SurrealDB");

    // Load Circuit
    let processor = persistence::load_circuit(&persistence_path);
    let processor_arc = Arc::new(Mutex::new(Box::new(processor)));

    // Initialize Background Saver
    let debounce_ms = 2000; // 2 seconds
    let saver = Arc::new(BackgroundSaver::new(
        persistence_path.clone(),
        processor_arc.clone(),
        debounce_ms,
    ));
    
    // Spawn saver task
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
    
    // Graceful shutdown
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
    // Give a moment for the saver to finish
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
) -> Json<Vec<MaterializedViewUpdate>> {
    debug!("Received ingest request");

    let (clean_record, hash) = spooky_stream_processor::service::ingest::prepare(payload.record);

    let updates = {
        let mut circuit = state.processor.lock().unwrap();
        let ups = circuit.ingest_record(
            payload.table,
            payload.op,
            payload.id,
            clean_record,
            hash
        );
        // Trigger async save instead of blocking
        state.saver.trigger_save();
        ups
    };

    // Update SurrealDB incantations
    for update in &updates {
        update_incantation_in_db(&state.db, update).await;
    }

    Json(updates)
}

#[instrument(skip(state))]
async fn register_view_handler(
    State(state): State<AppState>,
    Json(payload): Json<Value>, 
) ->  impl IntoResponse {
    let result = spooky_stream_processor::service::view::prepare_registration(payload);
    let data = match result {
        Ok(d) => d,
        Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    };

    debug!("Registering view {}", data.plan.id);

    let update = {
        let mut circuit = state.processor.lock().unwrap();
        let res = circuit.register_view(data.plan.clone(), data.safe_params);
        state.saver.trigger_save();
        res
    };

    let result_update = update.unwrap_or_else(|| spooky_stream_processor::service::view::default_result(&data.plan.id));

    let m = &data.metadata;
    let query = "UPSERT type::thing('_spooky_incantation', $id) SET hash = $hash, tree = $tree, clientId = $clientId, surrealQL = $surrealQL, params = $params, ttl = <duration>$ttl, lastActiveAt = <datetime>$lastActiveAt";
    
    let raw_id = m["id"].as_str().unwrap();
    let id_str = raw_id.strip_prefix("_spooky_incantation:").unwrap_or(raw_id).to_string();
    let client_id_str = m["clientId"].as_str().unwrap().to_string();
    let surql_str = m["surrealQL"].as_str().unwrap().to_string();
    let ttl_str = m["ttl"].as_str().unwrap().to_string();
    let last_active_str = m["lastActiveAt"].as_str().unwrap().to_string();
    let params_val = m["safe_params"].clone();

    let db_res = state.db.query(query)
        .bind(("id", id_str))
        .bind(("hash", result_update.result_hash.clone()))
        .bind(("tree", result_update.tree.clone()))
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

    Json(json!({
        "hash": result_update.result_hash,
        "tree": result_update.tree
    })).into_response()
}

#[instrument(skip(state))]
async fn unregister_view_handler(
    State(state): State<AppState>,
    Json(payload): Json<UnregisterViewRequest>,
) -> Json<Value> {
    debug!("Unregistering view {}", payload.id);
    {
        let mut circuit = state.processor.lock().unwrap();
        circuit.unregister_view(&payload.id);
        state.saver.trigger_save();
    }
    Json(json!({ "msg": "Unregistered", "id": payload.id }))
}

async fn reset_handler(State(state): State<AppState>) -> Json<Value> {
    info!("Resetting circuit state");
    {
        let mut circuit = state.processor.lock().unwrap();
        *circuit = Box::new(Circuit::new());
        if state.persistence_path.exists() {
             let _ = std::fs::remove_file(&state.persistence_path);
        }
        // For reset, we might want immediate save to confirm empty state
        state.saver.trigger_save();
    }
    Json(Value::Null)
}

async fn save_handler(State(state): State<AppState>) -> Json<Value> {
    info!("Force saving state");
    // Trigger immediate background save? Or force sync?
    // Let's trigger background save, good enough for "Save" endpoint usually
    state.saver.trigger_save();
    Json(Value::Null)
}

async fn version_handler() -> Json<Value> {
    Json(json!("0.1.0"))
}

async fn update_incantation_in_db(db: &Surreal<Client>, update: &MaterializedViewUpdate) {
    let query = "UPDATE type::thing('_spooky_incantation', $id) SET hash = $hash, tree = $tree";
    let raw_id = &update.query_id;
    let id_str = raw_id.strip_prefix("_spooky_incantation:").unwrap_or(raw_id).to_string();
    if let Err(e) = db.query(query)
        .bind(("id", id_str))
        .bind(("hash", update.result_hash.clone()))
        .bind(("tree", update.tree.clone()))
        .await 
    {
        error!("Failed to update incantation result in DB: {}", e);
    }
}
