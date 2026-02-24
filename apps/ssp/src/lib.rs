use anyhow::Context;
use axum::{
    Router,
    extract::{Json, Path, Request, State},
    http::{StatusCode, header::AUTHORIZATION},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

use ssp::circuit::{Circuit, Record, ViewDelta, Change, ChangeSet, Operation};
use ssp::circuit::view::OutputFormat;
use surrealdb::engine::remote::ws::{Client, Ws};
use surrealdb::opt::auth::Root;
use surrealdb::types::RecordId;
use surrealdb::{Connection, Surreal};
use tokio::signal;
use tracing::field::Empty;
use tracing::{Span, debug, error, info, instrument, warn};

// Expose modules for use in main.rs and tests
pub mod metrics;
pub mod open_telemetry;

use metrics::Metrics;

use job_runner::{JobConfig, JobEntry, JobRunner};
use tokio::sync::mpsc;

/// Shared database connection wrapped in Arc for zero-copy sharing across tasks
pub type SharedDb = Arc<Surreal<Client>>;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SspStatus {
    Bootstrapping,
    Ready,
    Failed,
}

#[derive(Serialize)]
pub struct SspError {
    pub code: &'static str,
    pub message: String,
}

pub mod error_codes {
    pub const NOT_READY: &str = "SSP_NOT_READY";
}

#[derive(Clone)]
pub struct AppState {
    pub db: SharedDb,
    pub processor: Arc<RwLock<Circuit>>,
    pub status: Arc<RwLock<SspStatus>>,
    pub metrics: Arc<Metrics>,
    pub job_config: Arc<JobConfig>,
    pub job_queue_tx: mpsc::Sender<JobEntry>,
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
    pub listen_addr: String,
    pub db_addr: String,
    pub db_user: String,
    pub db_pass: String,
    pub db_ns: String,
    pub db_db: String,
    pub spooky_config_path: PathBuf,
    pub scheduler_url: Option<String>,
    pub ssp_id: String,
    pub heartbeat_interval_ms: u64,
}

pub fn load_config() -> Config {
    Config {
        listen_addr: std::env::var("LISTEN_ADDR").unwrap_or_else(|_| "0.0.0.0:8667".to_string()),
        db_addr: std::env::var("SURREALDB_ADDR").unwrap_or_else(|_| "127.0.0.1:8000".to_string()),
        db_user: std::env::var("SURREALDB_USER").unwrap_or_else(|_| "root".to_string()),
        db_pass: std::env::var("SURREALDB_PASS").unwrap_or_else(|_| "root".to_string()),
        db_ns: std::env::var("SURREALDB_NS").unwrap_or_else(|_| "test".to_string()),
        db_db: std::env::var("SURREALDB_DB").unwrap_or_else(|_| "test".to_string()),
        spooky_config_path: PathBuf::from(
            std::env::var("SPOOKY_CONFIG_PATH")
                .unwrap_or_else(|_| "spooky.yml".to_string()),
        ),
        scheduler_url: std::env::var("SCHEDULER_URL").ok(),
        ssp_id: std::env::var("SSP_ID")
            .unwrap_or_else(|_| format!("ssp-{}", uuid::Uuid::new_v4())),
        heartbeat_interval_ms: std::env::var("HEARTBEAT_INTERVAL_MS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(5000),
    }
}

// --- Bootstrap Source ---

/// Abstraction for database access during bootstrap.
/// In standalone mode, bootstraps directly from SurrealDB.
/// In cluster mode, bootstraps from the scheduler's HTTP proxy.
pub enum BootstrapSource {
    /// Direct SurrealDB connection (standalone mode)
    Direct(SharedDb),
    /// HTTP proxy to scheduler's snapshot DB (cluster mode)
    Proxy {
        client: reqwest::Client,
        proxy_url: String,
    },
}

impl BootstrapSource {
    async fn query(&self, surql: &str) -> anyhow::Result<Value> {
        match self {
            BootstrapSource::Direct(db) => {
                let mut response = db.query(surql).await
                    .with_context(|| format!("Query failed: {}", surql))?;
                let val: surrealdb::types::Value = response.take(0)
                    .context("Failed to parse query response")?;
                Ok(serde_json::to_value(&val).unwrap_or_default())
            }
            BootstrapSource::Proxy { client, proxy_url } => {
                let url = format!("{}/query", proxy_url);
                let resp = client
                    .post(&url)
                    .json(&json!({"query": surql}))
                    .send()
                    .await
                    .with_context(|| format!("Proxy query failed: {}", surql))?;

                if !resp.status().is_success() {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    anyhow::bail!("Proxy returned {}: {}", status, body);
                }

                resp.json().await.context("Failed to parse proxy response")
            }
        }
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

    // Start with an empty circuit — self-bootstrap will populate it
    let processor_arc = Arc::new(RwLock::new(Circuit::new()));
    let status = Arc::new(RwLock::new(SspStatus::Bootstrapping));

    // Load job configuration
    let job_config = if config.spooky_config_path.exists() {
        match job_runner::load_config(&config.spooky_config_path) {
            Ok(cfg) => {
                info!(
                    job_tables = cfg.job_tables.len(),
                    "Loaded job configuration"
                );
                Arc::new(cfg)
            }
            Err(e) => {
                warn!(error = %e, "Failed to load job config, job runner disabled");
                Arc::new(JobConfig::default())
            }
        }
    } else {
        info!("No spooky config found, job runner disabled");
        Arc::new(JobConfig::default())
    };

    // Create job queue channel
    let (job_queue_tx, job_queue_rx) = mpsc::channel::<JobEntry>(100);

    // Spawn job runner if there are job tables configured
    if !job_config.job_tables.is_empty() {
        let job_runner = JobRunner::new(job_queue_rx, job_queue_tx.clone(), db.clone());
        tokio::spawn(async move {
            job_runner.run().await;
        });
        info!("Job runner started");
    }

    // Clone for scheduler integration
    let processor_for_scheduler = processor_arc.clone();

    let state = AppState {
        db: db.clone(),
        processor: processor_arc.clone(),
        status: status.clone(),
        metrics: metrics.clone(),
        job_config,
        job_queue_tx,
    };

    let app = create_app(state);

    let listener = tokio::net::TcpListener::bind(&config.listen_addr)
        .await
        .context("Failed to bind port")?;

    info!(addr = %config.listen_addr, "Listening for requests");

    // Spawn self-bootstrap task (runs while server is already accepting /health requests)
    {
        let db = db.clone();
        let processor = processor_arc.clone();
        let status = status.clone();
        let metrics = metrics.clone();
        let scheduler_url = config.scheduler_url.clone();
        let ssp_id = config.ssp_id.clone();
        let listen_addr = config.listen_addr.clone();

        tokio::spawn(async move {
            // Choose bootstrap source based on mode
            let source = if let Some(ref scheduler_url) = scheduler_url {
                // Cluster mode: register with scheduler, then bootstrap from proxy
                let client = reqwest::Client::new();
                let scheduler_base = scheduler_url.trim_end_matches('/');

                let registration_url = format!("{}/ssp/register", scheduler_base);
                info!("Registering SSP {} with scheduler at {}", ssp_id, scheduler_base);

                let payload = json!({
                    "ssp_id": ssp_id,
                    "url": format!("http://{}", listen_addr),
                });

                match client.post(&registration_url).json(&payload).send().await {
                    Ok(resp) if resp.status().is_success() => {
                        info!("Successfully registered with scheduler");
                    }
                    Ok(resp) => {
                        error!("Failed to register with scheduler: HTTP {}", resp.status());
                        *status.write().await = SspStatus::Failed;
                        return;
                    }
                    Err(e) => {
                        error!("Failed to connect to scheduler: {}", e);
                        *status.write().await = SspStatus::Failed;
                        return;
                    }
                }

                let proxy_url = format!("{}/proxy", scheduler_base);
                info!("Bootstrapping from scheduler proxy at {}", proxy_url);
                BootstrapSource::Proxy { client, proxy_url }
            } else {
                // Standalone mode: bootstrap directly from DB
                info!("Standalone mode: bootstrapping from SurrealDB");
                BootstrapSource::Direct(db)
            };

            match self_bootstrap(&source, &processor).await {
                Ok(()) => {
                    let guard = processor.read().await;
                    metrics.view_count.add(guard.view_count() as i64, &[]);
                    info!(
                        tables = guard.table_names().len(),
                        views = guard.view_count(),
                        "Bootstrap complete"
                    );
                    *status.write().await = SspStatus::Ready;
                }
                Err(e) => {
                    error!(error = %e, "Bootstrap failed");
                    *status.write().await = SspStatus::Failed;
                }
            }
        });
    }

    // Spawn heartbeat loop if scheduler configured
    if let Some(scheduler_url) = &config.scheduler_url {
        let ssp_id = config.ssp_id.clone();
        let scheduler_url_clone = scheduler_url.clone();
        let heartbeat_interval = config.heartbeat_interval_ms;
        let processor_clone = processor_for_scheduler.clone();

        tokio::spawn(async move {
            let client = reqwest::Client::new();
            let heartbeat_url = format!("{}/ssp/heartbeat", scheduler_url_clone.trim_end_matches('/'));
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(heartbeat_interval));

            loop {
                interval.tick().await;

                let active_queries = {
                    let circuit = processor_clone.read().await;
                    circuit.view_count()
                };

                let payload = json!({
                    "ssp_id": ssp_id,
                    "timestamp": std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs(),
                    "active_queries": active_queries,
                    "cpu_usage": None::<f64>,
                    "memory_usage": None::<f64>,
                });

                match client.post(&heartbeat_url).json(&payload).send().await {
                    Ok(resp) if resp.status() == StatusCode::NOT_FOUND => {
                        warn!("Scheduler doesn't recognize us, needs re-registration");
                    }
                    Ok(resp) if resp.status() == StatusCode::CONFLICT => {
                        error!("Buffer overflow detected, need to re-bootstrap");
                    }
                    Ok(resp) if !resp.status().is_success() => {
                        warn!("Heartbeat failed: HTTP {}", resp.status());
                    }
                    Ok(_) => {
                        debug!("Heartbeat sent successfully");
                    }
                    Err(e) => {
                        warn!("Failed to send heartbeat: {}", e);
                    }
                }
            }
        });
    } else {
        info!("No SCHEDULER_URL configured, running in standalone mode");
    }

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(meter_provider))
        .await
        .context("Server error")?;

    opentelemetry::global::shutdown_tracer_provider();

    Ok(())
}

async fn shutdown_signal(
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

    if let Err(e) = meter_provider.shutdown() {
        error!(error = %e, "Failed to shutdown meter provider");
    }
}

// --- Self-Bootstrap ---

/// Bootstrap the circuit by loading all table data and view definitions.
/// Works with either a direct SurrealDB connection or the scheduler's HTTP proxy.
async fn self_bootstrap(
    source: &BootstrapSource,
    processor: &Arc<RwLock<Circuit>>,
) -> anyhow::Result<()> {
    info!("Starting self-bootstrap");

    // Step 1: Discover tables via INFO FOR DB
    let info_json = source.query("INFO FOR DB").await
        .context("Failed to query INFO FOR DB")?;

    let tables: Vec<String> = match info_json.get("tables") {
        Some(Value::Object(tables_map)) => tables_map
            .keys()
            .filter(|name| !name.starts_with("_spooky_"))
            .cloned()
            .collect(),
        _ => {
            info!("No tables found in database");
            vec![]
        }
    };

    info!(count = tables.len(), "Discovered tables: {:?}", tables);

    // Step 2: Load all table data
    for table in &tables {
        let result = source.query(&format!("SELECT * FROM {}", table)).await
            .with_context(|| format!("Failed to query table {}", table))?;

        let rows: Vec<Value> = match result {
            Value::Array(arr) => arr,
            _ => vec![],
        };
        let record_count = rows.len();

        let records: Vec<Record> = rows
            .into_iter()
            .filter_map(|row| {
                let id = row.get("id")?.as_str()?.to_string();
                Some(Record::new(table, &id, row))
            })
            .collect();

        {
            let mut circuit = processor.write().await;
            circuit.load(records);
        }

        info!(table = %table, records = record_count, "Loaded table data");
    }

    // Step 3: Re-register views from _spooky_query
    let result = source.query("SELECT * FROM _spooky_query").await
        .context("Failed to query _spooky_query")?;

    let views: Vec<Value> = match result {
        Value::Array(arr) => arr,
        _ => vec![],
    };
    info!(count = views.len(), "Found persisted views");

    for view_row in views {
        let view_id = match view_row.get("id") {
            Some(Value::String(s)) => s.clone(),
            Some(v) => v.to_string().trim_matches('"').to_string(),
            None => {
                warn!("Skipping view with missing id");
                continue;
            }
        };

        // Strip the table prefix if present (e.g. "_spooky_query:abc" -> "abc")
        let raw_id = view_id
            .strip_prefix("_spooky_query:")
            .unwrap_or(&view_id)
            .to_string();

        let surql = match view_row.get("surql").and_then(|v| v.as_str()) {
            Some(s) => s.to_string(),
            None => {
                warn!(view_id = %raw_id, "Skipping view with missing surql");
                continue;
            }
        };

        let client_id = view_row
            .get("clientId")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let ttl = view_row
            .get("ttl")
            .and_then(|v| v.as_str())
            .unwrap_or("30m")
            .to_string();
        let last_active_at = view_row
            .get("lastActiveAt")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let params = view_row
            .get("params")
            .cloned()
            .unwrap_or(json!({}));

        let payload = json!({
            "id": raw_id,
            "surql": surql,
            "clientId": client_id,
            "ttl": ttl,
            "lastActiveAt": last_active_at,
            "params": params,
        });

        match ssp::service::view::prepare_registration_dbsp(payload) {
            Ok(data) => {
                let mut circuit = processor.write().await;
                circuit.add_query(
                    data.plan,
                    data.safe_params,
                    Some(OutputFormat::Streaming),
                );
                info!(view_id = %raw_id, "Re-registered view");
            }
            Err(e) => {
                warn!(view_id = %raw_id, error = %e, "Failed to re-register view");
            }
        }
    }

    Ok(())
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
) -> Response {
    // Gate: reject if not ready
    let status = *state.status.read().await;
    if status != SspStatus::Ready {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(SspError {
                code: error_codes::NOT_READY,
                message: format!("SSP is in {:?} state", status),
            }),
        )
            .into_response();
    }

    let start = std::time::Instant::now();
    let span = Span::current();

    let payload_size = body.len();
    span.record("payload_size_bytes", payload_size);

    // Deserialize request
    let payload: IngestRequest = match serde_json::from_slice(&body) {
        Ok(p) => p,
        Err(e) => {
            error!(error = %e, "Invalid JSON payload");
            return StatusCode::BAD_REQUEST.into_response();
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
            return StatusCode::BAD_REQUEST.into_response();
        }
    };

    // Prepare record data
    let clean = ssp::sanitizer::normalize_record(payload.record.clone());

    // Check if this is a job table and queue the job if pending
    if let Some(backend_info) = state.job_config.job_tables.get(&payload.table) {
        if op == Operation::Create {
            if let Some(status) = payload.record.get("status").and_then(|v| v.as_str()) {
                if status == "pending" {
                    let job_entry = JobEntry::from_record(
                        payload.id.clone(),
                        backend_info.base_url.clone(),
                        backend_info.auth_token.clone(),
                        &payload.record,
                    );

                    debug!(
                        job_id = %job_entry.id,
                        path = %job_entry.path,
                        backend = %backend_info.name,
                        "Queueing job"
                    );

                    if let Err(e) = state.job_queue_tx.send(job_entry).await {
                        error!(error = %e, "Failed to queue job");
                    }
                }
            }
        }
    }

    // Process through circuit
    let change = match op {
        Operation::Create => Change::create(&payload.table, &payload.id, clean),
        Operation::Update => Change::update(&payload.table, &payload.id, clean),
        Operation::Delete => Change::delete(&payload.table, &payload.id),
    };
    let deltas = {
        let mut circuit = state.processor.write().await;
        circuit.step(ChangeSet { changes: vec![change] })
    };

    // Record metrics
    state.metrics.inc_ingest(
        1,
        &[
            opentelemetry::KeyValue::new("table", payload.table.clone()),
            opentelemetry::KeyValue::new("op", payload.op.clone()),
        ],
    );
    span.record("views_affected", deltas.len());

    if !deltas.is_empty() {
        let edge_count: usize = deltas
            .iter()
            .map(|d| d.additions.len() + d.updates.len() + d.removals.len())
            .sum();
        span.record("edges_updated", edge_count);

        // Update edges in database
        let delta_refs: Vec<&ViewDelta> = deltas.iter().collect();
        let circuit = state.processor.read().await;
        update_all_edges(&state.db, &delta_refs, &state.metrics, &circuit).await;
    }

    // Record duration
    let duration_ms = start.elapsed().as_secs_f64() * 1000.0;
    state.metrics.ingest_duration.record(duration_ms, &[]);

    StatusCode::OK.into_response()
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
) -> Response {
    // Gate: reject if not ready
    let status = *state.status.read().await;
    if status != SspStatus::Ready {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(SspError {
                code: error_codes::NOT_READY,
                message: format!("SSP is in {:?} state", status),
            }),
        )
            .into_response();
    }

    let span = Span::current();

    // Parse and validate registration data
    let data = match ssp::service::view::prepare_registration_dbsp(payload) {
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
        circuit.get_view(&data.plan.id).is_some()
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
        circuit.add_query(
            data.plan.clone(),
            data.safe_params,
            Some(OutputFormat::Streaming),
        )
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
    if let Some(ref delta) = update {
        debug!(incantation_id);
        let circuit = state.processor.read().await;
        update_incantation_edges(&state.db, delta, &state.metrics, &circuit).await;
    }

    StatusCode::OK.into_response()
}

/// Unregister view handler - removes view and deletes all associated edges
#[instrument(skip(state), fields(view_id = %payload.id))]
async fn unregister_view_handler(
    State(state): State<AppState>,
    Json(payload): Json<UnregisterViewRequest>,
) -> Response {
    // Gate: reject if not ready
    let status = *state.status.read().await;
    if status != SspStatus::Ready {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(SspError {
                code: error_codes::NOT_READY,
                message: format!("SSP is in {:?} state", status),
            }),
        )
            .into_response();
    }

    debug!("Unregistering view: {}", payload.id);

    // Remove from circuit
    {
        let mut circuit = state.processor.write().await;
        circuit.remove_query(&payload.id);
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

    StatusCode::OK.into_response()
}

/// Reset handler - clears all circuit state and edges
async fn reset_handler(State(state): State<AppState>) -> impl IntoResponse {
    info!("Resetting circuit state");

    let old_view_count = {
        let mut circuit = state.processor.write().await;
        let count = circuit.view_count();
        *circuit = Circuit::new();
        count
    };

    state.metrics.view_count.add(-(old_view_count as i64), &[]);

    // Delete all edges
    if let Err(e) = state.db.query("DELETE _spooky_list_ref").await {
        error!("Failed to delete all edges on reset: {}", e);
    }

    StatusCode::OK
}

/// Health check handler
async fn health_handler(State(state): State<AppState>) -> Response {
    let status = *state.status.read().await;
    let circuit = state.processor.read().await;
    let http_status = match status {
        SspStatus::Ready => StatusCode::OK,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    };
    (
        http_status,
        Json(json!({
            "status": match status {
                SspStatus::Bootstrapping => "bootstrapping",
                SspStatus::Ready => "ready",
                SspStatus::Failed => "failed",
            },
            "views": circuit.view_count(),
            "tables": circuit.table_names().len(),
        })),
    )
        .into_response()
}

/// Debug view handler - returns cache state for a specific view
async fn debug_view_handler(
    State(state): State<AppState>,
    Path(view_id): Path<String>,
) -> impl IntoResponse {
    let circuit = state.processor.read().await;

    if let Some(view) = circuit.get_view(&view_id) {
        let cache_summary: Vec<_> = view
            .cache
            .iter()
            .map(|(k, &w)| json!({ "key": k, "weight": w }))
            .collect();

        Json(json!({
            "view_id": view_id,
            "cache_size": view.cache.len(),
            "last_hash": view.last_hash,
            "format": format!("{:?}", view.format),
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
#[instrument(skip(db, deltas, metrics, circuit), fields(total_operations = Empty))]
pub async fn update_all_edges<C: Connection>(
    db: &Surreal<C>,
    deltas: &[&ViewDelta],
    metrics: &Metrics,
    circuit: &Circuit,
) {
    if deltas.is_empty() {
        return;
    }

    let span = Span::current();
    let mut all_statements: Vec<String> = Vec::new();
    let mut bindings: Vec<(String, RecordId)> = Vec::new();

    let mut created_count: u64 = 0;
    let mut updated_count: u64 = 0;
    let mut deleted_count: u64 = 0;

    for (idx, delta) in deltas.iter().enumerate() {
        if delta.additions.is_empty() && delta.updates.is_empty() && delta.removals.is_empty() {
            continue;
        }

        let incantation_id = format_incantation_id(&delta.query_id);

        let Some(from_id) = parse_record_id(&incantation_id) else {
            error!(
                incantation_id = %incantation_id,
                "Invalid incantation ID format - skipping view"
            );
            continue;
        };

        let binding_name = format!("from{}", idx);
        bindings.push((binding_name.clone(), from_id));

        // Process additions (Created)
        for id in &delta.additions {
            if parse_record_id(id).is_none() {
                error!(
                    target: "ssp::edges",
                    record_id = %id,
                    view_id = %delta.query_id,
                    "Invalid record ID format - skipping edge create"
                );
                continue;
            }

            let version = circuit.store.get_record_version_by_key(id).unwrap_or(1);
            created_count += 1;
            all_statements.push(format!(
                "RELATE ${1}->_spooky_list_ref->{0} SET version = {2}, clientId = (SELECT VALUE clientId FROM ${1} LIMIT 1)[0]",
                id, binding_name, version
            ));
        }

        // Process updates (Updated)
        for id in &delta.updates {
            if parse_record_id(id).is_none() {
                error!(
                    target: "ssp::edges",
                    record_id = %id,
                    view_id = %delta.query_id,
                    "Invalid record ID format - skipping edge update"
                );
                continue;
            }

            let version = circuit.store.get_record_version_by_key(id).unwrap_or(1);
            updated_count += 1;
            all_statements.push(format!(
                "UPDATE _spooky_list_ref SET version = {2} WHERE in = ${0} AND out = {1}",
                binding_name, id, version
            ));
        }

        // Process removals (Deleted)
        for id in &delta.removals {
            if parse_record_id(id).is_none() {
                error!(
                    target: "ssp::edges",
                    record_id = %id,
                    view_id = %delta.query_id,
                    "Invalid record ID format - skipping edge delete"
                );
                continue;
            }

            deleted_count += 1;
            all_statements.push(format!(
                "DELETE ${1}->_spooky_list_ref WHERE out = {0}",
                id, binding_name
            ));
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
        views = deltas.len(),
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
    delta: &ViewDelta,
    metrics: &Metrics,
    circuit: &Circuit,
) {
    update_all_edges(db, &[delta], metrics, circuit).await;
}
