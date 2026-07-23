use anyhow::Context;
use axum::{
    Router,
    extract::{Request, State},
    http::{StatusCode, header::AUTHORIZATION},
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};

use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use ssp::circuit::{Circuit, Record, ViewDelta};
use ssp::circuit::view::OutputFormat;
use surrealdb::engine::remote::http::Client;
use surrealdb::Surreal;
use tokio::signal;
use tracing::{debug, error, info, warn};

// Expose modules for use in main.rs and tests
pub mod adapters;
pub mod crdt;
pub mod edge_updates;
pub mod maintenance_host;
pub mod metrics;
pub mod open_telemetry;
pub mod tables;
pub mod view_metrics;

use metrics::Metrics;
use view_metrics::ViewMetrics;

use ssp_node::jobs::{
    JobConfig, JobControl, JobEntry, JobRunner,
};
use tokio::sync::mpsc;

/// Shared database connection wrapped in Arc for zero-copy sharing across tasks
pub type SharedDb = Arc<Surreal<Client>>;

// Status types live in the portable core; re-exported for compatibility.
pub use ssp_node::{error_codes, SspError, SspStatus};

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
    /// Backend health monitoring — standalone mode only (`None` in cluster
    /// mode, where the scheduler owns backend healthchecks). Populated from
    /// `SPKY_BACKENDS` and live-updatable via `PUT /backends`.
    pub backend_health: Option<maintenance::BackendHealthCache>,
    /// Live-updatable backend configs backing the health monitor.
    pub shared_backend_configs: Option<maintenance::SharedBackendConfigs>,
    /// Platform port bundle (see `ssp-node` + `docs/platform-architecture.md`).
    /// Handlers migrate onto these ports incrementally; today it proves
    /// construction/object-safety and carries the VM adapters.
    pub platform: ssp_node::Platform,
    /// Bearer secret for the authenticated route group (`NodeConfig.auth_secret`).
    /// Empty accepts any bearer (dev only).
    pub auth_secret: String,
    /// The portable node — serves every route already migrated out of this
    /// shell (mounted as the axum fallback in `create_app`).
    pub node: Arc<ssp_node::SspNode>,
}

// --- Request/Response DTOs ---


// --- Configuration ---

/// Node configuration now lives in the portable core (`ssp_node::NodeConfig`)
/// so both shells share one definition; this shell builds it from env vars.
pub use ssp_node::NodeConfig as Config;

pub fn load_config() -> Config {
    Config {
        listen_addr: std::env::var("SPKY_SSP_LISTEN_ADDR").unwrap_or_else(|_| "0.0.0.0:8667".to_string()),
        // SPKY_DB_URL is canonical; SPKY_DB_WS kept as a legacy fallback
        // (any scheme is accepted — ws:// URLs are normalized to HTTP).
        db_addr: std::env::var("SPKY_DB_URL")
            .or_else(|_| std::env::var("SPKY_DB_WS"))
            .unwrap_or_else(|_| "http://127.0.0.1:8000".to_string()),
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
        auth_secret: std::env::var("SPKY_AUTH_SECRET").unwrap_or_default(),
        bootstrap_page_size: std::env::var("SPKY_SSP_BOOTSTRAP_PAGE_SIZE")
            .ok()
            .and_then(|s| s.parse().ok())
            .filter(|n: &usize| *n > 0)
            .unwrap_or(200),
        crdt_cache_size: std::env::var("SPKY_CRDT_CACHE_SIZE")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1024),
        health_check_interval_secs: std::env::var("SPKY_HEALTH_CHECK_INTERVAL_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(15),
        // VM holds the circuit in memory for its lifetime — no periodic
        // checkpoint (its CircuitStore is noop). Ephemeral hosts set this.
        checkpoint_interval_secs: None,
        max_snapshot_age_secs: 3600,
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
                // JSON flattening lives with the Db adapter so the port impl
                // and this bootstrap path can never drift.
                crate::adapters::SurrealSdkDb::flatten_first(db, surql).await
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

    let db_config = maintenance::db::DbConfig {
        url: config.db_addr.clone(),
        namespace: config.db_ns.clone(),
        database: config.db_db.clone(),
        username: config.db_user.clone(),
        password: config.db_pass.clone(),
    };
    let db = maintenance::db::connect_http(&db_config)
        .await
        .context("Failed to connect to SurrealDB")?;

    // HTTP-engine tokens expire; the run_server timer dispatcher keeps the
    // handle fresh via TimerKind::DbResignin (maintenance::db::resignin_once).

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

    // Single parser lives in ssp_node so every host (VM env var, CF Worker
    // binding) agrees on the shape + the null/[]→empty semantics.
    let cfg = JobConfig::from_json(&json);
    if cfg.job_tables.is_empty() {
        info!("SPKY_JOB_CONFIG lists no outbox backends, job runner disabled");
    } else {
        info!(job_tables = cfg.job_tables.len(), "Loaded job config from SPKY_JOB_CONFIG");
    }
    Arc::new(cfg)
}

// --- Router Setup ---

pub fn create_app(state: AppState) -> Router {
    // Routes NOT yet migrated into the portable core (ssp_node::SspNode).
    // Everything absent from this table falls through to `node_fallback`,
    // which dispatches into `node.route()` — migrated routes are simply
    // removed here, one at a time, keeping every commit green.
    //
    // Every SSP route now lives in the portable core (`SspNode::route`); the
    // shell is a pure adapter that converts axum requests and dispatches via
    // `node_fallback`. Auth + CORS are handled inside the core.
    Router::new().fallback(node_fallback).with_state(state)
}

/// Bridge axum → the portable node for every route the core already serves
/// (`/version`, `/log`, `/reset`, `/job/*`). Auth and CORS for these routes
/// live in `SspNode::route` — this shim only shuffles bytes.
async fn node_fallback(State(state): State<AppState>, req: Request) -> Response {
    let method = match *req.method() {
        axum::http::Method::GET => ssp_node::Method::Get,
        axum::http::Method::POST => ssp_node::Method::Post,
        axum::http::Method::PUT => ssp_node::Method::Put,
        axum::http::Method::DELETE => ssp_node::Method::Delete,
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    let path = req.uri().path().to_string();
    let bearer = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .map(|s| s.to_string());
    let body = match axum::body::to_bytes(req.into_body(), 32 * 1024 * 1024).await {
        Ok(b) => b,
        Err(_) => return StatusCode::PAYLOAD_TOO_LARGE.into_response(),
    };

    let api_req = ssp_node::ApiRequest { method, path, bearer, body };
    match state.node.route(api_req).await {
        Some(resp) => {
            let mut builder = Response::builder().status(resp.status);
            for (name, value) in &resp.headers {
                builder = builder.header(*name, value);
            }
            let (content_type, body_bytes) = match resp.body {
                ssp_node::ApiBody::Json(v) => ("application/json", v.to_string()),
                ssp_node::ApiBody::Text { content_type, body } => (content_type, body),
            };
            builder
                .header(axum::http::header::CONTENT_TYPE, content_type)
                .body(axum::body::Body::from(body_bytes))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
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

    // VM platform adapters (ssp-node ports) — constructed early because the
    // job runner consumes them. `timer_rx` feeds the timer dispatcher spawned
    // further down (once all its dependencies exist).
    let (platform, mut timer_rx) = adapters::vm_platform(db.clone(), metrics.clone());

    // HTTP-engine tokens expire; keep the long-lived shared handle fresh.
    platform
        .scheduler
        .schedule(
            ssp_node::TimerKind::DbResignin,
            ssp_node::now_epoch_ms() + maintenance::db::RESIGNIN_INTERVAL_SECS * 1000,
        )
        .await;

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
            Arc::clone(&platform.db),
            Arc::clone(&platform.http),
            Arc::clone(&platform.scheduler),
            Arc::clone(&platform.spawner),
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
            // Timer-driven (TimerKind::JobRecoverySweep): armed immediately so
            // the first pass runs as soon as the SSP is Ready (the startup
            // sweep), then re-armed every JOB_RECOVERY_INTERVAL_SECS by the
            // dispatcher below.
            platform
                .scheduler
                .schedule(ssp_node::TimerKind::JobRecoverySweep, ssp_node::now_epoch_ms())
                .await;
            info!(
                interval_secs = JOB_RECOVERY_INTERVAL_SECS,
                "Job recovery sweep timer armed"
            );
        } else {
            info!("Job recovery sweep disabled (cluster mode — recovery is the scheduler's responsibility)");
        }
    }

    // Clone for scheduler integration
    let processor_for_scheduler = processor_arc.clone();

    let crdt_cache = Arc::new(crdt::CrdtCache::new(
        config.crdt_cache_size,
        crdt::allow_list_from_env(),
    ));

    // Coalescing flusher for query edge-updates: edge writes to `_00_list_ref`
    // (from ingests and registrations) are batched into one transaction per
    // `query_update_throttle_ms` window so a burst of view updates lands as a
    // few batched LIVE deliveries instead of one transaction per record (which
    // paced a fresh client's window sync over ~10s). See `Config`.
    let (edge_update_tx, edge_update_rx) = mpsc::unbounded_channel::<Vec<ViewDelta>>();
    platform.spawner.spawn(Box::pin(edge_updates::run_edge_update_service(
        edge_update_rx,
        edge_updates::SurrealEdgeSink {
            db: Arc::clone(&platform.db),
            processor: processor_arc.clone(),
            telemetry: Arc::clone(&platform.telemetry),
            mode: config.ref_mode,
        },
        Arc::clone(&platform.scheduler),
        std::time::Duration::from_millis(config.query_update_throttle_ms),
        edge_updates::MAX_EDGE_BATCH,
    )));

    // Standalone mode: this SSP owns the maintenance plane the scheduler
    // provides in cluster mode — backend health monitoring and the
    // backup/restore workers + routes. Gated on the same condition as the
    // job recovery sweep.
    let standalone = config.scheduler_url.is_none();
    let (backend_health, shared_backend_configs) = if standalone {
        let backends = maintenance::backend_health::backends_from_env();
        let cache = maintenance::create_health_cache(&backends);
        let configs = maintenance::create_shared_configs(&backends);
        // Timer-driven (TimerKind::BackendHealth): first sweep immediately,
        // re-armed every health_check_interval_secs by the dispatcher below.
        platform
            .scheduler
            .schedule(ssp_node::TimerKind::BackendHealth, ssp_node::now_epoch_ms())
            .await;
        (Some(cache), Some(configs))
    } else {
        (None, None)
    };


    // The portable node: serves every route already migrated out of this
    // shell (dispatched via the axum fallback in `create_app`).
    // Env vars surfaced verbatim in `/info` (the core never reads the
    // environment — the shell collects these once here).
    let info_env: Vec<(String, String)> = [
        "SPKY_DB_URL", "SPKY_DB_NS", "SPKY_DB_NAME", "SPKY_DB_USER",
        "SPKY_SCHEDULER_URL", "SPKY_SSP_LISTEN_ADDR", "SPKY_SSP_ADVERTISE_ADDR", "SPKY_SSP_ID",
        "HEARTBEAT_INTERVAL_MS", "TTL_CLEANUP_INTERVAL_SECS",
    ]
    .iter()
    .filter_map(|&key| std::env::var(key).ok().map(|val| (key.to_string(), val)))
    .collect();
    // Derive IP from SPKY_SSP_ADVERTISE_ADDR (e.g. "10.100.1.30:8667" -> "10.100.1.30").
    let advertise_ip = config
        .advertise_addr
        .as_deref()
        .and_then(|addr| addr.split(':').next().map(|s| s.to_string()));

    let node_backend_health: Option<Arc<dyn ssp_node::ports::BackendHealth>> =
        match (backend_health.clone(), shared_backend_configs.clone()) {
            (Some(cache), Some(configs)) => Some(Arc::new(
                adapters::MaintenanceBackendHealth { cache, configs },
            )),
            _ => None,
        };

    // Shared between the still-shell ingest handler (via AppState) and the
    // migrated register/unregister/crdt handlers (via SspNode) — one instance.
    let view_metrics = Arc::new(RwLock::new(std::collections::HashMap::new()));

    let node = Arc::new(ssp_node::SspNode {
        platform: platform.clone(),
        status: status.clone(),
        processor: processor_arc.clone(),
        job_config: job_config.clone(),
        job_control: job_control.clone(),
        job_queue_tx: job_queue_tx.clone(),
        ssp_id: config.ssp_id.clone(),
        auth_secret: config.auth_secret.clone(),
        ref_mode: config.ref_mode,
        version: env!("CARGO_PKG_VERSION"),
        surrealdb_version: surrealdb_version.clone(),
        advertise_ip,
        info_env,
        start_epoch_ms: ssp_node::now_epoch_ms(),
        backend_health: node_backend_health,
        crdt_cache: crdt_cache.clone(),
        view_metrics: view_metrics.clone(),
        edge_update_tx: edge_update_tx.clone(),
        anonymous_live_queries: config.anonymous_live_queries,
        standalone: config.scheduler_url.is_none(),
        ttl_cleanup_interval_secs: config.ttl_cleanup_interval_secs,
        bootstrap_page_size: config.bootstrap_page_size,
        checkpoint_interval_secs: config.checkpoint_interval_secs,
        max_snapshot_age_secs: config.max_snapshot_age_secs,
    });
    let runtime = ssp_node::Runtime::new(node.clone());

    // Timer dispatcher: the VM shell's `on_timer` equivalent. Portable arms
    // (JobRecoverySweep/TtlCleanup/CircuitCheckpoint) run + re-arm in the core
    // via `runtime.on_timer` — the exact shape a DO shell drives from alarm().
    // Host-specific arms (BackendHealth/DbResignin — non-wasm `maintenance` +
    // reqwest) stay inline here.
    {
        let runtime = runtime.clone();
        let db = db.clone();
        let scheduler = Arc::clone(&platform.scheduler);
        let backend_cache = backend_health.clone();
        let backend_configs = shared_backend_configs.clone();
        let health_client = maintenance::backend_health::health_http_client();
        let resignin_db_config = maintenance::DbConfig {
            url: config.db_addr.clone(),
            namespace: config.db_ns.clone(),
            database: config.db_db.clone(),
            username: config.db_user.clone(),
            password: config.db_pass.clone(),
        };
        let health_interval_ms = config.health_check_interval_secs * 1000;
        tokio::spawn(async move {
            while let Some(kind) = timer_rx.recv().await {
                match &kind {
                    ssp_node::TimerKind::BackendHealth => {
                        if let (Some(configs), Some(cache)) = (&backend_configs, &backend_cache) {
                            maintenance::backend_health::check_backends_once(configs, cache, &health_client).await;
                            scheduler
                                .schedule(kind.clone(), ssp_node::now_epoch_ms() + health_interval_ms)
                                .await;
                        }
                    }
                    ssp_node::TimerKind::DbResignin => {
                        maintenance::db::resignin_once(&db, &resignin_db_config).await;
                        scheduler
                            .schedule(
                                kind.clone(),
                                ssp_node::now_epoch_ms() + maintenance::db::RESIGNIN_INTERVAL_SECS * 1000,
                            )
                            .await;
                    }
                    _ => runtime.on_timer(kind).await,
                }
            }
        });
    }

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
        view_metrics,
        ref_mode: config.ref_mode,
        anonymous_live_queries: config.anonymous_live_queries,
        surrealdb_version,
        edge_update_tx,
        backend_health,
        shared_backend_configs,
        platform: platform.clone(),
        auth_secret: config.auth_secret.clone(),
        node,
    };

    let mut app = create_app(state);

    if standalone {
        // Backup/restore plane (same routes as the scheduler's port 9667,
        // but behind this SSP's bearer auth — spooky-cloud must send
        // `Authorization: Bearer $SPKY_AUTH_SECRET` when targeting an SSP).
        let host: Arc<dyn maintenance::MaintenanceHost> =
            Arc::new(maintenance_host::SspHost {
                db: db.clone(),
                processor: processor_arc.clone(),
                status: status.clone(),
                platform: platform.clone(),
                ref_mode: config.ref_mode,
                bootstrap_page_size: config.bootstrap_page_size,
            });
        let db_config = Arc::new(maintenance::DbConfig {
            url: config.db_addr.clone(),
            namespace: config.db_ns.clone(),
            database: config.db_db.clone(),
            username: config.db_user.clone(),
            password: config.db_pass.clone(),
        });
        let backup_config = Arc::new(maintenance::BackupConfig::from_env());
        let backup_registry = Arc::new(maintenance::BackupRegistry::new());
        let restore_registry = Arc::new(maintenance::RestoreRegistry::new());
        let (backup_tx, backup_rx) = maintenance::create_backup_channel();
        let (restore_tx, restore_rx) = maintenance::create_restore_channel();
        let backup_restore_lock = Arc::new(tokio::sync::Mutex::new(()));

        tokio::spawn(maintenance::run_backup_worker(
            backup_rx,
            Arc::clone(&host),
            Arc::clone(&backup_config),
            Arc::clone(&db_config),
            Arc::clone(&backup_registry),
            Arc::clone(&backup_restore_lock),
        ));
        tokio::spawn(maintenance::run_restore_worker(
            restore_rx,
            Arc::clone(&host),
            Arc::clone(&backup_config),
            Arc::clone(&db_config),
            Arc::clone(&restore_registry),
            Arc::clone(&backup_restore_lock),
        ));

        let backup_router = maintenance::create_backup_router(maintenance::BackupState {
            host,
            config: backup_config,
            registry: backup_registry,
            tx: backup_tx,
            restore_registry,
            restore_tx,
            backup_restore_lock,
        })
        .layer(middleware::from_fn_with_state(
            config.auth_secret.clone(),
            auth_middleware,
        ));
        app = app.merge(backup_router);
        info!("Standalone maintenance plane active: /backup/*, /backends, backend health monitor");
    }

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
        let bootstrap_page_size = config.bootstrap_page_size;

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
                match self_bootstrap_with_metadata(&metadata_source, &data_source, &processor, bootstrap_page_size).await {
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
                            let mut diffs = ssp_protocol::snapshot_hash::diff_table_hashes(
                                &expected_hashes,
                                &actual,
                            );
                            // Synced `_00_*` meta tables (feature flags, app
                            // releases) are low-stakes announcement data — a
                            // hash mismatch there must degrade to a warning,
                            // never crash-loop the SSP and take sync down for
                            // every real table (observed live when
                            // _00_app_release first joined the snapshot).
                            diffs.retain(|d| {
                                let meta = ssp_protocol::SYNCED_META_TABLES
                                    .contains(&d.table.as_str());
                                if meta {
                                    warn!(
                                        table = %d.table,
                                        expected = %d.a,
                                        actual = %d.b,
                                        "Ignoring integrity mismatch on synced meta table"
                                    );
                                }
                                !meta
                            });
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

    // TTL cleanup: timer-driven (TimerKind::TtlCleanup), re-armed by the
    // dispatcher every ttl_cleanup_interval_secs.
    platform
        .scheduler
        .schedule(
            ssp_node::TimerKind::TtlCleanup,
            ssp_node::now_epoch_ms() + config.ttl_cleanup_interval_secs * 1000,
        )
        .await;
    info!(interval_secs = config.ttl_cleanup_interval_secs, "TTL cleanup timer armed");

    // Circuit checkpoint: only when a host opts in (ephemeral/edge). The VM
    // leaves checkpoint_interval_secs unset — its NoopCircuitStore holds the
    // circuit in-process, so periodic snapshots would be pure overhead.
    if let Some(secs) = config.checkpoint_interval_secs {
        platform
            .scheduler
            .schedule(
                ssp_node::TimerKind::CircuitCheckpoint,
                ssp_node::now_epoch_ms() + secs * 1000,
            )
            .await;
        info!(interval_secs = secs, "Circuit checkpoint timer armed");
    }

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(meter_provider, runtime.clone()))
        .await
        .context("Server error")?;

    opentelemetry::global::shutdown_tracer_provider();

    Ok(())
}

async fn shutdown_signal(
    meter_provider: opentelemetry_sdk::metrics::SdkMeterProvider,
    runtime: ssp_node::Runtime,
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

    // Persist a final snapshot so an ephemeral host restarts warm. No-op on the
    // VM (NoopCircuitStore) and skipped unless the circuit is Ready.
    runtime.checkpoint().await;

    if let Err(e) = meter_provider.shutdown() {
        error!(error = %e, "Failed to shutdown meter provider");
    }
}

// --- Self-Bootstrap ---
//
// The pure bootstrap helpers (`bootstrap_page_query`, `parse_link_target`,
// `extract_select_permission_text`) and the standalone rebuild now live in the
// portable core (`ssp_node::bootstrap`); the shell keeps only the cluster
// split-source path below.

#[cfg(test)]
mod bootstrap_pagination_tests {
    use ssp_node::bootstrap::bootstrap_page_query;
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
            "SELECT * FROM game WHERE id > type::record('game', 'abc') ORDER BY id LIMIT 200"
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
    page_size: usize,
) -> anyhow::Result<()> {
    info!("Starting self-bootstrap");

    // Standalone (metadata + data both Direct against the local DB): the
    // canonical rebuild lives in the portable core, over the `Db` port — one
    // implementation, exercised by the reference host + ssp-node tests. The
    // cluster split-source path (metadata Direct, data via the scheduler
    // Proxy) can't use a single-DB port, so it stays inline below.
    if let (BootstrapSource::Direct(_), BootstrapSource::Direct(db)) =
        (metadata_source, data_source)
    {
        return ssp_node::bootstrap::rebuild_from_db(
            &crate::adapters::SurrealSdkDb::new(db.clone()),
            processor,
            page_size,
        )
        .await;
    }

    use ssp_node::bootstrap::{
        bootstrap_page_query, extract_select_permission_text, parse_link_target,
    };
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
            .filter(|(name, _)| !ssp_protocol::table_excluded_from_sync(name))
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

    // Step 1c: Per-table record-link map. `INFO FOR TABLE` exposes each field's
    // `DEFINE FIELD ... TYPE record<X>`; capture `field -> X` so the
    // registration pipeline can lower a link-traversal permission
    // (`assigned_to.owner.id = $auth.id`) into a SemiJoin against `X`. The
    // target table isn't derivable from the field name (`assigned_to` links to
    // `connection`), and a flat Filter can't dereference a link across rows, so
    // without this every live query on an outbox table matches zero rows.
    // Best-effort: a failed INFO or an unparseable field just leaves that link
    // unresolved (its permission stays flat, i.e. today's behavior).
    {
        let mut resolved: Vec<(String, String, String)> = Vec::new();
        for table in &tables {
            let info = match metadata_source.query(&format!("INFO FOR TABLE {}", table)).await {
                Ok(v) => v,
                Err(e) => {
                    warn!(target: "ssp::policy", table = %table, error = %e, "INFO FOR TABLE failed; skipping link map");
                    continue;
                }
            };
            let Some(fields) = info.get("fields").and_then(|f| f.as_object()) else {
                continue;
            };
            for (field_name, def) in fields {
                if let Some(target) = def.as_str().and_then(parse_link_target) {
                    resolved.push((table.clone(), field_name.clone(), target));
                }
            }
        }
        if !resolved.is_empty() {
            let mut circuit = processor.write().await;
            for (table, field, target) in &resolved {
                info!(
                    target: "ssp::policy",
                    table = %table, field = %field, link_target = %target,
                    "registered record-link target"
                );
                circuit.set_link_target(table.clone(), field.clone(), target.clone());
            }
        }
    }

    // Step 2: Load all table data, paged. Pulling the entire table in one
    // request blew up at multi-GB DBs because the SurrealDB engine or the
    // scheduler proxy had to materialise the full result set in one HTTP
    // body. Paging keeps each round-trip bounded; the SSP still loads
    // everything into the circuit store but does so a chunk at a time.
    // `page_size` comes from NodeConfig (env: SPKY_SSP_BOOTSTRAP_PAGE_SIZE).
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
            ssp::service::view::prepare_registration_dbsp(
                payload,
                circuit.permissions(),
                circuit.link_targets(),
            )
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

// --- Middleware ---

/// Bearer auth against the config-carried secret (`NodeConfig.auth_secret`) —
/// no per-request env read; the CF shell provides the same secret from a
/// Worker binding.
async fn auth_middleware(
    State(secret): State<String>,
    req: Request,
    next: Next,
) -> Response {
    let auth_header = req.headers().get(AUTHORIZATION);

    match auth_header {
        Some(header) if header.to_str().unwrap_or_default() == format!("Bearer {}", secret) => {
            next.run(req).await
        }
        _ => StatusCode::UNAUTHORIZED.into_response(),
    }
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
// --- TTL Cleanup ---

/// Thin shell wrapper over `ssp_node::ttl_cleanup_sweep` (the single impl),
/// adapting the VM's concrete `SharedDb` + OTel `Metrics` to the `Db` +
/// `Telemetry` ports. Kept for the DB integration tests that call it directly.
pub async fn ttl_cleanup_sweep(
    db: &SharedDb,
    processor: &Arc<RwLock<Circuit>>,
    metrics: &Arc<Metrics>,
    mode: ssp_protocol::RefMode,
) -> usize {
    ssp_node::ttl_cleanup_sweep(
        &crate::adapters::SurrealSdkDb::new(db.clone()),
        processor,
        &crate::adapters::OtelTelemetry::new(metrics.clone()),
        mode,
    )
    .await
}

// --- Helper Functions ---





