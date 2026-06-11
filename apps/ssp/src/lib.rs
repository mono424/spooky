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

use std::collections::BTreeMap;
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
pub mod crdt;
pub mod metrics;
pub mod open_telemetry;
pub mod tables;
pub mod view_metrics;

use metrics::Metrics;
use view_metrics::{ViewMetrics, ViewMetricsState};

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
    pub crdt_cache: Arc<crdt::CrdtCache>,
    /// Per-view rolling materialization-latency samples plus running
    /// update/error counters. Persisted onto `_00_query` after each
    /// affected ingest step.
    pub view_metrics: Arc<ViewMetrics>,
    /// `_00_query` / `_00_list_ref` storage layout. See
    /// `ssp_protocol::RefMode` and `crate::tables`.
    pub ref_mode: ssp_protocol::RefMode,
    /// Version of the upstream SurrealDB server, queried once at startup
    /// (`"unknown"` if the query failed). Surfaced via `/info` so the DevTools
    /// can report the SurrealDB backend version.
    pub surrealdb_version: String,
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
    pub scheduler_url: Option<String>,
    pub ssp_id: String,
    pub heartbeat_interval_ms: u64,
    pub advertise_addr: Option<String>,
    pub ttl_cleanup_interval_secs: u64,
    /// Storage layout for `_00_query` / `_00_list_ref`. See
    /// `ssp_protocol::RefMode`. Defaults to `Dedicated` so cross-session
    /// LIVE delivery doesn't depend on the SurrealDB v3 LIVE-permission
    /// path; flip to `Single` only when running against a SurrealDB
    /// version that delivers cross-session LIVE notifications correctly
    /// through permission rules.
    pub ref_mode: ssp_protocol::RefMode,
}

pub fn load_config() -> Config {
    Config {
        listen_addr: std::env::var("SPKY_SSP_LISTEN_ADDR").unwrap_or_else(|_| "0.0.0.0:8667".to_string()),
        db_addr: std::env::var("SPKY_DB_WS").unwrap_or_else(|_| "ws://127.0.0.1:8000".to_string()),
        db_user: std::env::var("SPKY_DB_USER").unwrap_or_else(|_| "root".to_string()),
        db_pass: std::env::var("SPKY_DB_PASS").unwrap_or_else(|_| "root".to_string()),
        db_ns: std::env::var("SPKY_DB_NS").unwrap_or_else(|_| "test".to_string()),
        db_db: std::env::var("SPKY_DB_NAME").unwrap_or_else(|_| "test".to_string()),
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
        ref_mode: std::env::var("SPKY_SSP_REF_MODE")
            .ok()
            .as_deref()
            .and_then(ssp_protocol::RefMode::parse_str)
            .unwrap_or_default(),
    }
}

// --- Scheduler Registration Helper ---

/// Build the SSP registration payload and POST it to the scheduler.
/// Returns the registration response (which carries `snapshot_seq` and the
/// per-table content hashes the SSP must match after bootstrap) or an error.
async fn register_with_scheduler(
    client: &reqwest::Client,
    scheduler_url: &str,
    ssp_id: &str,
    listen_addr: &str,
    advertise_addr: Option<&str>,
) -> Result<ssp_protocol::SspRegistrationResponse, String> {
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
        Ok(resp) if resp.status().is_success() => resp
            .json::<ssp_protocol::SspRegistrationResponse>()
            .await
            .map_err(|e| format!("Failed to parse registration response: {}", e)),
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
                // `into_json_value()` flattens RecordId/Datetime to plain
                // strings and unwraps SurrealDB's tagged Value enum into
                // ordinary JSON. `serde_json::to_value(&val)` (the previous
                // implementation) emitted the tagged shape `{"Object": {
                // "tables": {"Object": {...}}}}`, which broke
                // `info_json.get("tables")` and made the bootstrap report
                // an empty table list against a populated DB.
                Ok(val.into_json_value())
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

    let addr = config.db_addr
        .strip_prefix("ws://")
        .or_else(|| config.db_addr.strip_prefix("wss://"))
        .unwrap_or(&config.db_addr);

    let db = Surreal::new::<Ws>(addr)
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

/// Load job config from the SPKY_JOB_CONFIG env var (JSON).
/// Format: [{"name":"api","table":"job","base_url":"http://...","auth_token":null,"timeout":10,"timeout_overridable":false}, ...]
fn load_job_config_from_env() -> Arc<JobConfig> {
    let json = match std::env::var("SPKY_JOB_CONFIG") {
        Ok(v) if !v.is_empty() => v,
        _ => {
            info!("No SPKY_JOB_CONFIG set, job runner disabled");
            return Arc::new(JobConfig::default());
        }
    };

    let entries: Vec<serde_json::Value> = match serde_json::from_str(&json) {
        Ok(v) => v,
        Err(e) => {
            warn!(error = %e, "Failed to parse SPKY_JOB_CONFIG, job runner disabled");
            return Arc::new(JobConfig::default());
        }
    };

    let mut job_tables = std::collections::HashMap::new();
    for entry in &entries {
        let name = entry.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let table = entry.get("table").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let base_url = entry.get("base_url").and_then(|v| v.as_str()).unwrap_or("").to_string();
        if table.is_empty() || base_url.is_empty() { continue; }
        let auth_token = entry.get("auth_token").and_then(|v| v.as_str()).map(|s| s.to_string());
        let timeout = entry.get("timeout").and_then(|v| v.as_u64()).map(|v| v as u32);
        let timeout_overridable = entry.get("timeout_overridable").and_then(|v| v.as_bool()).unwrap_or(false);
        job_tables.insert(table, job_runner::BackendInfo { name, base_url, auth_token, timeout, timeout_overridable });
    }

    info!(job_tables = job_tables.len(), "Loaded job config from SPKY_JOB_CONFIG");
    Arc::new(JobConfig { job_tables })
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
        .route("/crdt/apply", post(crdt_apply_handler))
        .route("/reset", post(reset_handler))
        .layer(middleware::from_fn(auth_middleware));

    // Public routes — no auth required (health checks, info, version).
    // A permissive CORS header lets the browser DevTools read these
    // cross-origin (simple GETs, so no preflight is needed).
    let public = Router::new()
        .route("/health", get(health_handler))
        .route("/info", get(info_handler))
        .route("/info/text", get(info_text_handler))
        .route("/version", get(version_handler))
        .layer(middleware::from_fn(cors_allow_all));

    authenticated.merge(public).with_state(state)
}

/// Add `Access-Control-Allow-Origin: *` so browser clients (e.g. the DevTools
/// extension's page context) can read the public info/version responses.
async fn cors_allow_all(req: Request, next: Next) -> Response {
    let mut res = next.run(req).await;
    res.headers_mut().insert(
        axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN,
        axum::http::HeaderValue::from_static("*"),
    );
    res
}

// --- Server Lifecycle ---

pub async fn run_server() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    // Initialize observability
    open_telemetry::init_tracing().context("Failed to initialize OpenTelemetry tracing")?;
    let (meter_provider, metrics) =
        metrics::init_metrics().context("Failed to initialize metrics")?;
    let metrics = Arc::new(metrics);

    info!(
        "\n ____  ____  ____\n/ ___)/ ___)(  _ \\\n\\___ \\\\___ \\ ) __/\n(____/(____/(__)    v{}\n\nSp00ky Sync Provider — streaming mode\nBuilt: {}",
        env!("CARGO_PKG_VERSION"),
        env!("SPOOKY_BUILD_TIMESTAMP"),
    );

    let config = load_config();
    let db = connect_database(&config).await?;

    // Query the upstream SurrealDB server version once; surfaced via `/info`.
    let surrealdb_version = match db.version().await {
        Ok(v) => v.to_string(),
        Err(e) => {
            info!(error = %e, "Could not read SurrealDB server version");
            "unknown".to_string()
        }
    };

    // Start with an empty circuit — self-bootstrap will populate it
    let processor_arc = Arc::new(RwLock::new(Circuit::new()));
    let status = Arc::new(RwLock::new(SspStatus::Bootstrapping));

    // Load job configuration from SPKY_JOB_CONFIG env var
    let job_config = load_job_config_from_env();

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

    let crdt_cache_capacity = std::env::var("SPKY_CRDT_CACHE_SIZE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1024);
    let crdt_cache = Arc::new(crdt::CrdtCache::new(
        crdt_cache_capacity,
        crdt::CrdtAllowList::from_env(),
    ));

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
        crdt_cache,
        view_metrics: Arc::new(RwLock::new(std::collections::HashMap::new())),
        ref_mode: config.ref_mode,
        surrealdb_version,
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
            // Choose bootstrap source based on mode. The metadata source
            // (INFO FOR DB → permissions) is always upstream SurrealDB
            // regardless of mode, because the scheduler's replica is
            // records-only and won't carry DEFINE TABLE strings.
            let metadata_source = BootstrapSource::Direct(db.clone());
            let (data_source, expected_hashes) = if let Some(ref scheduler_url) = scheduler_url {
                // Cluster mode: register with scheduler, then bootstrap from proxy
                let client = reqwest::Client::new();
                let scheduler_base = scheduler_url.trim_end_matches('/');

                info!("Registering SSP {} with scheduler at {}", ssp_id, scheduler_base);

                let registration = match register_with_scheduler(
                    &client,
                    scheduler_url,
                    &ssp_id,
                    &listen_addr,
                    advertise_addr.as_deref(),
                ).await {
                    Ok(r) => {
                        info!(
                            snapshot_seq = r.snapshot_seq,
                            tables = r.table_hashes.len(),
                            "Successfully registered with scheduler"
                        );
                        r
                    }
                    Err(e) => {
                        error!("Failed to register with scheduler: {}", e);
                        *status.write().await = SspStatus::Failed;
                        return;
                    }
                };

                let proxy_url = format!("{}/proxy", scheduler_base);
                info!("Bootstrapping from scheduler proxy at {}", proxy_url);
                (
                    BootstrapSource::Proxy { client, proxy_url },
                    registration.table_hashes,
                )
            } else {
                // Standalone mode: bootstrap directly from DB. No expected
                // hashes — verification only applies in cluster mode.
                info!("Standalone mode: bootstrapping from SurrealDB");
                (BootstrapSource::Direct(db), BTreeMap::new())
            };

            // Retry bootstrap up to 10 times with backoff (tables may not exist yet
            // if migrations haven't run)
            let mut attempt = 0;
            loop {
                attempt += 1;
                match self_bootstrap_with_metadata(&metadata_source, &data_source, &processor).await {
                    Ok(()) => {
                        // Integrity check: only when the scheduler handed us
                        // expected hashes (cluster mode). Mismatch ⇒ wipe
                        // the circuit and retry once. Second failure exits
                        // the process; supervisor restarts → fresh
                        // registration with the current frozen snapshot.
                        if !expected_hashes.is_empty() {
                            let actual = {
                                let guard = processor.read().await;
                                guard.compute_table_hashes()
                            };
                            let diffs = ssp_protocol::snapshot_hash::diff_table_hashes(
                                &expected_hashes,
                                &actual,
                            );
                            if !diffs.is_empty() {
                                for d in &diffs {
                                    error!(
                                        table = %d.table,
                                        expected = %d.a,
                                        actual = %d.b,
                                        "Bootstrap integrity mismatch"
                                    );
                                }
                                if attempt >= 2 {
                                    error!(
                                        attempts = attempt,
                                        diffs = diffs.len(),
                                        "Integrity mismatch persisted after retry — exiting for restart"
                                    );
                                    *status.write().await = SspStatus::Failed;
                                    // Exit so the supervisor restarts us
                                    // with a clean circuit and fresh
                                    // registration handshake.
                                    std::process::exit(2);
                                }
                                warn!(
                                    attempt,
                                    diffs = diffs.len(),
                                    "Wiping circuit and retrying bootstrap"
                                );
                                {
                                    let mut guard = processor.write().await;
                                    *guard = Circuit::new();
                                }
                                continue;
                            }
                        }

                        let guard = processor.read().await;
                        metrics.view_count.add(guard.view_count() as i64, &[]);
                        info!(
                            tables = guard.table_names().len(),
                            views = guard.view_count(),
                            verified = !expected_hashes.is_empty(),
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
        // listen_addr / advertise_addr aren't read here — the heartbeat
        // path no longer self-registers; if the scheduler 404s us, we
        // exit and the supervisor reruns the full register handshake.
        let status_for_heartbeat = status.clone();

        tokio::spawn(async move {
            let client = reqwest::Client::new();
            let heartbeat_url = format!("{}/ssp/heartbeat", scheduler_url_clone.trim_end_matches('/'));
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(heartbeat_interval));

            loop {
                interval.tick().await;

                // Don't heartbeat until the bootstrap task has finished
                // registering and verifying. Otherwise the first tick can
                // race ahead of `POST /ssp/register`, hit the scheduler as
                // an unknown SSP, get 404, and we'd exit while our own
                // registration is still in flight.
                if *status_for_heartbeat.read().await != SspStatus::Ready {
                    continue;
                }

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
                        warn!("Scheduler doesn't recognize us, exiting for clean restart");
                        // The scheduler has dropped us. Restarting through
                        // the supervisor re-runs the bootstrap loop above,
                        // which re-registers and re-verifies hashes — much
                        // safer than trying to re-register from a heartbeat
                        // task that can't replay events into the circuit.
                        std::process::exit(3);
                    }
                    Ok(resp) if resp.status() == StatusCode::CONFLICT => {
                        // Either buffer overflow or scheduler-driven
                        // integrity-check resync. Either way the circuit
                        // can't be trusted; exit so the supervisor brings
                        // us back with a clean state.
                        let body = resp.text().await.unwrap_or_default();
                        error!(reason = %body, "Scheduler requested re-bootstrap, exiting");
                        std::process::exit(4);
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
        let ref_mode = config.ref_mode;

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

                ttl_cleanup_sweep(&db, &processor, &metrics, ref_mode).await;
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
/// Bootstrap with separate sources for metadata (INFO FOR DB) and data
/// (record SELECTs). In cluster mode the scheduler's proxy backs onto its
/// RocksDB replica, which only stores records — DEFINE TABLE / PERMISSIONS
/// strings never land there, so `INFO FOR DB` against the proxy returns
/// at most an empty or schemaless view of each table. That breaks the
/// SSP's policy bootstrap (every table comes back without a registered
/// permission and gets default-denied at view-register time). Upstream
/// SurrealDB is the source of truth for schema, so callers in cluster
/// mode pass it explicitly here as `metadata_source`.
async fn self_bootstrap_with_metadata(
    metadata_source: &BootstrapSource,
    data_source: &BootstrapSource,
    processor: &Arc<RwLock<Circuit>>,
) -> anyhow::Result<()> {
    info!("Starting self-bootstrap");
    let source = data_source;

    // Step 1: Discover tables via INFO FOR DB (against upstream so we get
    // the real DEFINE TABLE strings with PERMISSIONS clauses).
    let info_json = metadata_source.query("INFO FOR DB").await
        .context("Failed to query INFO FOR DB")?;

    // INFO FOR DB returns `tables: { name: "DEFINE TABLE ... PERMISSIONS ...;" }`.
    // Keep both the table list (for data-loading) and the raw DEFINE strings
    // (for permission extraction below).
    let table_defs: Vec<(String, String)> = match info_json.get("tables") {
        Some(Value::Object(tables_map)) => tables_map
            .iter()
            .filter(|(name, _)| !name.starts_with("_00_"))
            .map(|(name, def)| {
                (name.clone(), def.as_str().unwrap_or("").to_string())
            })
            .collect(),
        _ => {
            info!("No tables found in database");
            vec![]
        }
    };
    let tables: Vec<String> = table_defs.iter().map(|(n, _)| n.clone()).collect();

    info!(count = tables.len(), "Discovered tables: {:?}", tables);

    // Step 1b: Pull `PERMISSIONS FOR select WHERE <expr>` text out of each
    // DEFINE TABLE string and stash it on the circuit. Stored as raw text so
    // `prepare_registration_dbsp` can route it through the same converter that
    // handles user queries (see permission_inject.rs).
    {
        let mut circuit = processor.write().await;
        for (name, def) in &table_defs {
            let permission = extract_select_permission_text(def);
            info!(
                target: "ssp::policy",
                table = %name,
                permission = %permission,
                "registered table permission"
            );
            circuit.set_permission(name, permission);
        }
    }

    // Step 2: Load all table data, paged. Pulling the entire table in one
    // request blew up at multi-GB DBs because either the SurrealDB Rust SDK's
    // Ws engine (64 MiB tungstenite frame ceiling) or the scheduler proxy
    // had to materialise the full result set in one HTTP body. Paging keeps
    // each round-trip bounded; the SSP still loads everything into the
    // circuit store but does so a chunk at a time.
    //
    // Default page is 200 records; override via SPKY_SSP_BOOTSTRAP_PAGE_SIZE.
    let page_size: usize = std::env::var("SPKY_SSP_BOOTSTRAP_PAGE_SIZE")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|n: &usize| *n > 0)
        .unwrap_or(200);

    for table in &tables {
        let mut record_count: usize = 0;
        let mut start: usize = 0;
        loop {
            let result = source
                .query(&format!(
                    "SELECT * FROM {} LIMIT {} START {}",
                    table, page_size, start
                ))
                .await
                .with_context(|| format!("Failed to page-query table {}", table))?;

            let rows: Vec<Value> = match result {
                Value::Array(arr) => arr,
                _ => vec![],
            };
            let n = rows.len();
            if n == 0 {
                break;
            }

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

            record_count += n;
            if n < page_size {
                break;
            }
            start += page_size;
        }
        info!(table = %table, records = record_count, "Loaded table data");
    }

    // Step 3: Re-register views from the global `_00_query` table.
    // `_00_query` is global in both modes (only `_00_list_ref` splits
    // per user); each row carries an `auth_id` field which decides
    // where the corresponding `_00_list_ref_user_<id>` writes go.
    let result = source.query("SELECT * FROM _00_query").await
        .context("Failed to query _00_query")?;
    let views: Vec<Value> = match result {
        Value::Array(arr) => arr,
        _ => vec![],
    };
    info!(count = views.len(), "Found persisted views in _00_query");

    for view_row in views {
        let view_id = match view_row.get("id") {
            Some(Value::String(s)) => s.clone(),
            Some(v) => v.to_string().trim_matches('"').to_string(),
            None => {
                warn!("Skipping view with missing id");
                continue;
            }
        };

        // Strip the table prefix (e.g. "_00_query:abc" → "abc").
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
        let auth_id = view_row
            .get("auth_id")
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
            "authId": auth_id,
            "ttl": ttl,
            "lastActiveAt": last_active_at,
            "params": params,
        });

        let prep = {
            let circuit = processor.read().await;
            ssp::service::view::prepare_registration_dbsp(payload, circuit.permissions())
        };
        match prep {
            Ok(data) => {
                let mut circuit = processor.write().await;
                circuit.add_query_with_auth(
                    data.plan,
                    data.safe_params,
                    Some(OutputFormat::Streaming),
                    auth_id.clone(),
                );
                info!(view_id = %raw_id, auth_id = %auth_id, "Re-registered view");
            }
            Err(e) => {
                warn!(
                    target: "ssp::policy",
                    view_id = %raw_id,
                    error = %e,
                    "Failed to re-register view; the owner client will get the same error on next register"
                );
            }
        }
    }

    Ok(())
}

/// Pull the raw `PERMISSIONS FOR select WHERE <expr>` text out of a
/// `DEFINE TABLE` statement.
///
/// Returns the body of the `WHERE` clause (no leading `WHERE`), or one of the
/// sentinels `"true"` / `"false"` for the bare-keyword and missing-action
/// cases. This text is later spliced into a synthetic
/// `SELECT * FROM <table> WHERE <text>` and routed through the shared
/// converter, so it must be valid SurrealQL `WHERE` syntax. See
/// `permission_inject.rs` for how it gets used.
///
/// Behavior:
/// - `WHERE true` or no PERMISSIONS clause at all (SurrealDB defaults to FULL)
///   -> `"true"`.
/// - `PERMISSIONS NONE` or no `select` action -> `"false"` (deny).
/// - `FOR select WHERE <expr>` -> the raw `<expr>` text.
fn extract_select_permission_text(define_table: &str) -> String {
    let def = define_table.trim().trim_end_matches(';');
    let upper = def.to_uppercase();

    // No PERMISSIONS clause → SurrealDB defaults to FULL → allow.
    let Some(perm_idx) = upper.find("PERMISSIONS") else {
        return "true".into();
    };
    let perm_section = def[perm_idx + "PERMISSIONS".len()..].trim();
    let perm_upper = perm_section.to_uppercase();

    if perm_upper.starts_with("FULL") {
        return "true".into();
    }
    if perm_upper.starts_with("NONE") {
        return "false".into();
    }

    let lower = perm_section.to_lowercase();
    let mut clause_starts: Vec<usize> = Vec::new();
    for (i, _) in lower.match_indices("for ") {
        if i == 0 || lower.as_bytes()[i - 1].is_ascii_whitespace() {
            clause_starts.push(i);
        }
    }
    if clause_starts.is_empty() {
        warn!(target: "ssp::policy", def = %def, "PERMISSIONS clause has no FOR clauses; denying");
        return "false".into();
    }

    for (idx, &start) in clause_starts.iter().enumerate() {
        let end = clause_starts.get(idx + 1).copied().unwrap_or(perm_section.len());
        let clause = &perm_section[start..end];
        let lower_clause = clause.to_lowercase();
        let where_idx = lower_clause.find("where");
        let header = match where_idx {
            Some(w) => &clause[..w],
            None => clause,
        };
        let header_lower = header.to_lowercase();
        if !header_lower.contains("select") {
            continue;
        }
        let Some(w) = where_idx else {
            // FOR select with no WHERE - SurrealDB treats this as full-allow.
            return "true".into();
        };
        // Strip leading "WHERE" keyword and trailing comma/semicolon/whitespace.
        let body = clause[w + "where".len()..]
            .trim()
            .trim_end_matches(',')
            .trim_end_matches(';')
            .trim()
            .to_string();
        if body.is_empty() {
            return "true".into();
        }
        return body;
    }

    // SurrealDB default for unlisted actions is DENY.
    "false".into()
}

#[cfg(test)]
mod permission_extract_tests {
    use super::*;

    #[test]
    fn no_permissions_clause_is_full() {
        assert_eq!(
            extract_select_permission_text("DEFINE TABLE foo SCHEMAFULL;"),
            "true"
        );
    }

    #[test]
    fn permissions_full_is_true() {
        assert_eq!(
            extract_select_permission_text("DEFINE TABLE foo SCHEMAFULL PERMISSIONS FULL"),
            "true"
        );
    }

    #[test]
    fn permissions_none_is_false() {
        assert_eq!(
            extract_select_permission_text("DEFINE TABLE foo SCHEMAFULL PERMISSIONS NONE"),
            "false"
        );
    }

    #[test]
    fn for_select_where_true_is_true_text() {
        assert_eq!(
            extract_select_permission_text(
                "DEFINE TABLE foo SCHEMAFULL PERMISSIONS FOR select WHERE true",
            ),
            "true"
        );
    }

    #[test]
    fn for_select_where_false_is_false_text() {
        assert_eq!(
            extract_select_permission_text(
                "DEFINE TABLE foo SCHEMAFULL PERMISSIONS FOR select WHERE false",
            ),
            "false"
        );
    }

    #[test]
    fn for_other_action_only_denies_select() {
        assert_eq!(
            extract_select_permission_text(
                "DEFINE TABLE foo SCHEMAFULL PERMISSIONS FOR update WHERE true",
            ),
            "false"
        );
    }

    #[test]
    fn extracts_select_auth_eq_body() {
        let text = extract_select_permission_text(
            "DEFINE TABLE thread SCHEMAFULL PERMISSIONS FOR select WHERE author.id = $auth.id FOR update WHERE false",
        );
        assert_eq!(text, "author.id = $auth.id");
    }

    #[test]
    fn extracts_unparseable_text_verbatim() {
        // The boot loader extracts text and the registration step decides
        // whether it can represent it. Boundary check - subquery WHERE comes
        // through as raw text.
        let text = extract_select_permission_text(
            "DEFINE TABLE thread SCHEMAFULL PERMISSIONS FOR select WHERE $auth.id IN (SELECT id FROM x)",
        );
        assert_eq!(text, "$auth.id IN (SELECT id FROM x)");
    }

    #[test]
    fn select_in_action_list() {
        assert_eq!(
            extract_select_permission_text(
                "DEFINE TABLE foo PERMISSIONS FOR create, select, update WHERE true",
            ),
            "true"
        );
    }

    #[test]
    fn extracts_complex_select_with_param_lhs() {
        // The exact shape used by the example app's thread permission. The
        // text comes out verbatim; the converter decides whether it parses.
        let text = extract_select_permission_text(
            "DEFINE TABLE thread SCHEMAFULL PERMISSIONS FOR select WHERE published = true OR $access = 'account' AND author.id = $auth.id, FOR create, delete WHERE $access = 'account' AND author.id = $auth.id",
        );
        assert_eq!(
            text,
            "published = true OR $access = 'account' AND author.id = $auth.id"
        );
    }

    #[test]
    fn for_select_no_where_is_true() {
        // `PERMISSIONS FOR select` with no WHERE is full-allow per SurrealDB.
        assert_eq!(
            extract_select_permission_text(
                "DEFINE TABLE foo SCHEMAFULL PERMISSIONS FOR select",
            ),
            "true"
        );
    }
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

    // Pre-emptively create the new user's dedicated `_00_list_ref_user_<id>`
    // and `_00_query_user_<id>` tables (idempotent — no-op in
    // `RefMode::Single`). Without this, the client's `setCurrentUserId`
    // call after auth-success races the SSP's lazy table creation in
    // `register_view_handler` and the initial `LIVE SELECT * FROM
    // _00_list_ref_user_<id>` fails with "table not found" until the
    // retry backoff in `Sp00kySync::setCurrentUserId` kicks in.
    if payload.table == "user" && op == Operation::Create {
        if let Err(e) =
            tables::ensure_user_tables(&state.db, state.ref_mode, &payload.id).await
        {
            warn!(
                target: "ssp::ingest",
                error = %e,
                auth_id = %payload.id,
                "Pre-emptive ensure_user_tables failed; falling back to lazy creation on view register"
            );
        }
    }

    // Inverse of pre-emptive creation: when the owning user record is
    // deleted, drop the dedicated `_00_list_ref_user_<id>` so leftover
    // schema state doesn't accumulate forever. The schema's
    // `_00_user_delete` event fires this ingest with op=DELETE and the
    // pre-delete row payload, which carries the same id sanitization
    // path. No-op in `RefMode::Single` (no per-user tables exist).
    if payload.table == "user" && op == Operation::Delete {
        if let Err(e) =
            tables::drop_user_tables(&state.db, state.ref_mode, &payload.id).await
        {
            warn!(
                target: "ssp::ingest",
                error = %e,
                auth_id = %payload.id,
                "drop_user_tables failed; per-user list_ref table will linger"
            );
        }
    }

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
    let step_start = std::time::Instant::now();
    let deltas = {
        let mut circuit = state.processor.write().await;
        circuit.step(ChangeSet { changes: vec![change] })
    };
    let materialization_time_ms = step_start.elapsed().as_secs_f64() * 1000.0;

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

        // Defer the actual `UPDATE _00_list_ref_user_<...>` writes
        // into a background task and return 200 to the schema event
        // immediately. Without this, the schema's `http::post` blocks
        // alice's parent transaction until `update_all_edges` completes
        // and commits — in a SEPARATE transaction over the SSP's own
        // connection. That separate tx commits BEFORE alice's parent
        // tx, so when bob's LIVE on `_00_list_ref_user_<bob>` fires
        // and his client re-fetches the row over a different session,
        // alice's UPDATE on the source row isn't visible yet and bob
        // ingests stale content with the new list_ref version (see
        // docs/surrealdb-bugs/ws-row-cache-stale-after-update.md).
        //
        // By spawning the work and waiting on `_00_version` (which is
        // bumped inside alice's tx and only becomes visible to other
        // connections post-commit) we ensure the list_ref UPDATE
        // lands AFTER the source row is readable downstream.
        let expected_version: Option<i64> = payload
            .record
            .get("_00_rv")
            .and_then(|v| v.as_i64())
            .filter(|&v| v > 0);
        let row_id_for_wait = payload.id.clone();
        let table_for_wait = payload.table.clone();
        let op_for_wait = payload.op.clone();
        let state_for_task = state.clone();
        let deltas_for_task = deltas;
        let span_for_task = span.clone();
        tokio::spawn(async move {
            let _enter = span_for_task.enter();
            if let Some(expected) = expected_version {
                if !wait_for_row_committed(
                    &state_for_task.db,
                    &row_id_for_wait,
                    expected,
                    std::time::Duration::from_secs(5),
                )
                .await
                {
                    warn!(
                        target: "ssp::ingest",
                        table = %table_for_wait,
                        op = %op_for_wait,
                        id = %row_id_for_wait,
                        expected_version = expected,
                        "Timed out waiting for source row to commit; proceeding with edges update"
                    );
                }
            }
            let delta_refs: Vec<&ViewDelta> = deltas_for_task.iter().collect();
            let circuit = state_for_task.processor.read().await;
            update_all_edges(
                &state_for_task.db,
                &delta_refs,
                &state_for_task.metrics,
                &circuit,
                state_for_task.ref_mode,
            )
            .await;
            drop(circuit);
            persist_view_metrics(
                &state_for_task,
                deltas_for_task.iter().map(|d| d.records.len()).collect::<Vec<_>>(),
                deltas_for_task
                    .iter()
                    .map(|d| d.query_id.clone())
                    .collect::<Vec<_>>(),
                materialization_time_ms,
            )
            .await;
        });
    }

    // Orphan-proof delete. A deleted record must not remain in ANY query's
    // `_00_list_ref`, but the circuit only emits a removal when the record is in
    // its in-memory `view.cache` — which can be incomplete (a missed ingest, or
    // an SSP restart while `_00_list_ref` persists in SurrealDB). Those leftover
    // edges are exactly why a delete in one window never reaches another window:
    // the deleted id never leaves the other session's `_00_list_ref`, so that
    // client sees no removal to react to. Independently of the circuit deltas,
    // drop every edge pointing at the deleted record from the owner's per-user
    // list_ref table. Idempotent: a no-op when the circuit already removed them.
    if op == Operation::Delete {
        if let Some(owner) = payload.record.get("owner").and_then(|v| v.as_str()) {
            if parse_record_id(&payload.id).is_some() {
                let list_ref_tbl = tables::list_ref_table(state.ref_mode, owner);
                // Interpolate the record id directly (it's a validated record-id
                // literal), matching the edge-write statements in `update_all_edges`.
                let stmt = format!("DELETE {} WHERE out = {}", list_ref_tbl, payload.id);
                let state_for_cleanup = state.clone();
                let id_for_log = payload.id.clone();
                tokio::spawn(async move {
                    if let Err(e) = state_for_cleanup.db.query(&stmt).await {
                        error!(
                            target: "ssp::ingest",
                            id = %id_for_log,
                            error = %e,
                            "list_ref delete cleanup failed"
                        );
                    } else {
                        debug!(
                            target: "ssp::ingest",
                            id = %id_for_log,
                            "Removed list_ref edges for deleted record"
                        );
                    }
                });
            }
        }
    }

    // Record duration
    let duration_ms = start.elapsed().as_secs_f64() * 1000.0;
    state.metrics.ingest_duration.record(duration_ms, &[]);

    StatusCode::OK.into_response()
}

/// Poll `_00_version` on the SSP's own connection until the source row
/// reaches `expected_version`. The schema event bumps `_00_version`
/// inside alice's parent transaction; from the SSP's connection that
/// bump is only visible AFTER alice's tx commits, so this is a clean
/// proxy for "alice's source row is now readable downstream". Returns
/// `true` if the version is observed within `timeout`, `false` on
/// timeout (caller proceeds anyway so a missing `_00_version` row
/// doesn't permanently wedge the fan-out).
async fn wait_for_row_committed(
    db: &SharedDb,
    row_id: &str,
    expected_version: i64,
    timeout: std::time::Duration,
) -> bool {
    let Some(rid) = parse_record_id(row_id) else {
        return false;
    };
    let start = std::time::Instant::now();
    let mut backoff_ms: u64 = 10;
    while start.elapsed() < timeout {
        match db
            .query("SELECT VALUE version FROM ONLY _00_version WHERE record_id = $rid LIMIT 1")
            .bind(("rid", rid.clone()))
            .await
        {
            Ok(mut response) => {
                let v: Option<i64> = response.take(0).ok().flatten();
                if let Some(v) = v {
                    if v >= expected_version {
                        return true;
                    }
                }
            }
            Err(e) => {
                debug!(
                    target: "ssp::ingest",
                    error = %e,
                    row_id,
                    "Source-row commit poll query failed; retrying"
                );
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
        if backoff_ms < 80 {
            backoff_ms *= 2;
        }
    }
    false
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

    // Parse and validate the registration. The per-table permission text is
    // stashed on the Circuit by self_bootstrap; reading it under the read lock
    // keeps the registration path off the write lock until we actually mutate
    // the circuit below. Permission-injection failures (default-deny, missing
    // $auth, unrepresentable expression) come back as a 400 with the offending
    // table named so the client can fix its schema or pass the missing param.
    let data = {
        let circuit = state.processor.read().await;
        match ssp::service::view::prepare_registration_dbsp(payload, circuit.permissions()) {
            Ok(d) => d,
            Err(e) => {
                error!(target: "ssp::policy", error = %e, "Rejected view registration");
                return (StatusCode::BAD_REQUEST, e.to_string()).into_response();
            }
        }
    };

    span.record("view_id", &data.plan.id);

    // Auth identity for per-user table routing. Extracted up front so
    // every downstream record-id computation uses the same value.
    let auth_id = data
        .metadata
        .get("authId")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // Lazy-define the per-user `_00_query_user_<id>` and
    // `_00_list_ref_user_<id>` tables (idempotent). No-op in
    // `RefMode::Single`. Done before the UPSERT below so the table
    // exists when we try to write into it.
    if let Err(e) = tables::ensure_user_tables(&state.db, state.ref_mode, &auth_id).await {
        error!(error = %e, auth_id = %auth_id, "Failed to ensure per-user tables");
        return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
    }

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

        // Still update the _00_query record for fresh
        // clientId/lastActiveAt/auth_id. auth_id can shift across
        // re-registrations if the same WS session re-authenticates,
        // and we want list_ref entries written after this point to
        // carry the latest user attribution.
        let client_id = data.metadata.get("clientId").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let last_active_at = data.metadata.get("lastActiveAt").and_then(|v| v.as_str()).unwrap_or("").to_string();

        let query = "UPDATE <record>$id SET clientId = <string>$clientId, auth_id = <string>$authId, lastActiveAt = <datetime>$lastActiveAt";
        if let Err(e) = state.db.query(query)
            .bind(("id", incantation_id.clone()))
            .bind(("clientId", client_id))
            .bind(("authId", auth_id.clone()))
            .bind(("lastActiveAt", last_active_at))
            .await
        {
            error!("Failed to update incantation metadata: {}", e);
        }

        return StatusCode::OK.into_response();
    }

    debug!("Registering view: {}", data.plan.id);

    // Register view with Streaming format
    let register_start = std::time::Instant::now();
    let update = {
        let mut circuit = state.processor.write().await;
        circuit.add_query_with_auth(
            data.plan.clone(),
            data.safe_params,
            Some(OutputFormat::Streaming),
            auth_id.clone(),
        )
    };
    let registration_time_ms = register_start.elapsed().as_secs_f64() * 1000.0;

    state.metrics.view_count.add(1, &[]);

    // Seed an empty per-view metrics state so the first ingest doesn't have
    // to take the write lock to insert.
    {
        let mut metrics_map = state.view_metrics.write().await;
        metrics_map
            .entry(data.plan.id.clone())
            .or_insert_with(ViewMetricsState::default);
    }

    // Extract metadata fields. `auth_id` was already pulled earlier
    // to drive table routing; the others go on the `_00_query[_user_*]`
    // row as observability metadata.
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

    // Capture initial row count from the snapshot delta so `_00_query.rowCount`
    // reflects the materialized view's content immediately after register,
    // not just after the first per-ingest update. Without this the row
    // count stays at the schema default 0 until something mutates the
    // upstream tables — confusing when the view actually has rows
    // already, e.g. on page reload.
    let initial_row_count: i64 = update
        .as_ref()
        .map(|d| d.records.len() as i64)
        .unwrap_or(0);

    // Store incantation metadata. createdAt has DEFAULT time::now() and is
    // READONLY so it's only set on the inserting branch of the UPSERT;
    // counters default to 0 if absent.
    let query = "UPSERT <record>$id SET clientId = <string>$clientId, auth_id = <string>$authId, surql = <string>$surql, params = $params, ttl = <duration>$ttl, lastActiveAt = <datetime>$lastActiveAt, registrationTime = <float>$registrationTime, rowCount = <int>$rowCount";

    if let Err(e) = state
        .db
        .query(query)
        .bind(("id", incantation_id.clone()))
        .bind(("clientId", client_id))
        .bind(("authId", auth_id))
        .bind(("surql", surreal_ql))
        .bind(("params", params))
        .bind(("ttl", ttl))
        .bind(("lastActiveAt", last_active_at))
        .bind(("registrationTime", registration_time_ms))
        .bind(("rowCount", initial_row_count))
        .await
    {
        error!("Failed to upsert incantation metadata: {}", e);
        return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
    }

    // Create initial edges
    if let Some(ref delta) = update {
        debug!(incantation_id);
        let circuit = state.processor.read().await;
        update_incantation_edges(&state.db, delta, &state.metrics, &circuit, state.ref_mode).await;
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

    // Look up the auth_id from the View before removing it, so we can
    // target the right per-user `_00_list_ref_user_<id>` for the edge
    // cleanup below.
    let auth_id = {
        let circuit = state.processor.read().await;
        circuit
            .get_view(&payload.id)
            .map(|v| v.auth_id.clone())
            .unwrap_or_default()
    };

    // Remove from circuit
    {
        let mut circuit = state.processor.write().await;
        circuit.remove_query(&payload.id);
    }

    // Drop the per-view metrics state alongside the circuit entry so the
    // map doesn't grow unbounded across the lifetime of the SSP.
    {
        let mut metrics_map = state.view_metrics.write().await;
        metrics_map.remove(&payload.id);
    }

    state.metrics.view_count.add(-1, &[]);

    // Delete all edges for this incantation
    let incantation_id = format_incantation_id(&payload.id);
    let list_ref_tbl = tables::list_ref_table(state.ref_mode, &auth_id);
    if let Some(from_id) = parse_record_id(&incantation_id) {
        let stmt = format!("DELETE $from->{}", list_ref_tbl);
        if let Err(e) = state
            .db
            .query(stmt)
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

/// CRDT apply handler — merges a Loro update into the record's `_00_crdt[<field>]`
/// column server-side and persists the resulting snapshot. The record `UPDATE` then
/// flows through the existing event pipeline to all subscribed clients.
#[instrument(skip(state, payload), fields(table = %payload.table, record_id = %payload.record_id, field = %payload.field))]
async fn crdt_apply_handler(
    State(state): State<AppState>,
    Json(payload): Json<crdt::ApplyRequest>,
) -> Response {
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

    match state.crdt_cache.apply(&state.db, &payload).await {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(e) => {
            error!(error = %e, "CRDT apply failed");
            (StatusCode::BAD_REQUEST, e.to_string()).into_response()
        }
    }
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

    // Delete all edges. In dedicated mode that's every
    // `_00_list_ref_user_*` table; single mode is just the global one.
    match state.ref_mode {
        ssp_protocol::RefMode::Single => {
            if let Err(e) = state.db.query("DELETE _00_list_ref").await {
                error!("Failed to delete all edges on reset: {}", e);
            }
        }
        ssp_protocol::RefMode::Dedicated => {
            // Walk every per-user list_ref table and wipe it. Cheap
            // because this handler is only used in tests / explicit
            // resets.
            match state
                .db
                .query("INFO FOR DB")
                .await
                .and_then(|mut r| r.take::<surrealdb::types::Value>(0))
            {
                Ok(info_val) => {
                    let info_json: Value = info_val.into_json_value();
                    if let Some(tables) = info_json.get("tables").and_then(|t| t.as_object()) {
                        for name in tables.keys() {
                            if !name.starts_with("_00_list_ref_user_") {
                                continue;
                            }
                            if !name
                                .chars()
                                .all(|c| c.is_ascii_alphanumeric() || c == '_')
                            {
                                continue;
                            }
                            let stmt = format!("DELETE {}", name);
                            if let Err(e) = state.db.query(stmt).await {
                                error!(table = %name, "Failed to delete edges on reset: {}", e);
                            }
                        }
                    }
                }
                Err(e) => error!("Failed to enumerate edge tables on reset: {}", e),
            }
        }
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

    let circuit_tables: serde_json::Map<String, Value> = circuit
        .table_record_counts()
        .into_iter()
        .map(|(t, c)| (t, Value::from(c)))
        .collect();

    // Per-table content hashes — bit-identical to scheduler hashes when
    // the circuit is in sync with the frozen snapshot. Used by `spky
    // verify` and the scheduler's post-replay integrity check.
    let circuit_hashes: serde_json::Map<String, Value> = circuit
        .compute_table_hashes()
        .into_iter()
        .map(|(t, h)| (t, Value::String(h)))
        .collect();

    let ref_mode_str = match state.ref_mode {
        ssp_protocol::RefMode::Single => "single",
        ssp_protocol::RefMode::Dedicated => "dedicated",
    };

    Json(json!([
        {
            "entity": "ssp",
            "id": state.ssp_id,
            "ip": ip,
            "status": status_str,
            "views": circuit.view_count(),
            "version": env!("CARGO_PKG_VERSION"),
            "surrealdb_version": state.surrealdb_version,
            "uptime_seconds": state.start_time.elapsed().as_secs(),
            "last_heartbeat_seconds_ago": null,
            "circuit_tables": circuit_tables,
            "circuit_hashes": circuit_hashes,
            "ref_mode": ref_mode_str,
            "env": env_vars,
        }
    ]))
}

/// Info handler (text) — the same entity list as `/info`, serialized to a
/// JSON string and served as `text/plain`. The SurrealDB `/spooky` custom API
/// proxies this route via `http::get` and passes the body through verbatim, so
/// it must already be valid JSON text (mirrors the scheduler's `/info/text`).
async fn info_text_handler(State(state): State<AppState>) -> impl IntoResponse {
    let json_resp = info_handler(State(state)).await;
    let json_string = serde_json::to_string(&json_resp.0).unwrap_or_else(|_| "[]".to_string());
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "text/plain")],
        json_string,
    )
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
pub async fn cleanup_expired_query(
    db: &SharedDb,
    processor: &Arc<RwLock<Circuit>>,
    metrics: &Arc<Metrics>,
    query_id: &str,
    // `auth_id` is taken from the `_00_query` row (not the in-memory Circuit) so
    // we route the per-user `_00_list_ref_user_<id>` cleanup correctly even for
    // queries that aren't in this SSP's circuit (e.g. after a restart). Empty in
    // `RefMode::Single` or when the row has no auth_id.
    auth_id: &str,
    mode: ssp_protocol::RefMode,
) {
    let list_ref_tbl = tables::list_ref_table(mode, auth_id);
    // Build the record id with `type::record` from the raw query id rather than
    // parsing a literal (`RecordId::parse_simple` rejects some valid ids, which
    // silently skipped the delete). NOTE: it's `type::record` in SurrealDB v3.1+
    // (`type::thing` was renamed and now hard parse-errors — that error was being
    // swallowed by `unwrap_or_default`, so nothing was ever deleted). Conditional
    // `WHERE` so we only delete if the TTL is STILL expired (guards a heartbeat
    // landing between the sweep's SELECT and this DELETE). `LET … ; RETURN
    // array::len(...)` lets us read back the deleted count as a plain `i64`
    // (surrealdb v3 `take` of `RETURN BEFORE` records or `serde_json::Value` is
    // awkwardly tagged).
    let deleted_count: Option<i64> = match db
        .query(
            "LET $d = (DELETE type::record('_00_query', $qid) \
             WHERE lastActiveAt + ttl < time::now() RETURN BEFORE); \
             RETURN array::len($d);",
        )
        .bind(("qid", query_id.to_string()))
        .await
    {
        Ok(mut response) => response.take(1).unwrap_or_default(),
        Err(e) => {
            error!(query_id = %query_id, error = %e, "TTL cleanup: delete failed");
            return;
        }
    };
    if deleted_count.unwrap_or(0) == 0 {
        debug!(query_id = %query_id, "TTL cleanup: query refreshed, skipping");
        return;
    }

    // Delete associated list_ref edges.
    let edge_delete = format!("DELETE type::record('_00_query', $qid)->{}", list_ref_tbl);
    if let Err(e) = db
        .query(edge_delete)
        .bind(("qid", query_id.to_string()))
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

/// One sweep: delete every expired query view — its `_00_query` row, its per-user
/// `_00_list_ref` edges, and the in-memory circuit view — then drop any per-user
/// `_00_list_ref_user_<id>` table that no longer backs a live query.
///
/// Cleans ALL expired rows, not just those in this SSP's in-memory circuit. The
/// previous version gated on circuit membership, so views registered before a
/// restart (whose circuit is gone) leaked forever and accumulated. We read each
/// row's `auth_id` from `_00_query` so the per-user list_ref cleanup routes
/// correctly even when the query isn't in the circuit.
pub async fn ttl_cleanup_sweep(
    db: &SharedDb,
    processor: &Arc<RwLock<Circuit>>,
    metrics: &Arc<Metrics>,
    mode: ssp_protocol::RefMode,
) -> usize {
    // Fetch expired queries as `"<id>|<auth_id>"` strings. We deliberately
    // return a single concatenated string per row and `take::<Vec<String>>` it:
    // surrealdb v3's `take` deserializes scalar Strings natively, whereas going
    // through `serde_json::to_value(Value)` yields an externally-tagged shape
    // (`{"String": ...}`) that's awkward to read field-by-field.
    let rows: Vec<String> = match db
        .query(
            "SELECT VALUE (<string>id + '|' + <string>(auth_id OR '')) \
             FROM _00_query WHERE lastActiveAt + ttl < time::now()",
        )
        .await
    {
        Ok(mut response) => response.take(0).unwrap_or_default(),
        Err(e) => {
            error!("TTL cleanup: query failed: {}", e);
            return 0;
        }
    };

    let count = rows.len();
    let mut cleaned_auth_ids: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    for row in rows {
        let mut parts = row.splitn(2, '|');
        let id = parts.next().unwrap_or_default();
        let auth_id = parts.next().unwrap_or_default().to_string();
        if id.is_empty() {
            continue;
        }
        let raw = id.strip_prefix("_00_query:").unwrap_or(id).to_string();
        cleanup_expired_query(db, processor, metrics, &raw, &auth_id, mode).await;
        if !auth_id.is_empty() {
            cleaned_auth_ids.insert(auth_id);
        }
    }

    // Drop the per-user list_ref table for any user whose last query we just
    // removed, then a best-effort pass for pre-existing orphan tables (whose
    // owner has no live query at all — e.g. leftovers from older cleanups).
    for auth_id in &cleaned_auth_ids {
        if let Err(e) = tables::drop_user_table_if_unused(db, mode, auth_id).await {
            warn!(auth_id = %auth_id, error = %e, "TTL cleanup: drop_user_table_if_unused failed");
        }
    }
    if let Err(e) = tables::drop_orphaned_user_tables(db, mode).await {
        warn!(error = %e, "TTL cleanup: drop_orphaned_user_tables failed");
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

/// Format incantation ID with the global `_00_query` prefix. Strips
/// any existing prefix from `id` and re-applies `_00_query:`. The
/// registration table is global in both ref modes because the client
/// needs to compute the record id without knowing the user at
/// id-creation time; only `_00_list_ref` splits per user.
fn format_incantation_id(id: &str) -> String {
    let raw = id.rsplit(':').next().unwrap_or(id);
    format!("_00_query:{}", raw)
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
    mode: ssp_protocol::RefMode,
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

        // Route writes to the originating user's per-user `_00_query` /
        // `_00_list_ref` pair in `RefMode::Dedicated`. The delta carries
        // its owning user from `View.auth_id` (set in `add_query_with_auth`)
        // so we don't need an extra DB lookup per delta.
        let incantation_id = format_incantation_id(&delta.query_id);
        let list_ref_tbl = tables::list_ref_table(mode, &delta.auth_id);

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
                "RELATE ${binding}->{list_ref}->{out} SET version = {version}, clientId = (SELECT VALUE clientId FROM ${binding} LIMIT 1)[0], auth_id = (SELECT VALUE auth_id FROM ${binding} LIMIT 1)[0]",
                binding = binding_name,
                list_ref = list_ref_tbl,
                out = id,
                version = version,
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
                "UPDATE {list_ref} SET version = {version} WHERE in = ${binding} AND out = {out}",
                list_ref = list_ref_tbl,
                version = version,
                binding = binding_name,
                out = id,
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
                "DELETE ${binding}->{list_ref} WHERE out = {out}",
                binding = binding_name,
                list_ref = list_ref_tbl,
                out = id,
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
                        "RELATE ${binding}->{list_ref}->{id} SET \
                         version = {version}, \
                         clientId = (SELECT VALUE clientId FROM ${binding} LIMIT 1)[0], \
                         auth_id = (SELECT VALUE auth_id FROM ${binding} LIMIT 1)[0], \
                         parent = (SELECT VALUE id FROM {list_ref} WHERE in = ${binding} AND out = {parent} LIMIT 1)[0], \
                         parent_rel = '{alias}'",
                        binding = binding_name,
                        list_ref = list_ref_tbl,
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
                        "UPDATE {list_ref} SET version = {version} WHERE in = ${binding} AND out = {id}",
                        list_ref = list_ref_tbl,
                        binding = binding_name, id = item.id, version = version
                    ));
                }
                SubqueryOp::Remove => {
                    deleted_count += 1;
                    all_statements.push(format!(
                        "DELETE ${binding}->{list_ref} WHERE out = {id}",
                        binding = binding_name, list_ref = list_ref_tbl, id = item.id
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
    mode: ssp_protocol::RefMode,
) {
    update_all_edges(db, &[delta], metrics, circuit, mode).await;
}

/// Push the same materialization-step latency sample onto every affected
/// view's rolling window, then persist `_00_query.{materializationP55,
/// materializationP90, materializationP99, lastIngestLatency, updateCount,
/// rowCount}` for each view. Best-effort: logs but does not surface
/// failures to the ingest caller.
async fn persist_view_metrics(
    state: &AppState,
    row_counts: Vec<usize>,
    view_ids: Vec<String>,
    materialization_time_ms: f64,
) {
    if view_ids.is_empty() {
        return;
    }

    // Update the in-memory rolling window once per affected view, computing
    // the new percentiles and update_count under the same lock acquisition.
    let snapshots: Vec<_> = {
        let mut metrics_map = state.view_metrics.write().await;
        view_ids
            .iter()
            .zip(row_counts.iter())
            .map(|(view_id, row_count)| {
                let entry = metrics_map
                    .entry(view_id.clone())
                    .or_insert_with(ViewMetricsState::default);
                entry.record_sample(materialization_time_ms);
                entry.update_count = entry.update_count.saturating_add(1);
                let percentiles = entry.percentiles();
                (
                    view_id.clone(),
                    *row_count,
                    entry.update_count,
                    materialization_time_ms,
                    percentiles,
                )
            })
            .collect()
    };

    for (view_id, row_count, update_count, last_ingest_latency, percentiles) in snapshots {
        let incantation_id = format_incantation_id(&view_id);
        let query = "UPDATE <record>$id SET \
            rowCount = <int>$rowCount, \
            updateCount = <int>$updateCount, \
            lastIngestLatency = <float>$lastIngestLatency, \
            materializationP55 = $p55, \
            materializationP90 = $p90, \
            materializationP99 = $p99";

        let (p55, p90, p99) = match percentiles {
            Some(t) => (Some(t.0), Some(t.1), Some(t.2)),
            None => (None, None, None),
        };

        if let Err(e) = state
            .db
            .query(query)
            .bind(("id", incantation_id.clone()))
            .bind(("rowCount", row_count as i64))
            .bind(("updateCount", update_count as i64))
            .bind(("lastIngestLatency", last_ingest_latency))
            .bind(("p55", p55))
            .bind(("p90", p90))
            .bind(("p99", p99))
            .await
        {
            warn!(
                target: "ssp::view_metrics",
                error = %e,
                view_id = %incantation_id,
                "Failed to persist per-view metrics"
            );
        }
    }
}
