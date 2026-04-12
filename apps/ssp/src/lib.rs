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

use ssp::circuit::{Circuit, Record, ViewDelta, Change, ChangeSet, Operation, SubqueryOp};
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
    pub ssp_id: String,
    pub scheduler_url: Option<String>,
    pub start_time: std::time::Instant,
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

use ssp_protocol::{IngestRequest, ViewUnregisterRequest};

// --- Configuration ---

pub struct Config {
    pub listen_addr: String,
    pub db_addr: String,
    pub db_user: String,
    pub db_pass: String,
    pub db_ns: String,
    pub db_db: String,
    pub sp00ky_config_path: PathBuf,
    pub scheduler_url: Option<String>,
    pub ssp_id: String,
    pub heartbeat_interval_ms: u64,
    pub advertise_addr: Option<String>,
    pub ttl_cleanup_interval_secs: u64,
}

pub fn load_config() -> Config {
    Config {
        listen_addr: std::env::var("SPKY_SSP_LISTEN_ADDR").unwrap_or_else(|_| "0.0.0.0:8667".to_string()),
        db_addr: std::env::var("SPKY_DB_URL").unwrap_or_else(|_| "127.0.0.1:8000".to_string()),
        db_user: std::env::var("SPKY_DB_USER").unwrap_or_else(|_| "root".to_string()),
        db_pass: std::env::var("SPKY_DB_PASS").unwrap_or_else(|_| "root".to_string()),
        db_ns: std::env::var("SPKY_DB_NS").unwrap_or_else(|_| "test".to_string()),
        db_db: std::env::var("SPKY_DB_NAME").unwrap_or_else(|_| "test".to_string()),
        sp00ky_config_path: PathBuf::from(
            std::env::var("SPKY_CONFIG_PATH")
                .unwrap_or_else(|_| "sp00ky.yml".to_string()),
        ),
        scheduler_url: std::env::var("SPKY_SCHEDULER_URL").ok(),
        ssp_id: std::env::var("SPKY_SSP_ID")
            .unwrap_or_else(|_| format!("ssp-{}", uuid::Uuid::new_v4())),
        heartbeat_interval_ms: std::env::var("HEARTBEAT_INTERVAL_MS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(5000),
        advertise_addr: std::env::var("SPKY_SSP_ADVERTISE_ADDR").ok(),
        ttl_cleanup_interval_secs: std::env::var("TTL_CLEANUP_INTERVAL_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(60),
    }
}

// --- Scheduler Registration Helper ---

/// Build the SSP registration payload and POST it to the scheduler.
/// Returns `Ok(())` on success or an error on failure.
async fn register_with_scheduler(
    client: &reqwest::Client,
    scheduler_url: &str,
    ssp_id: &str,
    listen_addr: &str,
    advertise_addr: Option<&str>,
) -> Result<(), String> {
    let scheduler_base = scheduler_url.trim_end_matches('/');
    let registration_url = format!("{}/ssp/register", scheduler_base);

    let registration_host = if let Some(addr) = advertise_addr {
        addr.to_string()
    } else {
        let (host, port) = listen_addr.rsplit_once(':').unwrap_or(("0.0.0.0", "8667"));
        if host == "0.0.0.0" || host == "127.0.0.1" {
            let hostname = hostname::get()
                .map(|h| h.to_string_lossy().into_owned())
                .unwrap_or_else(|_| host.to_string());
            format!("{}:{}", hostname, port)
        } else {
            listen_addr.to_string()
        }
    };

    // Collect relevant env vars to send to scheduler
    let env_vars: std::collections::HashMap<String, String> = [
        "SPKY_DB_URL", "SPKY_DB_NS", "SPKY_DB_NAME", "SPKY_DB_USER",
        "SPKY_SCHEDULER_URL", "SPKY_SSP_LISTEN_ADDR", "SPKY_SSP_ADVERTISE_ADDR", "SPKY_SSP_ID",
        "HEARTBEAT_INTERVAL_MS", "TTL_CLEANUP_INTERVAL_SECS",
    ].iter().filter_map(|&key| {
        std::env::var(key).ok().map(|val| (key.to_string(), val))
    }).collect();

    let payload = ssp_protocol::SspRegistration {
        ssp_id: ssp_id.to_string(),
        url: format!("http://{}", registration_host),
        version: env!("CARGO_PKG_VERSION").to_string(),
        env: if env_vars.is_empty() { None } else { Some(env_vars) },
    };

    match client.post(&registration_url).json(&payload).send().await {
        Ok(resp) if resp.status().is_success() => Ok(()),
        Ok(resp) => Err(format!("HTTP {}", resp.status())),
        Err(e) => Err(format!("{}", e)),
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

// --- Job Config Loading ---

async fn load_job_config_from_db(db: &Surreal<Client>) -> anyhow::Result<JobConfig> {
    let result: Option<serde_json::Value> = db
        .select(("_sp00ky_config", "main"))
        .await
        .map_err(|e| anyhow::anyhow!("SurrealDB select failed: {}", e))?;
    match result {
        Some(record) => job_runner::from_db_record(&record),
        None => Ok(JobConfig::default()),
    }
}

fn load_job_config_from_file(path: &std::path::Path) -> Arc<JobConfig> {
    if path.exists() {
        match job_runner::load_config(path) {
            Ok(cfg) => {
                info!(job_tables = cfg.job_tables.len(), "Loaded job config from file");
                Arc::new(cfg)
            }
            Err(e) => {
                warn!(error = %e, "Failed to load job config from file, job runner disabled");
                Arc::new(JobConfig::default())
            }
        }
    } else {
        info!("No job config found, job runner disabled");
        Arc::new(JobConfig::default())
    }
}

// --- Router Setup ---

pub fn create_app(state: AppState) -> Router {
    // Authenticated routes — require Bearer token
    let authenticated = Router::new()
        .route("/ingest", post(ingest_handler))
        .route("/log", post(log_handler))
        .route("/debug/view/:view_id", get(debug_view_handler))
        .route("/debug/deps", get(debug_deps_handler))
        .route("/view/register", post(register_view_handler))
        .route("/view/unregister", post(unregister_view_handler))
        .route("/reset", post(reset_handler))
        .layer(middleware::from_fn(auth_middleware));

    // Public routes — no auth required (health checks, info, version)
    let public = Router::new()
        .route("/health", get(health_handler))
        .route("/info", get(info_handler))
        .route("/version", get(version_handler));

    authenticated.merge(public).with_state(state)
}

// --- Server Lifecycle ---

pub async fn run_server() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    // Initialize observability
    open_telemetry::init_tracing().context("Failed to initialize OpenTelemetry tracing")?;
    let (meter_provider, metrics) =
        metrics::init_metrics().context("Failed to initialize metrics")?;
    let metrics = Arc::new(metrics);

    info!("\n ____  ____  ____\n/ ___)/ ___)(  _ \\\n\\___ \\\\___ \\ ) __/\n(____/(____/(__)    v{}\n\nSp00ky Sync Provider — streaming mode", env!("CARGO_PKG_VERSION"));

    let config = load_config();
    let db = connect_database(&config).await?;

    // Start with an empty circuit — self-bootstrap will populate it
    let processor_arc = Arc::new(RwLock::new(Circuit::new()));
    let status = Arc::new(RwLock::new(SspStatus::Bootstrapping));

    // Load job configuration from SurrealDB (_sp00ky_config:main), fall back to file
    let job_config = match load_job_config_from_db(&db).await {
        Ok(cfg) if !cfg.job_tables.is_empty() => {
            info!(job_tables = cfg.job_tables.len(), "Loaded job config from SurrealDB");
            Arc::new(cfg)
        }
        Ok(_) => {
            info!("No job config in SurrealDB, trying file fallback");
            load_job_config_from_file(&config.sp00ky_config_path)
        }
        Err(e) => {
            warn!(error = %e, "Failed to load job config from SurrealDB, trying file fallback");
            load_job_config_from_file(&config.sp00ky_config_path)
        }
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
        ssp_id: config.ssp_id.clone(),
        scheduler_url: config.scheduler_url.clone(),
        start_time: std::time::Instant::now(),
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
        let advertise_addr = config.advertise_addr.clone();

        tokio::spawn(async move {
            // Choose bootstrap source based on mode
            let source = if let Some(ref scheduler_url) = scheduler_url {
                // Cluster mode: register with scheduler, then bootstrap from proxy
                let client = reqwest::Client::new();
                let scheduler_base = scheduler_url.trim_end_matches('/');

                info!("Registering SSP {} with scheduler at {}", ssp_id, scheduler_base);

                match register_with_scheduler(
                    &client,
                    scheduler_url,
                    &ssp_id,
                    &listen_addr,
                    advertise_addr.as_deref(),
                ).await {
                    Ok(()) => {
                        info!("Successfully registered with scheduler");
                    }
                    Err(e) => {
                        error!("Failed to register with scheduler: {}", e);
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

            // Retry bootstrap up to 10 times with backoff (tables may not exist yet
            // if migrations haven't run)
            let mut attempt = 0;
            loop {
                attempt += 1;
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
                        break;
                    }
                    Err(e) => {
                        if attempt >= 10 {
                            error!(error = %e, attempts = attempt, "Bootstrap failed after retries");
                            *status.write().await = SspStatus::Failed;
                            break;
                        }
                        warn!(error = %e, attempt = attempt, "Bootstrap failed, retrying in 5s...");
                        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    }
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
        let listen_addr = config.listen_addr.clone();
        let advertise_addr = config.advertise_addr.clone();

        tokio::spawn(async move {
            let client = reqwest::Client::new();
            let heartbeat_url = format!("{}/ssp/heartbeat", scheduler_url_clone.trim_end_matches('/'));
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(heartbeat_interval));

            loop {
                interval.tick().await;

                let views = {
                    let circuit = processor_clone.read().await;
                    circuit.view_count()
                };

                let payload = ssp_protocol::SspHeartbeat {
                    ssp_id: ssp_id.clone(),
                    timestamp: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs(),
                    views,
                    cpu_usage: None,
                    memory_usage: None,
                    version: env!("CARGO_PKG_VERSION").to_string(),
                };

                match client.post(&heartbeat_url).json(&payload).send().await {
                    Ok(resp) if resp.status() == StatusCode::NOT_FOUND => {
                        warn!("Scheduler doesn't recognize us, attempting re-registration");
                        match register_with_scheduler(
                            &client,
                            &scheduler_url_clone,
                            &ssp_id,
                            &listen_addr,
                            advertise_addr.as_deref(),
                        ).await {
                            Ok(()) => {
                                info!("Successfully re-registered with scheduler");
                            }
                            Err(e) => {
                                error!("Re-registration failed: {}", e);
                            }
                        }
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
        info!("No SPKY_SCHEDULER_URL configured, running in standalone mode");
    }

    // Spawn TTL cleanup loop
    {
        let db = db.clone();
        let processor = processor_arc.clone();
        let status = status.clone();
        let metrics = metrics.clone();
        let interval_secs = config.ttl_cleanup_interval_secs;

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(
                std::time::Duration::from_secs(interval_secs),
            );

            loop {
                interval.tick().await;

                // Only sweep when SSP is ready (bootstrapped)
                if *status.read().await != SspStatus::Ready {
                    continue;
                }

                ttl_cleanup_sweep(&db, &processor, &metrics).await;
            }
        });
        info!(interval_secs = config.ttl_cleanup_interval_secs, "TTL cleanup loop started");
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
            .filter(|name| !name.starts_with("_00_"))
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

    // Step 3: Re-register views from _00_query
    let result = source.query("SELECT * FROM _00_query").await
        .context("Failed to query _00_query")?;

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

        // Strip the table prefix if present (e.g. "_00_query:abc" -> "abc")
        let raw_id = view_id
            .strip_prefix("_00_query:")
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
    let secret = std::env::var("SPKY_AUTH_SECRET").unwrap_or_default();

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

    // Check if this is a job table and queue the job if pending (only on assigned SSP)
    if let Some(backend_info) = state.job_config.job_tables.get(&payload.table) {
        // In singlenode mode (no scheduler), this SSP handles all jobs.
        // In cluster mode, only process jobs assigned to this SSP.
        let is_standalone = state.scheduler_url.is_none();
        let is_assigned = is_standalone || payload.job_assignee.as_deref() == Some(&state.ssp_id);

        info!(
            table = %payload.table,
            op = ?op,
            record_id = %payload.id,
            backend = %backend_info.name,
            base_url = %backend_info.base_url,
            is_standalone,
            is_assigned,
            job_assignee = ?payload.job_assignee,
            ssp_id = %state.ssp_id,
            record_status = ?payload.record.get("status").and_then(|v| v.as_str()),
            "Job routing: table matched job config"
        );

        if is_assigned && op == Operation::Create {
            if let Some(status) = payload.record.get("status").and_then(|v| v.as_str()) {
                if status == "pending" {
                    let job_timeout_override = payload.record.get("timeout").and_then(|v| v.as_u64()).map(|v| v as u32);
                    let effective_timeout = backend_info.effective_timeout(job_timeout_override);

                    let job_entry = JobEntry::from_record(
                        payload.id.clone(),
                        backend_info.base_url.clone(),
                        backend_info.auth_token.clone(),
                        &payload.record,
                        effective_timeout,
                    );

                    info!(
                        job_id = %job_entry.id,
                        path = %job_entry.path,
                        backend = %backend_info.name,
                        timeout_secs = effective_timeout.as_secs(),
                        "Queueing job for execution"
                    );

                    if let Err(e) = state.job_queue_tx.send(job_entry).await {
                        error!(error = %e, "Failed to queue job");
                    }
                } else {
                    debug!(
                        record_id = %payload.id,
                        status,
                        "Job routing: skipped — status is not 'pending'"
                    );
                }
            } else {
                debug!(
                    record_id = %payload.id,
                    "Job routing: skipped — no 'status' field in record"
                );
            }
        } else if !is_assigned {
            debug!(
                record_id = %payload.id,
                job_assignee = ?payload.job_assignee,
                ssp_id = %state.ssp_id,
                "Job routing: skipped — not assigned to this SSP"
            );
        } else {
            debug!(
                record_id = %payload.id,
                op = ?op,
                "Job routing: skipped — operation is not CREATE"
            );
        }
    } else if !state.job_config.job_tables.is_empty() {
        debug!(
            table = %payload.table,
            configured_tables = ?state.job_config.job_tables.keys().collect::<Vec<_>>(),
            "Job routing: table not in job config"
        );
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
            "View already existed - updating metadata only"
        );

        // Still update the _00_query record for fresh clientId/lastActiveAt
        let client_id = data.metadata.get("clientId").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let last_active_at = data.metadata.get("lastActiveAt").and_then(|v| v.as_str()).unwrap_or("").to_string();

        let query = "UPDATE <record>$id SET clientId = <string>$clientId, lastActiveAt = <datetime>$lastActiveAt";
        if let Err(e) = state.db.query(query)
            .bind(("id", incantation_id.clone()))
            .bind(("clientId", client_id))
            .bind(("lastActiveAt", last_active_at))
            .await
        {
            error!("Failed to update incantation metadata: {}", e);
        }

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
    Json(payload): Json<ViewUnregisterRequest>,
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
            .query("DELETE $from->_00_list_ref")
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
    if let Err(e) = state.db.query("DELETE _00_list_ref").await {
        error!("Failed to delete all edges on reset: {}", e);
    }

    StatusCode::OK
}

/// Health check handler
async fn health_handler(State(state): State<AppState>) -> Response {
    let status = *state.status.read().await;
    let http_status = match status {
        SspStatus::Ready => StatusCode::OK,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    };
    let status_str = match status {
        SspStatus::Bootstrapping => "bootstrapping",
        SspStatus::Ready => "ready",
        SspStatus::Failed => "failed",
    };
    (http_status, Json(json!({ "status": status_str }))).into_response()
}

/// Info handler — returns entity list with identity and status
async fn info_handler(State(state): State<AppState>) -> Json<Value> {
    let status = *state.status.read().await;
    let circuit = state.processor.read().await;
    let status_str = match status {
        SspStatus::Bootstrapping => "bootstrapping",
        SspStatus::Ready => "ready",
        SspStatus::Failed => "failed",
    };
    // Collect relevant environment variables
    let env_vars: serde_json::Map<String, Value> = [
        "SPKY_DB_URL", "SPKY_DB_NS", "SPKY_DB_NAME", "SPKY_DB_USER",
        "SPKY_SCHEDULER_URL", "SPKY_SSP_LISTEN_ADDR", "SPKY_SSP_ADVERTISE_ADDR", "SPKY_SSP_ID",
        "HEARTBEAT_INTERVAL_MS", "TTL_CLEANUP_INTERVAL_SECS",
    ].iter().filter_map(|&key| {
        std::env::var(key).ok().map(|val| (key.to_string(), Value::String(val)))
    }).collect();

    // Derive IP from SPKY_SSP_ADVERTISE_ADDR (e.g. "10.100.1.30:8667" -> "10.100.1.30")
    let ip = std::env::var("SPKY_SSP_ADVERTISE_ADDR").ok()
        .and_then(|addr| addr.split(':').next().map(|s| s.to_string()));

    Json(json!([
        {
            "entity": "ssp",
            "id": state.ssp_id,
            "ip": ip,
            "status": status_str,
            "views": circuit.view_count(),
            "version": env!("CARGO_PKG_VERSION"),
            "uptime_seconds": state.start_time.elapsed().as_secs(),
            "last_heartbeat_seconds_ago": null,
            "env": env_vars,
        }
    ]))
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
            "subquery_tables": view.subquery_tables,
            "referenced_tables": view.referenced_tables,
            "content_generation": view.content_generation,
            "subquery_cache": view.subquery_cache.iter()
                .map(|(k, (pk, alias))| json!({"key": k, "parent_key": pk, "alias": alias}))
                .collect::<Vec<_>>(),
        }))
    } else {
        Json(json!({ "error": "View not found" }))
    }
}

/// Debug dependency map handler
async fn debug_deps_handler(State(state): State<AppState>) -> impl IntoResponse {
    let circuit = state.processor.read().await;
    let deps = circuit.dependency_map_dump();
    Json(json!({
        "dependency_map": deps,
        "tables_in_store": circuit.table_names(),
        "view_count": circuit.view_count(),
    }))
}

/// Version handler
async fn version_handler() -> impl IntoResponse {
    Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "mode": "streaming"
    }))
}

// --- TTL Cleanup ---

/// Clean up a single expired query. Uses conditional DELETE to guard against race conditions
/// where a client heartbeats between the sweep check and the actual delete.
async fn cleanup_expired_query(
    db: &SharedDb,
    processor: &Arc<RwLock<Circuit>>,
    metrics: &Arc<Metrics>,
    query_id: &str,
) {
    let incantation_id = format_incantation_id(query_id);
    let Some(record_id) = parse_record_id(&incantation_id) else {
        error!(query_id = %query_id, "TTL cleanup: invalid record ID");
        return;
    };

    // Conditional delete — only if TTL is STILL expired (guards against heartbeat race)
    match db
        .query("DELETE $id WHERE lastActiveAt + ttl < time::now() RETURN BEFORE")
        .bind(("id", record_id.clone()))
        .await
    {
        Ok(mut response) => {
            let deleted: Vec<Value> = response.take(0).unwrap_or_default();
            if deleted.is_empty() {
                debug!(query_id = %query_id, "TTL cleanup: query refreshed, skipping");
                return;
            }
        }
        Err(e) => {
            error!(query_id = %query_id, error = %e, "TTL cleanup: delete failed");
            return;
        }
    }

    // Delete associated list_ref edges
    if let Err(e) = db
        .query("DELETE $id->_00_list_ref")
        .bind(("id", parse_record_id(&incantation_id).unwrap()))
        .await
    {
        error!(query_id = %query_id, error = %e, "TTL cleanup: edge delete failed");
    }

    // Remove from circuit (in-memory)
    {
        let mut circuit = processor.write().await;
        circuit.remove_query(query_id);
    }
    metrics.view_count.add(-1, &[]);
    metrics.ttl_cleanup_count.add(1, &[]);
    info!(query_id = %query_id, "TTL cleanup: query expired and removed");
}

/// Perform one sweep — query SurrealDB for all expired queries and clean each one up.
async fn ttl_cleanup_sweep(
    db: &SharedDb,
    processor: &Arc<RwLock<Circuit>>,
    metrics: &Arc<Metrics>,
) -> usize {
    let view_ids: Vec<String> = {
        let circuit = processor.read().await;
        circuit.view_ids()
    };

    if view_ids.is_empty() {
        return 0;
    }

    let expired_ids: Vec<String> = match db
        .query("SELECT VALUE id FROM _00_query WHERE lastActiveAt + ttl < time::now()")
        .await
    {
        Ok(mut response) => response.take(0).unwrap_or_default(),
        Err(e) => {
            error!("TTL cleanup: query failed: {}", e);
            return 0;
        }
    };

    // Only clean up queries that are in OUR circuit
    let to_cleanup: Vec<String> = expired_ids
        .into_iter()
        .filter_map(|id| {
            let raw = id
                .strip_prefix("_00_query:")
                .unwrap_or(&id)
                .to_string();
            if view_ids.contains(&raw) { Some(raw) } else { None }
        })
        .collect();

    let count = to_cleanup.len();
    for query_id in to_cleanup {
        cleanup_expired_query(db, processor, metrics, &query_id).await;
    }

    if count > 0 {
        info!(count = count, "TTL cleanup sweep completed");
    }
    count
}

// --- Helper Functions ---

/// Parse a record ID string into SurrealDB RecordId
fn parse_record_id(id: &str) -> Option<RecordId> {
    RecordId::parse_simple(id).ok()
}

/// Format incantation ID with proper prefix
fn format_incantation_id(id: &str) -> String {
    if id.starts_with("_00_query:") {
        id.to_string()
    } else {
        format!("_00_query:{}", id)
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
                "RELATE ${1}->_00_list_ref->{0} SET version = {2}, clientId = (SELECT VALUE clientId FROM ${1} LIMIT 1)[0]",
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
                "UPDATE _00_list_ref SET version = {2} WHERE in = ${0} AND out = {1}",
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
                "DELETE ${1}->_00_list_ref WHERE out = {0}",
                id, binding_name
            ));
        }

        // Process subquery items (child records linked to parents via parent/parent_rel)
        // These are processed AFTER main records so parent list_ref entries exist in the same tx.
        for item in &delta.subquery_items {
            if parse_record_id(&item.id).is_none() {
                error!(
                    target: "ssp::edges",
                    record_id = %item.id,
                    view_id = %delta.query_id,
                    "Invalid subquery record ID format - skipping"
                );
                continue;
            }

            match item.op {
                SubqueryOp::Add => {
                    let version = circuit.store.get_record_version_by_key(&item.id).unwrap_or(1);
                    created_count += 1;
                    all_statements.push(format!(
                        "RELATE ${binding}->_00_list_ref->{id} SET \
                         version = {version}, \
                         clientId = (SELECT VALUE clientId FROM ${binding} LIMIT 1)[0], \
                         parent = (SELECT VALUE id FROM _00_list_ref WHERE in = ${binding} AND out = {parent} LIMIT 1)[0], \
                         parent_rel = '{alias}'",
                        binding = binding_name,
                        id = item.id,
                        version = version,
                        parent = item.parent_key,
                        alias = item.alias,
                    ));
                }
                SubqueryOp::Update => {
                    let version = circuit.store.get_record_version_by_key(&item.id).unwrap_or(1);
                    updated_count += 1;
                    all_statements.push(format!(
                        "UPDATE _00_list_ref SET version = {version} WHERE in = ${binding} AND out = {id}",
                        binding = binding_name, id = item.id, version = version
                    ));
                }
                SubqueryOp::Remove => {
                    deleted_count += 1;
                    all_statements.push(format!(
                        "DELETE ${binding}->_00_list_ref WHERE out = {id}",
                        binding = binding_name, id = item.id
                    ));
                }
            }
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
