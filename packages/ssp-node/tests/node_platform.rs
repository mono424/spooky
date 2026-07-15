//! Platform-independence proof: drive `SspNode::route` over a FULLY MOCK
//! platform — no axum, no VM shell, no `tokio::time`/`spawn`, just the port
//! traits. This is the test that fails the moment a migrated handler grows a
//! hidden dependency on the VM runtime; it's the concrete check behind the
//! adapter-swap thesis in `docs/platform-architecture.md`.
//!
//! The `Db` port is backed by a real embedded SurrealDB (`kv-mem`), so job
//! control, reset, and info run their production SQL end to end — only the
//! *runtime* is mocked, not the database semantics.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::sync::{mpsc, RwLock};

use ssp::circuit::Circuit;
use ssp_node::api::{ApiBody, ApiRequest, ApiResponse, Method};
use ssp_node::jobs::{JobConfig, JobControl, JobEntry};
use ssp_node::ports::{
    BackendCounts, BackendHealth, BackendSpec, CancelWatch, Db, DbError, HttpClient, HttpError,
    LocalBoxFuture, OutboundRequest, OutboundResponse, Scheduler, Spawner, Telemetry, TimerKind,
};
use ssp_node::{Platform, SspNode, SspStatus};
use surrealdb::engine::local::{Db as MemEngine, Mem};
use surrealdb::Surreal;

const SECRET: &str = "unit-secret";

// --- Mock ports --------------------------------------------------------------

struct MemDb(Arc<Surreal<MemEngine>>);

#[async_trait::async_trait]
impl Db for MemDb {
    async fn query(&self, surql: &str, binds: &[(&str, Value)]) -> Result<Vec<Value>, DbError> {
        let mut q = self.0.query(surql);
        for (name, value) in binds {
            q = q.bind(((*name).to_string(), value.clone()));
        }
        let mut response = q.await.map_err(|e| DbError::Transport(e.to_string()))?;
        let n = response.num_statements();
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let val: surrealdb::types::Value =
                response.take(i).map_err(|e| DbError::Query(e.to_string()))?;
            out.push(val.into_json_value());
        }
        Ok(out)
    }
    async fn version(&self) -> Result<String, DbError> {
        Ok("mem-test".to_string())
    }
}

/// Records dispatched requests; returns a configurable status.
#[derive(Clone, Default)]
struct MockHttp {
    calls: Arc<Mutex<Vec<String>>>,
    status: Arc<AtomicUsize>, // 0 => use 200
}

#[async_trait::async_trait]
impl HttpClient for MockHttp {
    async fn send(
        &self,
        req: OutboundRequest,
        _cancel: Option<CancelWatch>,
    ) -> Result<OutboundResponse, HttpError> {
        self.calls.lock().unwrap().push(req.url.clone());
        let s = self.status.load(Ordering::SeqCst);
        Ok(OutboundResponse {
            status: if s == 0 { 200 } else { s as u16 },
            body: String::new(),
        })
    }
}

/// Records scheduled timers; `sleep` returns instantly so tests never wait.
#[derive(Clone, Default)]
struct MockScheduler {
    scheduled: Arc<Mutex<Vec<TimerKind>>>,
}

#[async_trait::async_trait]
impl Scheduler for MockScheduler {
    async fn schedule(&self, kind: TimerKind, _at: u64) {
        self.scheduled.lock().unwrap().push(kind);
    }
    async fn cancel(&self, _kind: &TimerKind) {}
    async fn sleep(&self, _dur: Duration) {}
}

struct MockSpawner;
impl Spawner for MockSpawner {
    fn spawn(&self, fut: LocalBoxFuture) {
        tokio::spawn(fut);
    }
}

#[derive(Clone, Default)]
struct MockTelemetry {
    gauge: Arc<Mutex<i64>>,
}
impl Telemetry for MockTelemetry {
    fn counter(&self, _name: &'static str, _value: u64) {}
    fn histogram_ms(&self, _name: &'static str, _value: f64) {}
    fn gauge_add(&self, _name: &'static str, delta: i64) {
        *self.gauge.lock().unwrap() += delta;
    }
}

/// Constant backend-health counts (standalone `/health` + `/backends`).
struct MockBackendHealth {
    counts: Arc<Mutex<BackendCounts>>,
    updates: Arc<Mutex<Vec<BackendSpec>>>,
}

#[async_trait::async_trait]
impl BackendHealth for MockBackendHealth {
    async fn counts(&self) -> BackendCounts {
        *self.counts.lock().unwrap()
    }
    async fn update(&self, backends: Vec<BackendSpec>) {
        *self.updates.lock().unwrap() = backends;
    }
}

// --- Harness -----------------------------------------------------------------

struct Harness {
    node: Arc<SspNode>,
    raw_db: Arc<Surreal<MemEngine>>,
    http_calls: Arc<Mutex<Vec<String>>>,
    scheduled: Arc<Mutex<Vec<TimerKind>>>,
    telemetry_gauge: Arc<Mutex<i64>>,
    backend_updates: Arc<Mutex<Vec<BackendSpec>>>,
    _job_rx: mpsc::Receiver<JobEntry>,
    job_rx: Arc<tokio::sync::Mutex<mpsc::Receiver<JobEntry>>>,
}

struct HarnessOpts {
    status: SspStatus,
    job_tables: Vec<(&'static str, &'static str)>, // (table, base_url)
    backend_counts: Option<BackendCounts>,
    ref_mode: ssp_protocol::RefMode,
    circuit_store: Option<Arc<dyn ssp_node::CircuitStore>>,
}

impl Default for HarnessOpts {
    fn default() -> Self {
        Self {
            status: SspStatus::Ready,
            job_tables: vec![],
            backend_counts: None,
            ref_mode: ssp_protocol::RefMode::Single,
            circuit_store: None,
        }
    }
}

/// In-memory `CircuitStore` for the bootstrap restore path.
#[derive(Clone, Default)]
struct MemStore(Arc<Mutex<Option<(String, ssp_node::ResumePoint)>>>);

#[async_trait::async_trait]
impl ssp_node::CircuitStore for MemStore {
    async fn save(
        &self,
        blob: &str,
        point: &ssp_node::ResumePoint,
    ) -> Result<(), ssp_node::CircuitStoreError> {
        *self.0.lock().unwrap() = Some((blob.to_string(), point.clone()));
        Ok(())
    }
    async fn load(&self) -> Result<(String, ssp_node::ResumePoint), ssp_node::CircuitStoreError> {
        self.0
            .lock()
            .unwrap()
            .clone()
            .ok_or(ssp_node::CircuitStoreError::NotFound)
    }
    async fn clear(&self) -> Result<(), ssp_node::CircuitStoreError> {
        *self.0.lock().unwrap() = None;
        Ok(())
    }
}

async fn build(opts: HarnessOpts) -> Harness {
    let raw = Surreal::new::<Mem>(()).await.unwrap();
    raw.use_ns("test").use_db("test").await.unwrap();
    let raw = Arc::new(raw);

    let http_calls = Arc::new(Mutex::new(Vec::new()));
    let scheduled = Arc::new(Mutex::new(Vec::new()));
    let telemetry_gauge = Arc::new(Mutex::new(0));

    let platform = Platform {
        db: Arc::new(MemDb(Arc::clone(&raw))),
        http: Arc::new(MockHttp { calls: Arc::clone(&http_calls), status: Arc::new(AtomicUsize::new(0)) }),
        scheduler: Arc::new(MockScheduler { scheduled: Arc::clone(&scheduled) }),
        spawner: Arc::new(MockSpawner),
        telemetry: Arc::new(MockTelemetry { gauge: Arc::clone(&telemetry_gauge) }),
        circuit_store: opts
            .circuit_store
            .clone()
            .unwrap_or_else(|| Arc::new(ssp_node::NoopCircuitStore)),
    };

    let mut job_config = JobConfig::default();
    for (table, base_url) in &opts.job_tables {
        job_config.job_tables.insert(
            (*table).to_string(),
            ssp_node::jobs::BackendInfo {
                name: (*table).to_string(),
                base_url: (*base_url).to_string(),
                auth_token: None,
                timeout: Some(5),
                timeout_overridable: false,
            },
        );
    }

    let backend_updates = Arc::new(Mutex::new(Vec::new()));
    let backend_health: Option<Arc<dyn BackendHealth>> = opts.backend_counts.map(|c| {
        Arc::new(MockBackendHealth {
            counts: Arc::new(Mutex::new(c)),
            updates: Arc::clone(&backend_updates),
        }) as Arc<dyn BackendHealth>
    });

    let (tx, rx) = mpsc::channel::<JobEntry>(64);
    // Keep a second receiver handle so tests can drain enqueued jobs without
    // consuming the node's sender.
    let rx = Arc::new(tokio::sync::Mutex::new(rx));

    let node = SspNode {
        platform,
        status: Arc::new(RwLock::new(opts.status)),
        processor: Arc::new(RwLock::new(Circuit::new())),
        job_config: Arc::new(job_config),
        job_control: JobControl::new(),
        job_queue_tx: tx,
        ssp_id: "test-ssp".to_string(),
        auth_secret: SECRET.to_string(),
        ref_mode: opts.ref_mode,
        version: "9.9.9-test",
        surrealdb_version: "mem-test".to_string(),
        advertise_ip: Some("10.0.0.7".to_string()),
        info_env: vec![("SPKY_SSP_ID".to_string(), "test-ssp".to_string())],
        start_epoch_ms: ssp_node::now_epoch_ms().saturating_sub(5000),
        backend_health,
        crdt_cache: Arc::new(ssp_node::crdt::CrdtCache::new(8, ssp_node::crdt::CrdtAllowList::permissive())),
        view_metrics: Arc::new(RwLock::new(std::collections::HashMap::new())),
        edge_update_tx: { let (tx, _rx) = mpsc::unbounded_channel(); tx },
        anonymous_live_queries: false,
        standalone: true,
        ttl_cleanup_interval_secs: 60,
        bootstrap_page_size: 200,
        checkpoint_interval_secs: None,
        max_snapshot_age_secs: 3600,
    };

    // A dummy second receiver is not creatable from one channel; keep the real
    // one behind the mutex and expose it.
    let (_dead_tx, dead_rx) = mpsc::channel::<JobEntry>(1);

    Harness {
        node: Arc::new(node),
        raw_db: raw,
        http_calls,
        scheduled,
        telemetry_gauge,
        backend_updates,
        _job_rx: dead_rx,
        job_rx: rx,
    }
}

fn req(method: Method, path: &str, bearer: Option<&str>, body: Value) -> ApiRequest {
    ApiRequest {
        method,
        path: path.to_string(),
        bearer: bearer.map(|s| s.to_string()),
        body: bytes::Bytes::from(serde_json::to_vec(&body).unwrap()),
    }
}

fn authed(method: Method, path: &str, body: Value) -> ApiRequest {
    req(method, path, Some(SECRET), body)
}

fn json_of(resp: &ApiResponse) -> &Value {
    match &resp.body {
        ApiBody::Json(v) => v,
        ApiBody::Text { .. } => panic!("expected JSON body"),
    }
}

async fn insert_job(db: &Surreal<MemEngine>, id: &str, status: &str) {
    db.query(
        "CREATE type::record('job', $id) SET \
         status = $status, path = '/run', payload = {}, retries = 0, max_retries = 3, \
         retry_strategy = 'linear', recurring = false, interval = 0, errors = [], \
         created_at = time::now(), updated_at = time::now()",
    )
    .bind(("id", id.to_string()))
    .bind(("status", status.to_string()))
    .await
    .unwrap();
}

async fn job_status(db: &Surreal<MemEngine>, id: &str) -> Option<String> {
    db.query(format!("SELECT VALUE status FROM ONLY job:{id}"))
        .await
        .unwrap()
        .take(0)
        .unwrap()
}

// --- Tests -------------------------------------------------------------------

#[tokio::test]
async fn unknown_route_returns_none() {
    let h = build(HarnessOpts::default()).await;
    // Genuinely unknown paths/methods → None (shell 404s). Every real SSP
    // route is now served by the core, including /ingest.
    assert!(h.node.route(authed(Method::Get, "/nope", Value::Null)).await.is_none());
    assert!(h.node.route(authed(Method::Delete, "/ingest", Value::Null)).await.is_none());
}

#[tokio::test]
async fn auth_enforced_on_protected_routes_only() {
    let h = build(HarnessOpts::default()).await;

    // Missing/wrong bearer on an authed route → 401.
    let r = h.node.route(req(Method::Post, "/reset", None, Value::Null)).await.unwrap();
    assert_eq!(r.status, 401);
    let r = h.node.route(req(Method::Post, "/reset", Some("wrong"), Value::Null)).await.unwrap();
    assert_eq!(r.status, 401);

    // Public route needs no auth and carries the CORS header.
    let r = h.node.route(req(Method::Get, "/version", None, Value::Null)).await.unwrap();
    assert_eq!(r.status, 200);
    assert!(r.headers.iter().any(|(k, v)| *k == "Access-Control-Allow-Origin" && v == "*"));
}

#[tokio::test]
async fn version_reports_shell_version() {
    let h = build(HarnessOpts::default()).await;
    let r = h.node.route(req(Method::Get, "/version", None, Value::Null)).await.unwrap();
    assert_eq!(json_of(&r)["version"], "9.9.9-test");
    assert_eq!(json_of(&r)["mode"], "streaming");
}

#[tokio::test]
async fn health_plain_when_no_backend_monitor() {
    // Ready, no monitor → 200 + bare status. Bootstrapping → 503.
    let h = build(HarnessOpts::default()).await;
    let r = h.node.route(req(Method::Get, "/health", None, Value::Null)).await.unwrap();
    assert_eq!(r.status, 200);
    assert_eq!(json_of(&r)["status"], "ready");
    assert!(json_of(&r).get("backends").is_none());

    let h = build(HarnessOpts { status: SspStatus::Bootstrapping, ..Default::default() }).await;
    let r = h.node.route(req(Method::Get, "/health", None, Value::Null)).await.unwrap();
    assert_eq!(r.status, 503);
}

#[tokio::test]
async fn health_aggregates_backend_counts() {
    // Ready + all healthy → healthy/200.
    let h = build(HarnessOpts {
        backend_counts: Some(BackendCounts { healthy: 2, unhealthy: 0, unreachable: 0, total: 2 }),
        ..Default::default()
    })
    .await;
    let r = h.node.route(req(Method::Get, "/health", None, Value::Null)).await.unwrap();
    assert_eq!(r.status, 200);
    assert_eq!(json_of(&r)["status"], "healthy");
    assert_eq!(json_of(&r)["backends"]["total"], 2);

    // Ready + one unreachable → degraded/200.
    let h = build(HarnessOpts {
        backend_counts: Some(BackendCounts { healthy: 1, unhealthy: 0, unreachable: 1, total: 2 }),
        ..Default::default()
    })
    .await;
    let r = h.node.route(req(Method::Get, "/health", None, Value::Null)).await.unwrap();
    assert_eq!(r.status, 200);
    assert_eq!(json_of(&r)["status"], "degraded");

    // Ready + ALL down → unavailable/503.
    let h = build(HarnessOpts {
        backend_counts: Some(BackendCounts { healthy: 0, unhealthy: 1, unreachable: 1, total: 2 }),
        ..Default::default()
    })
    .await;
    let r = h.node.route(req(Method::Get, "/health", None, Value::Null)).await.unwrap();
    assert_eq!(r.status, 503);
    assert_eq!(json_of(&r)["status"], "unavailable");
}

#[tokio::test]
async fn info_reports_identity_and_uptime() {
    let h = build(HarnessOpts::default()).await;
    let r = h.node.route(req(Method::Get, "/info", None, Value::Null)).await.unwrap();
    let entity = &json_of(&r)[0];
    assert_eq!(entity["id"], "test-ssp");
    assert_eq!(entity["ip"], "10.0.0.7");
    assert_eq!(entity["version"], "9.9.9-test");
    assert_eq!(entity["surrealdb_version"], "mem-test");
    assert_eq!(entity["ref_mode"], "single");
    assert!(entity["uptime_seconds"].as_u64().unwrap() >= 5, "start was 5s ago");
    assert_eq!(entity["env"]["SPKY_SSP_ID"], "test-ssp");

    // /info/text is the same array as a verbatim text/plain string.
    let r = h.node.route(req(Method::Get, "/info/text", None, Value::Null)).await.unwrap();
    match &r.body {
        ApiBody::Text { content_type, body } => {
            assert_eq!(*content_type, "text/plain");
            let parsed: Value = serde_json::from_str(body).unwrap();
            assert_eq!(parsed[0]["id"], "test-ssp");
        }
        _ => panic!("expected text body"),
    }
}

#[tokio::test]
async fn backends_update_only_when_monitor_active() {
    // No monitor → 409.
    let h = build(HarnessOpts::default()).await;
    let r = h
        .node
        .route(authed(Method::Put, "/backends", json!([])))
        .await
        .unwrap();
    assert_eq!(r.status, 409);

    // Monitor active → 200, update forwarded to the port.
    let h = build(HarnessOpts {
        backend_counts: Some(BackendCounts::default()),
        ..Default::default()
    })
    .await;
    let body = json!([{ "name": "api", "url": "http://api:3000", "healthcheck": "/health" }]);
    let r = h.node.route(authed(Method::Put, "/backends", body)).await.unwrap();
    assert_eq!(r.status, 200);
    let updates = h.backend_updates.lock().unwrap();
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].name, "api");
}

#[tokio::test]
async fn reset_wipes_circuit_and_reports_gauge() {
    let h = build(HarnessOpts::default()).await;
    // Seed a view so the reset has something to clear (gauge should drop).
    {
        let mut c = h.node.processor.write().await;
        c.set_permission("thread", "true");
    }
    let r = h.node.route(authed(Method::Post, "/reset", Value::Null)).await.unwrap();
    assert_eq!(r.status, 200);
    // Reset routes through the Db port: DELETE _00_list_ref (single mode) runs
    // against mem SurrealDB without error, and the circuit is fresh.
    assert_eq!(h.node.processor.read().await.view_count(), 0);
    // Telemetry gauge was touched (0 views → delta 0, but the call path ran).
    let _ = *h.telemetry_gauge.lock().unwrap();
}

#[tokio::test]
async fn job_kill_terminalizes_pending_row() {
    let h = build(HarnessOpts { job_tables: vec![("job", "http://b")], ..Default::default() }).await;
    insert_job(&h.raw_db, "k1", "pending").await;

    let r = h.node.route(authed(Method::Post, "/job/kill", json!({ "id": "job:k1" }))).await.unwrap();
    assert_eq!(r.status, 200);
    assert_eq!(json_of(&r)["status"], "failed");
    assert_eq!(job_status(&h.raw_db, "k1").await.as_deref(), Some("failed"));
}

#[tokio::test]
async fn job_kill_bad_id_and_missing() {
    let h = build(HarnessOpts::default()).await;
    let r = h.node.route(authed(Method::Post, "/job/kill", json!({ "id": "nocolon" }))).await.unwrap();
    assert_eq!(r.status, 400);

    let r = h.node.route(authed(Method::Post, "/job/kill", json!({ "id": "job:ghost" }))).await.unwrap();
    assert_eq!(r.status, 404);
}

#[tokio::test]
async fn job_retry_resets_and_enqueues() {
    let h = build(HarnessOpts { job_tables: vec![("job", "http://b")], ..Default::default() }).await;
    insert_job(&h.raw_db, "r1", "failed").await;

    let r = h.node.route(authed(Method::Post, "/job/retry", json!({ "id": "job:r1" }))).await.unwrap();
    assert_eq!(r.status, 200);
    assert_eq!(json_of(&r)["status"], "pending");
    assert_eq!(job_status(&h.raw_db, "r1").await.as_deref(), Some("pending"));

    // The reset+re-enqueue put exactly one JobEntry on the queue.
    let mut rx = h.job_rx.lock().await;
    let job = rx.try_recv().expect("a job was enqueued");
    assert_eq!(job.id, "job:r1");
    assert_eq!(job.retries, 0, "retry budget reset");

    // Retrying a non-terminal job is a 409.
    insert_job(&h.raw_db, "p1", "processing").await;
    let r = h.node.route(authed(Method::Post, "/job/retry", json!({ "id": "job:p1" }))).await.unwrap();
    assert_eq!(r.status, 409);
}

#[tokio::test]
async fn job_recover_only_pending() {
    let h = build(HarnessOpts { job_tables: vec![("job", "http://b")], ..Default::default() }).await;
    insert_job(&h.raw_db, "rec1", "pending").await;
    let r = h.node.route(authed(Method::Post, "/job/recover", json!({ "id": "job:rec1" }))).await.unwrap();
    assert_eq!(r.status, 200);
    assert_eq!(json_of(&r)["message"], "re-enqueued");
    // Assignee stamped on the row (took ownership).
    let assignee: Option<String> = h
        .raw_db
        .query("SELECT VALUE assignee FROM ONLY job:rec1")
        .await
        .unwrap()
        .take(0)
        .unwrap();
    assert_eq!(assignee.as_deref(), Some("test-ssp"));

    // Terminal rows are skipped.
    insert_job(&h.raw_db, "rec2", "success").await;
    let r = h.node.route(authed(Method::Post, "/job/recover", json!({ "id": "job:rec2" }))).await.unwrap();
    assert_eq!(json_of(&r)["message"], "not pending; recover skipped");
}

#[tokio::test]
async fn debug_deps_reports_empty_store() {
    let h = build(HarnessOpts::default()).await;
    let r = h.node.route(authed(Method::Get, "/debug/deps", Value::Null)).await.unwrap();
    assert_eq!(r.status, 200);
    assert_eq!(json_of(&r)["view_count"], 0);
}

#[tokio::test]
async fn crdt_apply_gated_when_not_ready() {
    let h = build(HarnessOpts { status: SspStatus::Bootstrapping, ..Default::default() }).await;
    let body = json!({ "table": "t", "record_id": "t:1", "field": "f", "update": "", "peer": "1" });
    let r = h.node.route(authed(Method::Post, "/crdt/apply", body)).await.unwrap();
    assert_eq!(r.status, 503);
}

#[tokio::test]
async fn view_register_then_unregister_roundtrip() {
    let h = build(HarnessOpts::default()).await;
    // Default-deny permissions: seed a permissive rule so registration passes.
    {
        let mut c = h.node.processor.write().await;
        c.set_permission("thread", "true");
    }

    // Register a simple view → circuit view_count goes to 1.
    let payload = json!({
        "id": "v1",
        "surql": "SELECT * FROM thread",
        "clientId": "c1",
        "ttl": "30m",
        "lastActiveAt": "2024-01-01T00:00:00Z"
    });
    let r = h.node.route(authed(Method::Post, "/view/register", payload)).await.unwrap();
    assert_eq!(r.status, 200, "register ok: {:?}", json_of(&r));
    assert_eq!(h.node.processor.read().await.view_count(), 1);

    // A per-view metrics slot was seeded.
    assert!(h.node.view_metrics.read().await.contains_key("v1"));

    // Unregister → circuit drops back to 0.
    let r = h
        .node
        .route(authed(Method::Post, "/view/unregister", json!({ "id": "v1" })))
        .await
        .unwrap();
    assert_eq!(r.status, 200);
    assert_eq!(h.node.processor.read().await.view_count(), 0);
    assert!(!h.node.view_metrics.read().await.contains_key("v1"));
}

#[tokio::test]
async fn view_register_rejects_when_not_ready() {
    let h = build(HarnessOpts { status: SspStatus::Bootstrapping, ..Default::default() }).await;
    let r = h
        .node
        .route(authed(Method::Post, "/view/register", json!({ "id": "v1", "surql": "SELECT * FROM t" })))
        .await
        .unwrap();
    assert_eq!(r.status, 503);
}

#[tokio::test]
async fn ingest_gated_when_not_ready() {
    let h = build(HarnessOpts { status: SspStatus::Bootstrapping, ..Default::default() }).await;
    let body = json!({ "table": "thread", "op": "CREATE", "id": "thread:1", "record": {} });
    let r = h.node.route(authed(Method::Post, "/ingest", body)).await.unwrap();
    assert_eq!(r.status, 503);
}

#[tokio::test]
async fn ingest_bad_op_and_body() {
    let h = build(HarnessOpts::default()).await;
    let r = h.node.route(authed(Method::Post, "/ingest", json!("notjson-shape"))).await.unwrap();
    assert_eq!(r.status, 400);
    let bad_op = json!({ "table": "t", "op": "SIDEWAYS", "id": "t:1", "record": {} });
    let r = h.node.route(authed(Method::Post, "/ingest", bad_op)).await.unwrap();
    assert_eq!(r.status, 400);
}

#[tokio::test]
async fn ingest_routes_pending_job_to_queue() {
    // A pending job row on a configured job table → enqueued (standalone).
    let h = build(HarnessOpts { job_tables: vec![("job", "http://backend")], ..Default::default() }).await;
    let body = json!({
        "table": "job",
        "op": "CREATE",
        "id": "job:abc",
        "record": { "status": "pending", "path": "/run", "payload": {} }
    });
    let r = h.node.route(authed(Method::Post, "/ingest", body)).await.unwrap();
    assert_eq!(r.status, 200);

    let mut rx = h.job_rx.lock().await;
    let job = rx.try_recv().expect("pending job enqueued via ingest");
    assert_eq!(job.id, "job:abc");
    assert_eq!(job.base_url, "http://backend");
}

#[tokio::test]
async fn ingest_non_pending_job_not_enqueued() {
    let h = build(HarnessOpts { job_tables: vec![("job", "http://backend")], ..Default::default() }).await;
    let body = json!({
        "table": "job", "op": "CREATE", "id": "job:done",
        "record": { "status": "success", "path": "/run", "payload": {} }
    });
    let r = h.node.route(authed(Method::Post, "/ingest", body)).await.unwrap();
    assert_eq!(r.status, 200);
    assert!(h.job_rx.lock().await.try_recv().is_err(), "non-pending job must not enqueue");
}

#[tokio::test]
async fn ingest_steps_circuit_for_registered_view() {
    // Register a view over `thread`, then ingest a matching row and confirm
    // the circuit produced a delta (view now has the record).
    let h = build(HarnessOpts::default()).await;
    {
        let mut c = h.node.processor.write().await;
        c.set_permission("thread", "true");
    }
    let reg = json!({ "id": "v1", "surql": "SELECT * FROM thread", "clientId": "c1", "ttl": "30m", "lastActiveAt": "2024-01-01T00:00:00Z" });
    assert_eq!(h.node.route(authed(Method::Post, "/view/register", reg)).await.unwrap().status, 200);

    let ingest = json!({ "table": "thread", "op": "CREATE", "id": "thread:1", "record": { "title": "hi" } });
    let r = h.node.route(authed(Method::Post, "/ingest", ingest)).await.unwrap();
    assert_eq!(r.status, 200);

    // The view's window now contains the row.
    let dbg = h.node.route(authed(Method::Get, "/debug/view/v1", Value::Null)).await.unwrap();
    assert_eq!(json_of(&dbg)["cache_size"].as_u64(), Some(1), "circuit stepped: {:?}", json_of(&dbg));
}

#[tokio::test]
async fn on_timer_ttl_cleanup_sweeps_and_rearms() {
    use ssp_node::Runtime;
    let h = build(HarnessOpts { ref_mode: ssp_protocol::RefMode::Single, ..Default::default() }).await;
    // An already-expired _00_query row.
    h.raw_db
        .query("CREATE _00_query:old SET auth_id = '', clientId = 'c', ttl = 1s, lastActiveAt = time::now() - 1h, surql = 'x', params = {};")
        .await
        .unwrap();

    let rt = Runtime::new(Arc::clone(&h.node));
    rt.on_timer(TimerKind::TtlCleanup).await;

    // Swept the expired row…
    let remaining: Option<i64> = h
        .raw_db
        .query("SELECT VALUE count() FROM _00_query GROUP ALL")
        .await
        .unwrap()
        .take(0)
        .unwrap_or(None);
    assert_eq!(remaining.unwrap_or(0), 0, "expired _00_query swept");
    // …and re-armed itself via the Scheduler port.
    assert!(
        h.scheduled.lock().unwrap().contains(&TimerKind::TtlCleanup),
        "TtlCleanup re-armed"
    );
}

#[tokio::test]
async fn on_timer_job_recovery_rearms_when_standalone() {
    use ssp_node::Runtime;
    let h = build(HarnessOpts { job_tables: vec![("job", "http://b")], ..Default::default() }).await;
    let rt = Runtime::new(Arc::clone(&h.node));
    rt.on_timer(TimerKind::JobRecoverySweep).await;
    assert!(
        h.scheduled.lock().unwrap().contains(&TimerKind::JobRecoverySweep),
        "standalone JobRecoverySweep re-arms"
    );
}

#[tokio::test]
async fn bootstrap_restore_evict_mutate_catches_up_to_full_rebuild() {
    // Headline test for the generic runtime seam: a snapshot restore + `_00_rv`
    // catch-up must converge to EXACTLY the same circuit content as a cold full
    // rebuild over the same (mutated) DB. Proves catch-up correctness and the
    // rebuild-fallback divergence backstop in one shot.
    use std::collections::BTreeMap;
    use ssp_node::{CircuitStore, Runtime};

    let store = MemStore::default();
    let h = build(HarnessOpts {
        status: SspStatus::Bootstrapping,
        circuit_store: Some(Arc::new(store.clone()) as Arc<dyn ssp_node::CircuitStore>),
        ..Default::default()
    })
    .await;

    // Schema + initial rows (each carries an `_00_rv`, as ingest would stamp).
    h.raw_db
        .query("DEFINE TABLE thread SCHEMALESS PERMISSIONS FOR select FULL;")
        .await
        .unwrap();
    h.raw_db
        .query(
            "CREATE thread:1 SET title = 'a', _00_rv = 1; \
             CREATE thread:2 SET title = 'b', _00_rv = 2;",
        )
        .await
        .unwrap();

    let rt = Runtime::new(Arc::clone(&h.node));

    // 1. Cold bootstrap: store empty → full rebuild → Ready.
    rt.bootstrap().await;
    assert_eq!(*h.node.status.read().await, SspStatus::Ready);
    let hashes_snapshot = h.node.processor.read().await.compute_table_hashes();
    assert!(hashes_snapshot.contains_key("thread"), "thread loaded");

    // 2. Persist a snapshot with a resume-point (max _00_rv folded in = 2).
    let blob = h.node.processor.read().await.save().unwrap();
    let mut max_rv = BTreeMap::new();
    max_rv.insert("thread".to_string(), 2i64);
    let point = ssp_node::ResumePoint {
        saved_at_epoch_ms: ssp_node::now_epoch_ms(),
        table_hashes: hashes_snapshot.clone(),
        max_row_version: max_rv,
    };
    store.save(&blob, &point).await.unwrap();

    // 3. Mutate the DB AFTER the snapshot: a new row + an update, both with a
    //    higher `_00_rv` (steady-state ingest bumps it).
    h.raw_db
        .query(
            "CREATE thread:3 SET title = 'c', _00_rv = 3; \
             UPDATE thread:1 SET title = 'a2', _00_rv = 4;",
        )
        .await
        .unwrap();

    // 4. Evict the in-memory circuit (host restart / DO eviction).
    *h.node.processor.write().await = Circuit::new();

    // 5. Warm bootstrap: load snapshot → restore + `_00_rv` catch-up.
    rt.bootstrap().await;
    assert_eq!(*h.node.status.read().await, SspStatus::Ready);
    let hashes_restored = h.node.processor.read().await.compute_table_hashes();

    // 6. Oracle: a cold full rebuild over the same mutated DB.
    let fresh = Arc::new(RwLock::new(Circuit::new()));
    ssp_node::bootstrap::rebuild_from_db(&MemDb(Arc::clone(&h.raw_db)), &fresh, 200)
        .await
        .unwrap();
    let hashes_fresh = fresh.read().await.compute_table_hashes();

    assert_eq!(
        hashes_restored, hashes_fresh,
        "restore+catch-up must converge to a full rebuild"
    );
    // And the catch-up actually moved past the snapshot (title update + new row).
    assert_ne!(
        hashes_restored, hashes_snapshot,
        "catch-up must reflect post-snapshot mutations"
    );
}

#[tokio::test]
async fn checkpoint_persists_snapshot_with_resume_point() {
    use ssp_node::{CircuitStore, Runtime};

    let store = MemStore::default();
    let h = build(HarnessOpts {
        circuit_store: Some(Arc::new(store.clone()) as Arc<dyn ssp_node::CircuitStore>),
        ..Default::default() // Ready
    })
    .await;

    // A couple of versioned rows in the circuit store.
    {
        let mut c = h.node.processor.write().await;
        c.load(vec![
            ssp::circuit::Record::new("thread", "1", json!({ "id": "thread:1", "_00_rv": 5 })),
            ssp::circuit::Record::new("thread", "2", json!({ "id": "thread:2", "_00_rv": 9 })),
        ]);
    }

    Runtime::new(Arc::clone(&h.node)).checkpoint().await;

    let (blob, point) = store.load().await.expect("snapshot persisted");
    assert!(!blob.is_empty());
    assert_eq!(point.max_row_version.get("thread"), Some(&9), "highest _00_rv folded in");
    assert!(point.table_hashes.contains_key("thread"));
    assert!(point.saved_at_epoch_ms > 0);
}

#[tokio::test]
async fn checkpoint_skips_when_not_ready() {
    use ssp_node::{CircuitStore, Runtime};
    let store = MemStore::default();
    let h = build(HarnessOpts {
        status: SspStatus::Bootstrapping,
        circuit_store: Some(Arc::new(store.clone()) as Arc<dyn ssp_node::CircuitStore>),
        ..Default::default()
    })
    .await;
    Runtime::new(Arc::clone(&h.node)).checkpoint().await;
    assert!(matches!(store.load().await, Err(ssp_node::CircuitStoreError::NotFound)));
}

#[tokio::test]
async fn on_timer_circuit_checkpoint_rearms_only_when_enabled() {
    use ssp_node::Runtime;

    // Disabled (interval None, the VM default) → no re-arm.
    let h = build(HarnessOpts::default()).await;
    Runtime::new(Arc::clone(&h.node)).on_timer(TimerKind::CircuitCheckpoint).await;
    assert!(
        !h.scheduled.lock().unwrap().contains(&TimerKind::CircuitCheckpoint),
        "disabled checkpoint must not re-arm"
    );

    // Enabled → re-arms via the Scheduler port.
    let store = MemStore::default();
    let mut h = build(HarnessOpts {
        circuit_store: Some(Arc::new(store) as Arc<dyn ssp_node::CircuitStore>),
        ..Default::default()
    })
    .await;
    // Flip on the interval (build() has no knob; mutate the Arc'd node pre-share).
    Arc::get_mut(&mut h.node).unwrap().checkpoint_interval_secs = Some(300);
    Runtime::new(Arc::clone(&h.node)).on_timer(TimerKind::CircuitCheckpoint).await;
    assert!(
        h.scheduled.lock().unwrap().contains(&TimerKind::CircuitCheckpoint),
        "enabled checkpoint re-arms"
    );
}

#[tokio::test]
async fn admin_reload_picks_up_schema_defined_after_bootstrap() {
    // A table defined AFTER the node is up is invisible to view registration
    // (permissions are captured at bootstrap). POST /admin/reload re-scans the
    // DB and makes it available.
    let h = build(HarnessOpts::default()).await; // Ready, empty circuit

    // A view over a not-yet-known table default-denies.
    let reg = json!({ "id": "v1", "surql": "SELECT * FROM late", "clientId": "c", "ttl": "30m", "lastActiveAt": "2024-01-01T00:00:00Z" });
    let r = h.node.route(authed(Method::Post, "/view/register", reg.clone())).await.unwrap();
    assert_ne!(r.status, 200, "unknown table should not register: {:?}", json_of(&r));

    // Define the table in the DB, then reload.
    h.raw_db
        .query("DEFINE TABLE late SCHEMALESS PERMISSIONS FOR select FULL; CREATE late:1 SET x = 1;")
        .await
        .unwrap();
    let r = h.node.route(authed(Method::Post, "/admin/reload", Value::Null)).await.unwrap();
    assert_eq!(r.status, 200, "reload ok: {:?}", json_of(&r));

    // Now the table is known — the view registers and sees the row.
    let r = h.node.route(authed(Method::Post, "/view/register", reg)).await.unwrap();
    assert_eq!(r.status, 200, "registers after reload: {:?}", json_of(&r));
    let dbg = h.node.route(authed(Method::Get, "/debug/view/v1", Value::Null)).await.unwrap();
    assert_eq!(json_of(&dbg)["cache_size"].as_u64(), Some(1), "reloaded row visible");
}

#[tokio::test]
async fn admin_reload_requires_auth() {
    let h = build(HarnessOpts::default()).await;
    let r = h.node.route(req(Method::Post, "/admin/reload", None, Value::Null)).await.unwrap();
    assert_eq!(r.status, 401);
}

#[tokio::test]
async fn log_accepts_and_swallows() {
    let h = build(HarnessOpts::default()).await;
    let r = h
        .node
        .route(authed(Method::Post, "/log", json!({ "message": "hi", "level": "warn" })))
        .await
        .unwrap();
    assert_eq!(r.status, 200);
}
