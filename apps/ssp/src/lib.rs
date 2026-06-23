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
pub mod crdt;
pub mod edge_updates;
pub mod metrics;
pub mod open_telemetry;
pub mod tables;
pub mod view_metrics;

use metrics::Metrics;
use view_metrics::{ViewMetrics, ViewMetricsState};

use job_runner::{
    fail_if_pending_helper, reset_for_retry_helper, update_status_helper, BackendInfo, JobConfig,
    JobControl, JobEntry, JobRunner,
};
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
    /// Shared cancellation/kill state, also held by the `JobRunner`. Lets the
    /// `/job/kill` and `/job/retry` handlers cancel in-flight requests and drop
    /// queued jobs without racing the runner (which stays the sole `status` writer).
    pub job_control: JobControl,
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
    /// When `true`, anonymous (empty `auth_id`) query registrations are routed
    /// to the world-readable `_00_list_ref_anon` table so logged-out clients
    /// can sync live. See `Config::anonymous_live_queries`.
    pub anonymous_live_queries: bool,
    /// Version of the upstream SurrealDB server, queried once at startup
    /// (`"unknown"` if the query failed). Surfaced via `/info` so the DevTools
    /// can report the SurrealDB backend version.
    pub surrealdb_version: String,
    /// Sender into the edge-update coalescing flusher. View edge writes
    /// (`_00_list_ref`) from ingests and registrations are pushed here and
    /// batched into one transaction per `Config::query_update_throttle_ms`
    /// window by the `edge_updates::run_edge_update_service` task instead of one
    /// transaction per update.
    pub edge_update_tx: mpsc::UnboundedSender<Vec<ViewDelta>>,
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
    /// Total wall-clock budget (seconds) for retrying scheduler registration
    /// before the process exits to let the supervisor restart it. Must comfortably
    /// exceed a cold `--clean` re-clone window (scheduler returns 503 while
    /// `Cloning`). Env: `SPKY_SSP_REGISTER_MAX_WAIT_SECS`, default 180.
    pub register_max_wait_secs: u64,
    /// Storage layout for `_00_query` / `_00_list_ref`. See
    /// `ssp_protocol::RefMode`. Defaults to `Dedicated` so cross-session
    /// LIVE delivery doesn't depend on the SurrealDB v3 LIVE-permission
    /// path; flip to `Single` only when running against a SurrealDB
    /// version that delivers cross-session LIVE notifications correctly
    /// through permission rules.
    pub ref_mode: ssp_protocol::RefMode,
    /// Enable realtime sync for unauthenticated (anonymous) clients. When
    /// `true`, anonymous query registrations (empty `auth_id`) are routed to a
    /// dedicated `_00_list_ref_anon` table that anyone can SELECT, so a
    /// logged-out client's `_00_list_ref` poll can read its window. Defaults to
    /// `false`. Env: `SPKY_SSP_ANON_LIVE_QUERIES` (`1`/`true` to enable).
    pub anonymous_live_queries: bool,
    /// Coalescing window (ms) for query edge-update writes to `_00_list_ref`.
    /// View edge writes (from ingests and registrations) are buffered and
    /// flushed in ONE batched transaction every this-many ms, so a burst of
    /// query updates (a bulk import, a page registering several queries, a sync
    /// backfill) lands as a few batched LIVE deliveries instead of one
    /// transaction per record — the per-record pacing that streamed a fresh
    /// client's window sync over ~10s. Env: `SPKY_SSP_QUERY_UPDATE_THROTTLE_MS`,
    /// default 100. `0` disables batching (each update flushes immediately).
    pub query_update_throttle_ms: u64,
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
        register_max_wait_secs: std::env::var("SPKY_SSP_REGISTER_MAX_WAIT_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(180),
        ref_mode: std::env::var("SPKY_SSP_REF_MODE")
            .ok()
            .as_deref()
            .and_then(ssp_protocol::RefMode::parse_str)
            .unwrap_or_default(),
        anonymous_live_queries: std::env::var("SPKY_SSP_ANON_LIVE_QUERIES")
            .map(|v| {
                let v = v.trim().to_ascii_lowercase();
                v == "1" || v == "true"
            })
            .unwrap_or(false),
        query_update_throttle_ms: std::env::var("SPKY_SSP_QUERY_UPDATE_THROTTLE_MS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(100),
    }
}

// --- Scheduler Registration Helper ---

/// Why a registration attempt failed, used by the caller's retry loop to decide
/// whether retrying can ever succeed.
enum RegisterError {
    /// Transient: scheduler still cloning (503), a 5xx, a transport/timeout
    /// error, or an unparseable response. Worth retrying — the scheduler may
    /// just not be `Ready` yet (e.g. a cold `--clean` re-clone).
    Retryable(String),
    /// Permanent for this process: the scheduler rejected the request with a
    /// 4xx (e.g. 400 bad ssp_id/url). Retrying the same payload won't help —
    /// surface it loudly instead of silently spinning.
    Fatal(String),
}

impl std::fmt::Display for RegisterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegisterError::Retryable(m) => write!(f, "{}", m),
            RegisterError::Fatal(m) => write!(f, "{}", m),
        }
    }
}

/// Build the SSP registration payload and POST it to the scheduler.
/// Returns the registration response (which carries `snapshot_seq` and the
/// per-table content hashes the SSP must match after bootstrap) or an error
/// classified as retryable vs fatal.
async fn register_with_scheduler(
    client: &reqwest::Client,
    scheduler_url: &str,
    ssp_id: &str,
    listen_addr: &str,
    advertise_addr: Option<&str>,
) -> Result<ssp_protocol::SspRegistrationResponse, RegisterError> {
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

    match client
        .post(&registration_url)
        .json(&payload)
        // Bound a single attempt so a hung connection can't stall the retry
        // loop. Per-request (not client-wide) so it doesn't shorten the proxy
        // bootstrap queries that reuse the same client.
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => resp
            .json::<ssp_protocol::SspRegistrationResponse>()
            .await
            // A 2xx with an unparseable body is almost always a transient proxy
            // hiccup — retry rather than give up.
            .map_err(|e| RegisterError::Retryable(format!("Failed to parse registration response: {}", e))),
        // 503 (Cloning/Restoring) and any 5xx: scheduler not ready yet → retry.
        Ok(resp) if resp.status() == StatusCode::SERVICE_UNAVAILABLE || resp.status().is_server_error() => {
            Err(RegisterError::Retryable(format!("HTTP {}", resp.status())))
        }
        // 4xx (e.g. 400 bad ssp_id/url): a real misconfiguration → fatal.
        Ok(resp) => Err(RegisterError::Fatal(format!("HTTP {}", resp.status()))),
        // Connection refused / DNS not ready / timeout: scheduler not up yet → retry.
        Err(e) => Err(RegisterError::Retryable(format!("{}", e))),
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

    // The orchestrator emits a JSON array of backend entries. An empty
    // backend list can legitimately arrive as `null` (Go's json.Marshal
    // encodes a nil slice as null) or `[]`; both mean "no outbox backends",
    // not a misconfiguration, so treat them as an empty config instead of a
    // hard parse failure that masks the real (empty) state.
    let entries: Vec<serde_json::Value> =
        match serde_json::from_str::<Option<Vec<serde_json::Value>>>(&json) {
            Ok(Some(v)) => v,
            Ok(None) => Vec::new(),
            Err(e) => {
                warn!(error = %e, raw = %json, "Failed to parse SPKY_JOB_CONFIG, job runner disabled");
                return Arc::new(JobConfig::default());
            }
        };

    if entries.is_empty() {
        info!("SPKY_JOB_CONFIG lists no outbox backends, job runner disabled");
        return Arc::new(JobConfig::default());
    }

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
        .route("/job/kill", post(job_kill_handler))
        .route("/job/retry", post(job_retry_handler))
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

    // Shared kill/cancel state. Constructed unconditionally (AppState always needs
    // it); the runner only gets a clone when there are job tables to run.
    let job_control = JobControl::new();

    // Spawn job runner if there are job tables configured
    if !job_config.job_tables.is_empty() {
        let job_runner = JobRunner::new(
            job_queue_rx,
            job_queue_tx.clone(),
            db.clone(),
            job_control.clone(),
        );
        tokio::spawn(async move {
            job_runner.run().await;
        });
        info!("Job runner started");

        // Recovery sweep for jobs the CREATE-only pickup path can't reach (rows
        // created while this SSP was down, or 'processing' rows orphaned by a
        // crash). Singlenode only: in cluster mode the scheduler assigns jobs
        // round-robin and the assignee is not persisted on the row, so a
        // DB-driven sweep here couldn't tell ownership and might double-execute —
        // cluster recovery is the scheduler's responsibility.
        if config.scheduler_url.is_none() {
            let db = db.clone();
            let status = status.clone();
            let job_config = job_config.clone();
            let job_control = job_control.clone();
            let job_queue_tx = job_queue_tx.clone();
            tokio::spawn(async move {
                // `interval` fires immediately on first tick, so the first pass
                // runs as soon as the SSP is Ready (the startup sweep), then every
                // JOB_RECOVERY_INTERVAL_SECS thereafter.
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(
                    JOB_RECOVERY_INTERVAL_SECS,
                ));
                loop {
                    interval.tick().await;
                    if *status.read().await != SspStatus::Ready {
                        continue;
                    }
                    job_recovery_sweep(&db, &job_config, &job_control, &job_queue_tx).await;
                }
            });
            info!(
                interval_secs = JOB_RECOVERY_INTERVAL_SECS,
                "Job recovery sweep started"
            );
        } else {
            info!("Job recovery sweep disabled (cluster mode — recovery is the scheduler's responsibility)");
        }
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

    // Coalescing flusher for query edge-updates: edge writes to `_00_list_ref`
    // (from ingests and registrations) are batched into one transaction per
    // `query_update_throttle_ms` window so a burst of view updates lands as a
    // few batched LIVE deliveries instead of one transaction per record (which
    // paced a fresh client's window sync over ~10s). See `Config`.
    let (edge_update_tx, edge_update_rx) = mpsc::unbounded_channel::<Vec<ViewDelta>>();
    tokio::spawn(edge_updates::run_edge_update_service(
        edge_update_rx,
        edge_updates::SurrealEdgeSink {
            db: db.clone(),
            processor: processor_arc.clone(),
            metrics: metrics.clone(),
            mode: config.ref_mode,
        },
        std::time::Duration::from_millis(config.query_update_throttle_ms),
        edge_updates::MAX_EDGE_BATCH,
    ));

    let state = AppState {
        db: db.clone(),
        processor: processor_arc.clone(),
        status: status.clone(),
        metrics: metrics.clone(),
        job_config,
        job_queue_tx,
        job_control,
        ssp_id: config.ssp_id.clone(),
        scheduler_url: config.scheduler_url.clone(),
        start_time: std::time::Instant::now(),
        crdt_cache,
        view_metrics: Arc::new(RwLock::new(std::collections::HashMap::new())),
        ref_mode: config.ref_mode,
        anonymous_live_queries: config.anonymous_live_queries,
        surrealdb_version,
        edge_update_tx,
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
        let register_max_wait_secs = config.register_max_wait_secs;

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

                // Retry registration with exponential backoff. The scheduler
                // returns 503 while `Cloning`/`Restoring` (e.g. a cold
                // `--clean` re-clone), and may be unreachable for a moment
                // while DNS/the container comes up — both are transient. A
                // single attempt that gave up (the old behaviour) left the SSP
                // a permanently-unregistered zombie: it never exited, so the
                // supervisor's restart-on-failure policy never fired, and the
                // heartbeat loop only runs once status==Ready. Now we keep
                // trying within a budget, then exit so the supervisor reruns
                // the whole handshake against a (by then) Ready scheduler.
                let registration = {
                    let max_wait = std::time::Duration::from_secs(register_max_wait_secs);
                    let start = std::time::Instant::now();
                    let mut backoff_ms: u64 = 1000;
                    loop {
                        match register_with_scheduler(
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
                                    waited_secs = start.elapsed().as_secs(),
                                    "Successfully registered with scheduler"
                                );
                                break r;
                            }
                            Err(RegisterError::Fatal(e)) => {
                                error!(
                                    error = %e,
                                    "Scheduler rejected registration (non-retryable) — exiting for visibility"
                                );
                                *status.write().await = SspStatus::Failed;
                                // exit(6): distinct from the data-bootstrap
                                // exit(2) and heartbeat exit(3)/exit(4) so the
                                // failure mode is identifiable in logs.
                                std::process::exit(6);
                            }
                            Err(RegisterError::Retryable(e)) => {
                                let elapsed = start.elapsed();
                                if elapsed >= max_wait {
                                    error!(
                                        error = %e,
                                        waited_secs = elapsed.as_secs(),
                                        max_wait_secs = register_max_wait_secs,
                                        "Scheduler registration still failing after max wait — exiting for restart"
                                    );
                                    *status.write().await = SspStatus::Failed;
                                    // exit(5): exhausted the retry budget; let
                                    // the supervisor restart us so the full
                                    // register→bootstrap handshake reruns.
                                    std::process::exit(5);
                                }
                                warn!(
                                    error = %e,
                                    backoff_ms,
                                    waited_secs = elapsed.as_secs(),
                                    "Scheduler not ready for registration, retrying"
                                );
                                tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
                                backoff_ms = (backoff_ms * 2).min(10_000);
                            }
                        }
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
                        // Seed the catch-up XOR accumulators from the freshly
                        // bulk-loaded rows (`Circuit::load` bypasses the per-row
                        // `apply_mutation` maintenance). Must run before the SSP
                        // goes Ready, i.e. before any replay events are ingested,
                        // so the accumulator starts from the snapshot content.
                        {
                            let mut guard = processor.write().await;
                            guard.reseed_catchup_hashes();
                        }
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

/// Build one page of the bootstrap table scan, using KEYSET pagination on the
/// record `id` (`WHERE id > <last>`) rather than an `OFFSET`/`START`.
///
/// Bootstrap pages the whole table over many round-trips, and the source DB is
/// live: clients keep creating/deleting records while the SSP boots (and again
/// on every re-bootstrap). Offset pagination is unsafe under those concurrent
/// writes — a delete before the current offset shifts every later row up by one,
/// so `START n` skips a row on the next page. (The original query had no
/// `ORDER BY` at all; adding one does NOT help here — `ORDER BY id LIMIT n
/// START m` is just as lossy under a concurrent delete, confirmed empirically.)
/// A skipped record never enters the circuit store, so it is invisible to the
/// DBSP views: deleting it later emits no `removals` delta and the windowed view
/// never updates — the client's live list goes stale until a manual reload. This
/// only bites tables big enough to page (default 200) under concurrent load,
/// which is why small/quiescent collections never show it.
///
/// Keyset pagination is immune: each page resumes from the last id seen, so a
/// delete behind the cursor can't shift rows ahead of it out of view. `id` is
/// the unique, indexed primary key, so `WHERE id > $last ORDER BY id` is a cheap
/// index range scan with a stable total order.
fn bootstrap_page_query(table: &str, page_size: usize, after_id: Option<&str>) -> String {
    match after_id {
        None => format!("SELECT * FROM {table} ORDER BY id LIMIT {page_size}"),
        Some(id) => format!("SELECT * FROM {table} WHERE id > {id} ORDER BY id LIMIT {page_size}"),
    }
}

#[cfg(test)]
mod bootstrap_pagination_tests {
    use super::bootstrap_page_query;
    use std::collections::BTreeSet;

    #[test]
    fn page_query_uses_ordered_keyset_not_offset() {
        // Regression guard: the bootstrap scan must NOT use OFFSET/START (lossy
        // under concurrent writes) and must resume by id keyset, ordered by id.
        let first = bootstrap_page_query("game", 200, None);
        assert_eq!(first, "SELECT * FROM game ORDER BY id LIMIT 200");

        let next = bootstrap_page_query("game", 200, Some("game:abc"));
        assert_eq!(
            next,
            "SELECT * FROM game WHERE id > game:abc ORDER BY id LIMIT 200"
        );

        // Neither page may fall back to offset pagination.
        assert!(!first.contains("START") && !next.contains("START"));
        assert!(first.contains("ORDER BY id") && next.contains("ORDER BY id"));
    }

    // Walk a paginated scan over a table of `total` rows (ids 0..total) while a
    // concurrent delete removes the first `deleted` ids right after the first
    // page — the exact race the SSP hits paging a live table during (re)bootstrap.
    // `keyset` switches between resuming by `id > last` (the fix) and a fixed
    // `START`/offset (the bug). Returns how many of the SURVIVING rows were seen.
    fn paginate_with_concurrent_delete(
        total: usize,
        page: usize,
        deleted: usize,
        keyset: bool,
    ) -> (usize, usize) {
        // The live table, ordered by id. A delete just removes ids from this set.
        let mut table: BTreeSet<usize> = (0..total).collect();
        let mut seen: BTreeSet<usize> = BTreeSet::new();
        let mut start = 0usize;
        let mut last: Option<usize> = None;
        let mut first_page = true;
        loop {
            // What an `ORDER BY id` scan returns for this page right now.
            let ordered: Vec<usize> = table.iter().copied().collect();
            let pagerows: Vec<usize> = if keyset {
                ordered
                    .into_iter()
                    .filter(|&id| last.map_or(true, |l| id > l))
                    .take(page)
                    .collect()
            } else {
                ordered.into_iter().skip(start).take(page).collect()
            };
            let n = pagerows.len();
            if n == 0 {
                break;
            }
            for id in &pagerows {
                seen.insert(*id);
            }
            last = pagerows.last().copied();
            if first_page {
                // Concurrent delete lands between page 1 and page 2.
                for id in 0..deleted {
                    table.remove(&id);
                }
                first_page = false;
            }
            if n < page {
                break;
            }
            start += page;
        }
        let survivors = total - deleted;
        let seen_survivors = seen.iter().filter(|&&id| id >= deleted).count();
        (survivors, seen_survivors)
    }

    #[test]
    fn keyset_survives_concurrent_delete_offset_loses_rows() {
        // Offset pagination drops surviving rows when a delete shifts the scan...
        let (survivors, seen) = paginate_with_concurrent_delete(500, 100, 30, false);
        assert!(
            seen < survivors,
            "offset pagination should drop survivors under a concurrent delete (saw {seen}/{survivors})"
        );
        // ...keyset pagination sees every surviving row — this is the fix.
        let (survivors, seen) = paginate_with_concurrent_delete(500, 100, 30, true);
        assert_eq!(
            seen, survivors,
            "keyset pagination must load every surviving row ({seen}/{survivors})"
        );
    }
}

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
    // Tables marked `-- @nosync` carry a `COMMENT 'sp00ky:nosync'` marker in
    // their `DEFINE TABLE` string. Skip them entirely: no permission is
    // registered and no data is loaded into the circuit, so the table never
    // participates in sync. It stays in the upstream DB (still backed up).
    let table_defs: Vec<(String, String)> = match info_json.get("tables") {
        Some(Value::Object(tables_map)) => tables_map
            .iter()
            .filter(|(name, _)| !name.starts_with("_00_"))
            .filter(|(name, def)| {
                let nosync = def
                    .as_str()
                    .map(ssp_protocol::define_str_is_nosync)
                    .unwrap_or(false);
                if nosync {
                    info!(table = %name, "Excluding @nosync table from bootstrap");
                }
                !nosync
            })
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
        // Keyset cursor: the highest `id` loaded so far. `None` = first page.
        let mut after_id: Option<String> = None;
        loop {
            let result = source
                .query(&bootstrap_page_query(table, page_size, after_id.as_deref()))
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

            // Advance the cursor to the last row's id BEFORE consuming `rows`
            // (the page is ORDER BY id, so the last row carries the max id).
            let next_after = rows
                .last()
                .and_then(|row| row.get("id"))
                .and_then(|v| v.as_str())
                .map(str::to_string);

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
            // No usable id to resume from → stop rather than loop forever.
            match next_after {
                Some(id) => after_id = Some(id),
                None => break,
            }
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

// --- Job control handlers (kill / retry) ---

#[derive(Deserialize, Debug)]
struct JobActionRequest {
    id: String,
}

/// Load only the scalar fields of a job row we need (avoids deserializing the
/// `id` RecordId / `assigned_to` record link / datetimes into `serde_json::Value`,
/// which can fail). Returns `None` when the job does not exist.
async fn load_job_record(state: &AppState, id: &str) -> Result<Option<Value>, surrealdb::Error> {
    let Ok(rid) = RecordId::parse_simple(id) else {
        return Ok(None);
    };
    let mut resp = state
        .db
        .query("SELECT status, path, payload, retries, max_retries, retry_strategy, timeout FROM ONLY $id")
        .bind(("id", rid))
        .await?;
    Ok(resp.take(0).ok().flatten())
}

/// `POST /job/kill` — stop a job.
///
/// - `processing` and in-flight on this SSP: fire the cancellation token and let
///   the runner write the terminal status. The handler deliberately does **not**
///   write `status` itself — the runner is the single writer, so this avoids
///   clobbering a request that completes in the same instant.
/// - `pending`/queued (or `processing` but owned by another SSP): set a kill flag
///   the runner honors at dequeue; it fails the job instead of running it.
/// - `success`/`failed`: idempotent no-op.
async fn job_kill_handler(State(state): State<AppState>, Json(req): Json<JobActionRequest>) -> Response {
    if RecordId::parse_simple(&req.id).is_err() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "code": "bad_id", "message": "id must be 'table:key'" })),
        )
            .into_response();
    }

    let record = match load_job_record(&state, &req.id).await {
        Ok(Some(r)) => r,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "code": "not_found", "message": format!("job '{}' not found", req.id) })),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "code": "db_error", "message": e.to_string() })),
            )
                .into_response();
        }
    };

    let status = record.get("status").and_then(|v| v.as_str()).unwrap_or("");
    match status {
        "success" | "failed" => (
            StatusCode::OK,
            Json(json!({ "id": req.id, "status": status, "message": "already terminal; no-op" })),
        )
            .into_response(),
        "processing" => {
            if let Some(token) = state.job_control.inflight.get(&req.id) {
                token.cancel();
                (
                    StatusCode::OK,
                    Json(json!({ "id": req.id, "status": "cancelling", "message": "cancelling in-flight request" })),
                )
                    .into_response()
            } else {
                // Processing, but not in-flight on this SSP (cluster: another SSP
                // owns it, or a stale row). Flag it so any later re-enqueue is
                // dropped, and report it wasn't local.
                state.job_control.killed_pending.insert(req.id.clone());
                (
                    StatusCode::OK,
                    Json(json!({ "id": req.id, "status": "processing", "message": "not in-flight on this ssp; kill flag set" })),
                )
                    .into_response()
            }
        }
        _ => {
            // pending / unknown. Two cooperating actions, in this order:
            //  1. Set the drop-flag first, so if this job is sitting in the
            //     in-memory queue the runner fails it at dequeue (and is the sole
            //     writer in that case — no clobber).
            //  2. Also terminalize the row directly *iff* it is still pending.
            //     Pickup is CREATE-only, so an orphaned pending row (created while
            //     this SSP was down) is never enqueued and the flag alone would be
            //     a no-op forever. The `WHERE status = 'pending'` guard inside the
            //     helper means we never clobber a row already advanced to
            //     'processing' by the runner.
            state.job_control.killed_pending.insert(req.id.clone());
            let error_entry = json!({ "code": "killed", "reason": "killed by operator" });
            match fail_if_pending_helper(&state.db, &req.id, error_entry).await {
                Ok(true) => (
                    StatusCode::OK,
                    Json(json!({ "id": req.id, "status": "failed", "message": "killed pending job" })),
                )
                    .into_response(),
                Ok(false) => (
                    // Not pending at write time (raced to 'processing', or already
                    // terminal). The flag still guards any queued copy.
                    StatusCode::OK,
                    Json(json!({ "id": req.id, "status": status, "message": "kill flag set; will fail at dequeue" })),
                )
                    .into_response(),
                Err(e) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "code": "db_error", "message": e.to_string() })),
                )
                    .into_response(),
            }
        }
    }
}

/// `POST /job/retry` — re-run a terminal (`failed`/`success`) job. Resets the row
/// (`status='pending'`, `retries=0`, `errors=[]`) and re-enqueues a fresh
/// `JobEntry` directly, because a plain `UPDATE` would not re-trigger the
/// CREATE-gated ingest path.
async fn job_retry_handler(State(state): State<AppState>, Json(req): Json<JobActionRequest>) -> Response {
    let Some((table, _)) = req.id.split_once(':') else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "code": "bad_id", "message": "id must be 'table:key'" })),
        )
            .into_response();
    };
    let table = table.to_string();

    let Some(backend) = state.job_config.job_tables.get(&table) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "code": "unknown_table", "message": format!("no job backend configured for table '{}'", table) })),
        )
            .into_response();
    };
    let backend = backend.clone();

    let record = match load_job_record(&state, &req.id).await {
        Ok(Some(r)) => r,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "code": "not_found", "message": format!("job '{}' not found", req.id) })),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "code": "db_error", "message": e.to_string() })),
            )
                .into_response();
        }
    };

    let status = record.get("status").and_then(|v| v.as_str()).unwrap_or("");
    if status != "failed" && status != "success" {
        return (
            StatusCode::CONFLICT,
            Json(json!({ "code": "not_terminal", "message": format!("cannot retry job in '{}' state; only 'failed'/'success'", status) })),
        )
            .into_response();
    }

    // Clear any stale kill flag so the re-enqueued job isn't dropped at dequeue.
    // Order matters: remove the flag BEFORE re-enqueueing.
    state.job_control.killed_pending.remove(&req.id);

    if let Err(e) = reset_for_retry_helper(&state.db, &req.id).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "code": "db_error", "message": e.to_string() })),
        )
            .into_response();
    }

    // Rebuild the JobEntry. `from_record` copies `retries` from the (pre-reset)
    // snapshot, so force it to 0 to honor the retry budget.
    let timeout_override = record.get("timeout").and_then(|v| v.as_u64()).map(|v| v as u32);
    let mut job = JobEntry::from_record(
        req.id.clone(),
        backend.base_url.clone(),
        backend.auth_token.clone(),
        &record,
        backend.effective_timeout(timeout_override),
    );
    job.retries = 0;

    // Guard against a concurrent retry / recovery sweep enqueueing the same id
    // twice. If it is already queued, this retry is a no-op.
    if !state.job_control.mark_enqueued(&req.id) {
        return (
            StatusCode::OK,
            Json(json!({ "id": req.id, "status": "pending", "message": "already queued" })),
        )
            .into_response();
    }
    if let Err(e) = state.job_queue_tx.send(job).await {
        state.job_control.clear_enqueued(&req.id);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "code": "enqueue_failed", "message": e.to_string() })),
        )
            .into_response();
    }

    (
        StatusCode::OK,
        Json(json!({ "id": req.id, "status": "pending", "message": "re-enqueued" })),
    )
        .into_response()
}

// --- Job recovery sweep (singlenode) ---
//
// Job pickup is CREATE-only: the runner enqueues a job when it observes the
// CREATE live-query event. A row that was created while this SSP was down (or
// before its live query was established) is never enqueued, and a 'processing'
// row whose SSP crashed mid-flight is never finished — both sit stuck forever.
// This sweep periodically (and once at startup) re-enqueues them. Every enqueue
// goes through `JobControl::mark_enqueued`, so it can never double-execute a job
// the live CREATE path already queued.

/// How often the recovery sweep runs.
const JOB_RECOVERY_INTERVAL_SECS: u64 = 60;
/// Only re-enqueue `pending` rows older than this, so the sweep never races the
/// live CREATE-pickup path on freshly created jobs.
const JOB_RECOVERY_PENDING_GRACE_SECS: u64 = 30;
/// A `processing` row untouched for longer than this is treated as orphaned (its
/// SSP crashed mid-flight) and reset to pending. Kept well above any realistic
/// job timeout so a legitimately long request is never reset out from under the
/// runner.
const JOB_RECOVERY_STALE_PROCESSING_SECS: u64 = 600;

/// Run one recovery pass across every configured job table.
async fn job_recovery_sweep<C: Connection>(
    db: &Surreal<C>,
    job_config: &JobConfig,
    job_control: &JobControl,
    job_queue_tx: &mpsc::Sender<JobEntry>,
) {
    for (table, backend_info) in &job_config.job_tables {
        match recover_job_table(db, table, backend_info, job_control, job_queue_tx).await {
            Ok(n) if n > 0 => {
                info!(target: "ssp::job_recovery", table = %table, recovered = n, "Recovery sweep re-enqueued stuck jobs")
            }
            Ok(_) => {}
            Err(e) => {
                warn!(target: "ssp::job_recovery", table = %table, error = %e, "Recovery sweep failed for table")
            }
        }
    }
}

/// Recover stuck `pending` and orphaned `processing` rows for one job table.
/// Returns the number of jobs re-enqueued.
async fn recover_job_table<C: Connection>(
    db: &Surreal<C>,
    table: &str,
    backend_info: &BackendInfo,
    job_control: &JobControl,
    job_queue_tx: &mpsc::Sender<JobEntry>,
) -> anyhow::Result<usize> {
    // `type::string(id) AS id` keeps the RecordId out of the JSON projection
    // (deserializing a RecordId into `serde_json::Value` can fail), mirroring
    // `load_job_record`.
    const FIELDS: &str = "type::string(id) AS id, status, path, payload, retries, \
                          max_retries, retry_strategy, timeout";
    let mut recovered = 0;

    // 1. Stuck pending rows (pending longer than the grace window).
    let pending_q = format!(
        "SELECT {FIELDS} FROM {table} \
         WHERE status = 'pending' AND updated_at < time::now() - {grace}s",
        grace = JOB_RECOVERY_PENDING_GRACE_SECS,
    );
    let mut resp = db
        .query(&pending_q)
        .await
        .context("pending recovery query failed")?;
    let rows: Vec<Value> = resp.take(0).context("reading pending recovery rows")?;
    for row in &rows {
        let Some(id) = row.get("id").and_then(|v| v.as_str()) else {
            continue;
        };
        if enqueue_recovered(job_control, job_queue_tx, backend_info, id, row).await {
            recovered += 1;
            warn!(target: "ssp::job_recovery", job_id = %id, "Re-enqueued stuck pending job");
        }
    }

    // 2. Orphaned processing rows (processing far longer than any job runs).
    let stale_q = format!(
        "SELECT {FIELDS} FROM {table} \
         WHERE status = 'processing' AND updated_at < time::now() - {stale}s",
        stale = JOB_RECOVERY_STALE_PROCESSING_SECS,
    );
    let mut resp = db
        .query(&stale_q)
        .await
        .context("stale processing query failed")?;
    let rows: Vec<Value> = resp.take(0).context("reading stale processing rows")?;
    for row in &rows {
        let Some(id) = row.get("id").and_then(|v| v.as_str()) else {
            continue;
        };
        // Never touch a request actually in-flight on this SSP.
        if job_control.inflight.contains_key(id) {
            continue;
        }
        // Reset to pending (preserving retries/errors) before re-enqueueing.
        if let Err(e) = update_status_helper(db, id, "pending").await {
            warn!(target: "ssp::job_recovery", job_id = %id, error = %e, "Failed to reset stale processing job");
            continue;
        }
        if enqueue_recovered(job_control, job_queue_tx, backend_info, id, row).await {
            recovered += 1;
            warn!(target: "ssp::job_recovery", job_id = %id, "Recovered orphaned processing job");
        }
    }

    Ok(recovered)
}

/// Enqueue a recovered job, guarded by `mark_enqueued` so a job already moving
/// through the runner is never enqueued twice. Returns `true` if it was sent.
async fn enqueue_recovered(
    job_control: &JobControl,
    job_queue_tx: &mpsc::Sender<JobEntry>,
    backend_info: &BackendInfo,
    id: &str,
    row: &Value,
) -> bool {
    if !job_control.mark_enqueued(id) {
        return false; // already queued or in-flight
    }
    let timeout_override = row.get("timeout").and_then(|v| v.as_u64()).map(|v| v as u32);
    let job_entry = JobEntry::from_record(
        id.to_string(),
        backend_info.base_url.clone(),
        backend_info.auth_token.clone(),
        row,
        backend_info.effective_timeout(timeout_override),
    );
    if let Err(e) = job_queue_tx.send(job_entry).await {
        job_control.clear_enqueued(id);
        warn!(target: "ssp::job_recovery", job_id = %id, error = %e, "Failed to enqueue recovered job");
        return false;
    }
    true
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

                    // Mark before sending so the recovery sweep won't also
                    // enqueue this row while it's pending-in-channel. Skip if it
                    // is somehow already queued (e.g. a sweep beat us to it).
                    let job_id = job_entry.id.clone();
                    if state.job_control.mark_enqueued(&job_id) {
                        if let Err(e) = state.job_queue_tx.send(job_entry).await {
                            state.job_control.clear_enqueued(&job_id);
                            error!(error = %e, "Failed to queue job");
                        }
                    } else {
                        debug!(job_id = %job_id, "Skipping enqueue — already queued");
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
            // Compute the view-metrics inputs BEFORE handing the deltas off (by
            // value) to the coalescing flusher, which batches the actual
            // `_00_list_ref` edge writes into one transaction per
            // `query_update_throttle_ms` window instead of one per ingest.
            let record_counts: Vec<usize> =
                deltas_for_task.iter().map(|d| d.records.len()).collect();
            let query_ids: Vec<_> =
                deltas_for_task.iter().map(|d| d.query_id.clone()).collect();
            // If the flusher is gone (shutdown) the send fails — fall back to a
            // direct write so edges are never silently dropped.
            if let Err(send_err) = state_for_task.edge_update_tx.send(deltas_for_task) {
                let deltas = send_err.0;
                let delta_refs: Vec<&ViewDelta> = deltas.iter().collect();
                let circuit = state_for_task.processor.read().await;
                update_all_edges(
                    &state_for_task.db,
                    &delta_refs,
                    &state_for_task.metrics,
                    &circuit,
                    state_for_task.ref_mode,
                )
                .await;
            }
            persist_view_metrics(
                &state_for_task,
                record_counts,
                query_ids,
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
        // Same orphan-proof cleanup for the shared anonymous table: the owner
        // field above routes to the deleter's per-user table, never the anon
        // one, so anon windows would keep a stale edge if the circuit cache was
        // incomplete. Independent of `owner` (anon views are over public data).
        if state.anonymous_live_queries && parse_record_id(&payload.id).is_some() {
            let stmt = format!("DELETE _00_list_ref_anon WHERE out = {}", payload.id);
            let state_for_cleanup = state.clone();
            let id_for_log = payload.id.clone();
            tokio::spawn(async move {
                if let Err(e) = state_for_cleanup.db.query(&stmt).await {
                    error!(
                        target: "ssp::ingest",
                        id = %id_for_log,
                        error = %e,
                        "anon list_ref delete cleanup failed"
                    );
                }
            });
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
    // every downstream record-id computation uses the same value. When
    // `anonymousLiveQueries` is on, an empty `auth_id` (logged-out caller) is
    // remapped to the `anon` sentinel so this registration's `_00_query` row,
    // its list_ref edges, and the table it routes to all land on the shared,
    // world-readable `_00_list_ref_anon`. When off, the empty id keeps the
    // legacy behaviour (auth-gated global table the client can't read).
    let auth_id = {
        let raw = data
            .metadata
            .get("authId")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if raw.is_empty() && state.anonymous_live_queries {
            ssp_protocol::ANON_AUTH_ID.to_string()
        } else {
            raw.to_string()
        }
    };

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

    // Create initial edges. Hand them to the coalescing flusher so a page that
    // registers several queries at once lands their `_00_list_ref` edges as one
    // batched transaction (≤ query_update_throttle_ms later) rather than one
    // transaction per query. Falls back to a direct write if the flusher is gone.
    if let Some(delta) = update {
        debug!(incantation_id);
        if let Err(send_err) = state.edge_update_tx.send(vec![delta]) {
            let circuit = state.processor.read().await;
            update_incantation_edges(
                &state.db,
                &send_err.0[0],
                &state.metrics,
                &circuit,
                state.ref_mode,
            )
            .await;
        }
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

    // Per-table incremental XOR set-hashes (`x3:`), maintained per ingest.
    // The scheduler reconstructs these at the catch-up cut M to verify a
    // rejoining SSP before routing live traffic to it (see `verify_catchup_at_m`).
    let catchup_hashes: serde_json::Map<String, Value> = circuit
        .compute_catchup_hashes()
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
            "catchup_hashes": catchup_hashes,
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
pub(crate) fn parse_record_id(id: &str) -> Option<RecordId> {
    RecordId::parse_simple(id).ok()
}

/// Format incantation ID with the global `_00_query` prefix. Strips
/// any existing prefix from `id` and re-applies `_00_query:`. The
/// registration table is global in both ref modes because the client
/// needs to compute the record id without knowing the user at
/// id-creation time; only `_00_list_ref` splits per user.
pub(crate) fn format_incantation_id(id: &str) -> String {
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

    // Build the aggregated edge statements for the whole batch (primary window
    // edges + subquery child edges), reading record versions from the circuit
    // store. The statement SQL lives in `edge_updates::build_edge_batch`, which
    // is unit-tested; this function only adds metrics + the DB round-trip.
    struct StoreVersions<'a>(&'a Circuit);
    impl crate::edge_updates::RecordVersions for StoreVersions<'_> {
        fn version_of(&self, key: &str) -> i64 {
            self.0.store.get_record_version_by_key(key).unwrap_or(1)
        }
    }
    let batch = crate::edge_updates::build_edge_batch(deltas, mode, &StoreVersions(circuit));
    if batch.is_empty() {
        return;
    }

    span.record("total_operations", batch.statements.len());

    metrics.edge_operations.add(
        batch.created,
        &[opentelemetry::KeyValue::new("operation", "create")],
    );
    metrics.edge_operations.add(
        batch.updated,
        &[opentelemetry::KeyValue::new("operation", "update")],
    );
    metrics.edge_operations.add(
        batch.deleted,
        &[opentelemetry::KeyValue::new("operation", "delete")],
    );

    debug!(
        created = batch.created,
        updated = batch.updated,
        deleted = batch.deleted,
        views = deltas.len(),
        "Processing edge operations"
    );

    // Aggregate every statement into ONE SurrealDB transaction.
    let Some(full_query) = crate::edge_updates::wrap_in_transaction(&batch.statements) else {
        return;
    };

    let mut query = db.query(&full_query);
    let op_count = batch.statements.len();

    #[cfg(debug_assertions)]
    {
        let mut debug_query = full_query.clone();
        for (name, id) in &batch.bindings {
            let id_str = format!("{:?}", id);
            debug_query = debug_query.replace(&format!("${}", name), &id_str);
        }
        debug!(target: "ssp::edges::sql", "{}", debug_query);
    }

    for (name, id) in batch.bindings {
        query = query.bind((name, id));
    }

    // Execute transaction
    match query.await {
        Ok(_) => {
            debug!(
                target: "ssp::edges",
                operations = op_count,
                "Edge update transaction completed successfully"
            );
        }
        Err(e) => {
            error!(
                target: "ssp::edges",
                error = %e,
                operations = op_count,
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
