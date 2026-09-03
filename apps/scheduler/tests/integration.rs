use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tempfile::TempDir;
use tokio::sync::RwLock;
use tower::ServiceExt;

use scheduler::config::{DbConfig, LoadBalanceStrategy, SchedulerConfig};
use scheduler::ingest::{self, IngestState};
use scheduler::job_scheduler::{self, JobState, JobTracker};
use scheduler::messages::BufferedEvent;
use scheduler::metrics::{self, MetricsState};
use scheduler::proxy::{self, ProxyState};
use scheduler::query::{self, QueryState, QueryTracker};
use scheduler::replica::Replica;
use scheduler::router::SspPool;
use scheduler::ssp_management::{self, SspManagementState};
use scheduler::transport::{HttpTransport, SspInfo};
use scheduler::wal::EventWal;
use scheduler::SchedulerStatus;

// ---------------------------------------------------------------------------
// Test Harness
// ---------------------------------------------------------------------------

struct TestHarness {
    replica: Arc<RwLock<Replica>>,
    ssp_pool: Arc<RwLock<SspPool>>,
    status: Arc<RwLock<SchedulerStatus>>,
    event_buffer: Arc<RwLock<VecDeque<BufferedEvent>>>,
    seq_counter: Arc<AtomicU64>,
    wal: Arc<RwLock<EventWal>>,
    drain_lock: Arc<tokio::sync::Mutex<()>>,
    reclone_lock: Arc<tokio::sync::Mutex<()>>,
    transport: Arc<HttpTransport>,
    query_tracker: Arc<QueryTracker>,
    job_tracker: Arc<JobTracker>,
    config: Arc<SchedulerConfig>,
    snapshot_seq_cell: Arc<AtomicU64>,
    _replica_dir: TempDir,
    _wal_dir: TempDir,
}

impl TestHarness {
    async fn new() -> Self {
        Self::with_options(SchedulerStatus::Ready, 10_000).await
    }

    async fn with_status(status: SchedulerStatus) -> Self {
        Self::with_options(status, 10_000).await
    }

    #[allow(dead_code)]
    async fn with_max_buffer(max_buffer: usize) -> Self {
        Self::with_options(SchedulerStatus::Ready, max_buffer).await
    }

    async fn with_options(status: SchedulerStatus, max_buffer: usize) -> Self {
        let replica_dir = TempDir::new().expect("Failed to create temp dir for replica");
        let wal_dir = TempDir::new().expect("Failed to create temp dir for WAL");

        let replica_path = replica_dir.path().join("replica_db");
        let wal_path = wal_dir.path().join("event_wal.log");

        let replica = Replica::new(replica_path.clone())
            .await
            .expect("Failed to create replica");

        let wal = EventWal::new(wal_path.clone()).expect("Failed to create WAL");

        let config = SchedulerConfig {
            db: DbConfig {
                url: "http://localhost:8000".to_string(),
                namespace: "sp00ky".to_string(),
                database: "sp00ky".to_string(),
                username: "root".to_string(),
                password: "root".to_string(),
            },
            load_balance: LoadBalanceStrategy::LeastQueries,
            heartbeat_interval_ms: 1000,
            heartbeat_timeout_ms: 5000,
            bootstrap_chunk_size: 100,
            job_tables: vec![],
            replica_db_path: replica_path,
            ingest_host: None,
            ingest_port: 0,
            snapshot_update_interval_secs: 300,
            max_buffer_per_ssp: max_buffer,
            bootstrap_timeout_secs: 5,
            ssp_poll_interval_ms: 100,
            wal_path,
            scheduler_id: "test-scheduler".to_string(),
            ..SchedulerConfig::default()
        };

        let snapshot_seq_cell = replica.snapshot_seq_cell();
        Self {
            snapshot_seq_cell,
            replica: Arc::new(RwLock::new(replica)),
            ssp_pool: Arc::new(RwLock::new(SspPool::new(
                LoadBalanceStrategy::LeastQueries,
                max_buffer,
            ))),
            status: Arc::new(RwLock::new(status)),
            event_buffer: Arc::new(RwLock::new(VecDeque::new())),
            seq_counter: Arc::new(AtomicU64::new(0)),
            wal: Arc::new(RwLock::new(wal)),
            drain_lock: Arc::new(tokio::sync::Mutex::new(())),
            reclone_lock: Arc::new(tokio::sync::Mutex::new(())),
            transport: Arc::new(HttpTransport::new()),
            query_tracker: Arc::new(QueryTracker::new()),
            job_tracker: Arc::new(JobTracker::new()),
            config: Arc::new(config),
            _replica_dir: replica_dir,
            _wal_dir: wal_dir,
        }
    }

    fn ingest_router(&self) -> Router {
        let state = IngestState {
            replica: Arc::clone(&self.replica),
            transport: Arc::clone(&self.transport),
            ssp_pool: Arc::clone(&self.ssp_pool),
            status: Arc::clone(&self.status),
            event_buffer: Arc::clone(&self.event_buffer),
            seq_counter: Arc::clone(&self.seq_counter),
            wal: Arc::clone(&self.wal),
            drain_lock: Arc::clone(&self.drain_lock),
            db_config: Arc::new(self.config.db.clone()),
            job_tables: Arc::new(vec![]),
            observer_permits: Arc::new(tokio::sync::Semaphore::new(8)),
            snapshot_seq: Arc::clone(&self.snapshot_seq_cell),
        };
        ingest::create_ingest_router(state)
    }

    fn ssp_router(&self) -> Router {
        let state = SspManagementState {
            ssp_pool: Arc::clone(&self.ssp_pool),
            replica: Arc::clone(&self.replica),
            transport: Arc::clone(&self.transport),
            config: Arc::clone(&self.config),
            status: Arc::clone(&self.status),
            event_buffer: Arc::clone(&self.event_buffer),
            seq_counter: Arc::clone(&self.seq_counter),
            reclone_lock: Arc::clone(&self.reclone_lock),
            wal: Arc::clone(&self.wal),
            drain_lock: Arc::clone(&self.drain_lock),
        };
        ssp_management::create_ssp_router(state)
    }

    fn proxy_router(&self) -> Router {
        let state = ProxyState {
            replica: Arc::clone(&self.replica),
            status: Arc::clone(&self.status),
        };
        proxy::create_proxy_router(state)
    }

    fn query_router(&self) -> Router {
        let state = QueryState {
            ssp_pool: Arc::clone(&self.ssp_pool),
            transport: Arc::clone(&self.transport),
            query_tracker: Arc::clone(&self.query_tracker),
        };
        query::create_query_router(state)
    }

    fn job_router(&self) -> Router {
        let state = JobState {
            ssp_pool: Arc::clone(&self.ssp_pool),
            transport: Arc::clone(&self.transport),
            job_tracker: Arc::clone(&self.job_tracker),
        };
        job_scheduler::create_job_router(state)
    }

    fn metrics_router(&self) -> Router {
        metrics::create_metrics_router(self.metrics_state())
    }

    fn metrics_state(&self) -> MetricsState {
        MetricsState {
            ssp_pool: Arc::clone(&self.ssp_pool),
            query_tracker: Arc::clone(&self.query_tracker),
            job_tracker: Arc::clone(&self.job_tracker),
            start_time: std::time::Instant::now(),
            scheduler_id: "test-scheduler".to_string(),
            status: Arc::clone(&self.status),
            backend_health: scheduler::backend_health::create_health_cache(&[]),
            shared_backend_configs: scheduler::backend_health::create_shared_configs(&[]),
            ingest: IngestState {
                replica: Arc::clone(&self.replica),
                transport: Arc::clone(&self.transport),
                ssp_pool: Arc::clone(&self.ssp_pool),
                status: Arc::clone(&self.status),
                event_buffer: Arc::clone(&self.event_buffer),
                seq_counter: Arc::clone(&self.seq_counter),
                wal: Arc::clone(&self.wal),
                drain_lock: Arc::clone(&self.drain_lock),
                db_config: Arc::new(self.config.db.clone()),
                job_tables: Arc::new(vec![]),
                observer_permits: Arc::new(tokio::sync::Semaphore::new(8)),
                snapshot_seq: Arc::clone(&self.snapshot_seq_cell),
            },
            replica: Arc::clone(&self.replica),
            surrealdb_version: Arc::new(RwLock::new("unknown".to_string())),
            heartbeat: scheduler::heartbeat::HeartbeatStats::new(),
            heartbeat_config: scheduler::heartbeat::Config {
                interval_secs: 30,
                timeout_secs: 25,
                fail_threshold: 3,
                ping_url: None,
                webhook_url: None,
            },
            drift: Arc::new(RwLock::new(scheduler::drift::DriftState::default())),
            drift_config: scheduler::drift::DriftConfig::default(),
        }
    }

    fn full_app(&self) -> Router {
        Router::new()
            .merge(self.ingest_router())
            .merge(self.ssp_router())
            .merge(self.proxy_router())
            .merge(self.query_router())
            .merge(self.job_router())
            .merge(self.metrics_router())
    }

    async fn add_ready_ssp(&self, id: &str, url: &str) {
        let ssp_info = SspInfo {
            id: id.to_string(),
            url: url.to_string(),
            version: "test".to_string(),
            connected_at: std::time::Instant::now(),
            last_heartbeat: std::time::Instant::now(),
            query_count: 0,
            views: 0,
            cpu_usage: None,
            memory_usage: None,
            env: None,
            bootstrap: None,
        };
        let mut pool = self.ssp_pool.write().await;
        pool.upsert(ssp_info);
        pool.mark_bootstrapping(id);
        let _ = pool.mark_ready(id);
    }

    async fn add_bootstrapping_ssp(&self, id: &str, url: &str) {
        let ssp_info = SspInfo {
            id: id.to_string(),
            url: url.to_string(),
            version: "test".to_string(),
            connected_at: std::time::Instant::now(),
            last_heartbeat: std::time::Instant::now(),
            query_count: 0,
            views: 0,
            cpu_usage: None,
            memory_usage: None,
            env: None,
            bootstrap: None,
        };
        let mut pool = self.ssp_pool.write().await;
        pool.upsert(ssp_info);
        pool.mark_bootstrapping(id);
    }

    async fn set_status(&self, status: SchedulerStatus) {
        *self.status.write().await = status;
    }
}

// ---------------------------------------------------------------------------
// Request Helpers
// ---------------------------------------------------------------------------

async fn post_json(app: Router, path: &str, body: &Value) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json")
        .body(axum::body::Body::from(serde_json::to_vec(body).unwrap()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body_value = serde_json::from_slice(&body_bytes).unwrap_or(Value::Null);
    (status, body_value)
}

async fn get_json(app: Router, path: &str) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("GET")
        .uri(path)
        .body(axum::body::Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body_value = serde_json::from_slice(&body_bytes).unwrap_or(Value::Null);
    (status, body_value)
}

// ---------------------------------------------------------------------------
// Mock SSP Server
// ---------------------------------------------------------------------------

struct MockSsp {
    addr: String,
    received: Arc<tokio::sync::Mutex<Vec<Value>>>,
}

impl MockSsp {
    async fn start() -> Self {
        Self::start_with_health("ready").await
    }

    /// Like `start`, but `/health` reports the given status (e.g. `"failed"`
    /// to exercise the scheduler's bootstrap-failure path).
    async fn start_with_health(health_status: &'static str) -> Self {
        let received = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let received_clone = Arc::clone(&received);

        let app = {
            let received = Arc::clone(&received_clone);
            Router::new()
                .route(
                    "/ingest",
                    axum::routing::post({
                        let received = Arc::clone(&received);
                        move |axum::Json(body): axum::Json<Value>| {
                            let received = Arc::clone(&received);
                            async move {
                                received.lock().await.push(body);
                                StatusCode::OK
                            }
                        }
                    }),
                )
                .route(
                    "/view/register",
                    axum::routing::post({
                        let received = Arc::clone(&received);
                        move |axum::Json(body): axum::Json<Value>| {
                            let received = Arc::clone(&received);
                            async move {
                                received.lock().await.push(body);
                                StatusCode::OK
                            }
                        }
                    }),
                )
                .route(
                    "/view/unregister",
                    axum::routing::post({
                        let received = Arc::clone(&received);
                        move |axum::Json(body): axum::Json<Value>| {
                            let received = Arc::clone(&received);
                            async move {
                                received.lock().await.push(body);
                                StatusCode::OK
                            }
                        }
                    }),
                )
                .route(
                    "/job/dispatch",
                    axum::routing::post({
                        let received = Arc::clone(&received);
                        move |axum::Json(body): axum::Json<Value>| {
                            let received = Arc::clone(&received);
                            async move {
                                received.lock().await.push(body);
                                StatusCode::OK
                            }
                        }
                    }),
                )
                .route(
                    "/health",
                    axum::routing::get(move || async move {
                        axum::Json(json!({"status": health_status}))
                    }),
                )
        };

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("Failed to bind mock SSP");
        let addr = format!("http://{}", listener.local_addr().unwrap());

        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        // Brief yield to let the server start
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        MockSsp {
            addr,
            received: received_clone,
        }
    }

    async fn received_count(&self) -> usize {
        self.received.lock().await.len()
    }

    async fn received_bodies(&self) -> Vec<Value> {
        self.received.lock().await.clone()
    }
}

// ---------------------------------------------------------------------------
// Helper: make an ingest payload
// ---------------------------------------------------------------------------

fn ingest_payload(table: &str, op: &str, id: &str) -> Value {
    json!({
        "table": table,
        "op": op,
        "id": id,
        "record": {"name": "test"}
    })
}

// ===========================================================================
// Module 1: Ingest Tests
// ===========================================================================

mod ingest_tests {
    use super::*;

    #[tokio::test]
    async fn ingest_rejects_during_cloning() {
        let h = TestHarness::with_status(SchedulerStatus::Cloning).await;
        let app = h.ingest_router();

        let (status, _) = post_json(app, "/ingest", &ingest_payload("user", "CREATE", "u1")).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn ingest_succeeds_when_ready() {
        let h = TestHarness::new().await;
        let app = h.ingest_router();

        let (status, _) = post_json(app, "/ingest", &ingest_payload("user", "CREATE", "u1")).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn ingest_succeeds_when_snapshot_frozen() {
        let h = TestHarness::with_status(SchedulerStatus::SnapshotFrozen).await;
        let app = h.ingest_router();

        let (status, _) = post_json(app, "/ingest", &ingest_payload("user", "CREATE", "u1")).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn ingest_invalid_operation() {
        let h = TestHarness::new().await;
        let app = h.ingest_router();

        let (status, _) = post_json(app, "/ingest", &ingest_payload("user", "MERGE", "u1")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn ingest_case_insensitive_op() {
        let h = TestHarness::new().await;

        for op in &["create", "Create", "CREATE", "update", "Update", "delete", "Delete"] {
            let app = h.ingest_router();
            let (status, _) =
                post_json(app, "/ingest", &ingest_payload("user", op, "u1")).await;
            assert_eq!(status, StatusCode::OK, "op '{}' should succeed", op);
        }
    }

    #[tokio::test]
    async fn ingest_assigns_monotonic_seq() {
        let h = TestHarness::new().await;

        for i in 0..5 {
            let app = h.ingest_router();
            let (status, _) = post_json(
                app,
                "/ingest",
                &ingest_payload("user", "CREATE", &format!("u{}", i)),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
        }

        // seq_counter should be 5
        assert_eq!(h.seq_counter.load(Ordering::SeqCst), 5);

        // Buffer events should have seq 1..5
        let buffer = h.event_buffer.read().await;
        assert_eq!(buffer.len(), 5);
        for (i, event) in buffer.iter().enumerate() {
            assert_eq!(event.seq, (i + 1) as u64);
        }
    }

    #[tokio::test]
    async fn ingest_writes_to_wal() {
        let h = TestHarness::new().await;
        let app = h.ingest_router();

        let (status, _) = post_json(app, "/ingest", &ingest_payload("user", "CREATE", "u1")).await;
        assert_eq!(status, StatusCode::OK);

        // Verify WAL contains the event
        let wal = h.wal.read().await;
        let events = wal.recover().expect("WAL recovery failed");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].seq, 1);
        assert_eq!(events[0].update.table, "user");
    }

    #[tokio::test]
    async fn ingest_buffers_for_bootstrapping_ssp() {
        let h = TestHarness::new().await;
        h.add_bootstrapping_ssp("ssp-1", "http://localhost:9999").await;

        let app = h.ingest_router();
        let (status, _) = post_json(app, "/ingest", &ingest_payload("user", "CREATE", "u1")).await;
        assert_eq!(status, StatusCode::OK);

        // Check that the message was buffered for the bootstrapping SSP
        let pool = h.ssp_pool.read().await;
        assert!(pool.buffer_size("ssp-1") >= 1);
    }

    #[tokio::test]
    async fn ingest_broadcasts_to_ready_ssp() {
        let mock = MockSsp::start().await;
        let h = TestHarness::new().await;
        h.add_ready_ssp("ssp-1", &mock.addr).await;

        let app = h.ingest_router();
        let (status, _) = post_json(app, "/ingest", &ingest_payload("user", "CREATE", "u1")).await;
        assert_eq!(status, StatusCode::OK);

        // Give broadcast a moment to complete
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Mock should have received the ingest
        assert!(mock.received_count().await >= 1);
        let bodies = mock.received_bodies().await;
        assert_eq!(bodies[0]["table"], "user");
        assert_eq!(bodies[0]["op"], "CREATE");
    }

    #[tokio::test]
    async fn ingest_event_buffer_ordering() {
        let h = TestHarness::new().await;

        for i in 0..10 {
            let app = h.ingest_router();
            let (status, _) = post_json(
                app,
                "/ingest",
                &ingest_payload("user", "CREATE", &format!("u{}", i)),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
        }

        let buffer = h.event_buffer.read().await;
        assert_eq!(buffer.len(), 10);

        // Verify strictly ascending
        for i in 1..buffer.len() {
            assert!(
                buffer[i].seq > buffer[i - 1].seq,
                "seq {} should be > seq {}",
                buffer[i].seq,
                buffer[i - 1].seq
            );
        }
    }
}

// ===========================================================================
// Module 2: SSP Management Tests
// ===========================================================================

mod ssp_management_tests {
    use super::*;

    fn register_payload(ssp_id: &str, url: &str) -> Value {
        json!({
            "ssp_id": ssp_id,
            "url": url,
            "version": "test"
        })
    }

    fn heartbeat_payload(ssp_id: &str) -> Value {
        json!({
            "ssp_id": ssp_id,
            "timestamp": 1000,
            "views": 5,
            "cpu_usage": 45.0,
            "memory_usage": 60.0,
            "version": "test"
        })
    }

    #[tokio::test]
    async fn register_returns_202_with_snapshot_seq() {
        let h = TestHarness::new().await;
        let app = h.ssp_router();

        let (status, body) = post_json(
            app,
            "/ssp/register",
            &register_payload("ssp-1", "http://localhost:9999"),
        )
        .await;

        assert_eq!(status, StatusCode::ACCEPTED);
        assert!(body.get("snapshot_seq").is_some());
    }

    #[tokio::test]
    async fn register_freezes_snapshot() {
        let h = TestHarness::new().await;
        let app = h.ssp_router();

        let (status, _) = post_json(
            app,
            "/ssp/register",
            &register_payload("ssp-1", "http://localhost:9999"),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);

        let current = *h.status.read().await;
        assert_eq!(current, SchedulerStatus::SnapshotFrozen);
    }

    #[tokio::test]
    async fn register_marks_bootstrapping() {
        let h = TestHarness::new().await;
        let app = h.ssp_router();

        let (status, _) = post_json(
            app,
            "/ssp/register",
            &register_payload("ssp-1", "http://localhost:9999"),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);

        let pool = h.ssp_pool.read().await;
        assert!(pool.get("ssp-1").is_some(), "SSP should exist in pool");
        assert!(!pool.is_ready("ssp-1"), "SSP should NOT be ready yet");
    }

    #[tokio::test]
    async fn register_records_bootstrap_seq() {
        let h = TestHarness::new().await;
        let app = h.ssp_router();

        let (status, body) = post_json(
            app,
            "/ssp/register",
            &register_payload("ssp-1", "http://localhost:9999"),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);

        let expected_seq = body["snapshot_seq"].as_u64().unwrap();
        let pool = h.ssp_pool.read().await;
        assert_eq!(pool.get_bootstrap_seq("ssp-1"), Some(expected_seq));
    }

    #[tokio::test]
    async fn register_empty_ssp_id() {
        let h = TestHarness::new().await;

        // Empty ID
        let app = h.ssp_router();
        let (status, _) = post_json(
            app,
            "/ssp/register",
            &register_payload("", "http://localhost:9999"),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        // Whitespace-only ID
        let app = h.ssp_router();
        let (status, _) = post_json(
            app,
            "/ssp/register",
            &register_payload("   ", "http://localhost:9999"),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn register_invalid_url() {
        let h = TestHarness::new().await;

        for url in &["ftp://example.com", "ws://example.com", "example.com"] {
            let app = h.ssp_router();
            let (status, _) =
                post_json(app, "/ssp/register", &register_payload("ssp-1", url)).await;
            assert_eq!(
                status,
                StatusCode::BAD_REQUEST,
                "URL '{}' should be rejected",
                url
            );
        }
    }

    #[tokio::test]
    async fn register_during_cloning() {
        let h = TestHarness::with_status(SchedulerStatus::Cloning).await;
        let app = h.ssp_router();

        let (status, _) = post_json(
            app,
            "/ssp/register",
            &register_payload("ssp-1", "http://localhost:9999"),
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn heartbeat_unknown_ssp() {
        let h = TestHarness::new().await;
        let app = h.ssp_router();

        let (status, _) =
            post_json(app, "/ssp/heartbeat", &heartbeat_payload("nonexistent")).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn heartbeat_registered_ssp() {
        let h = TestHarness::new().await;
        h.add_ready_ssp("ssp-1", "http://localhost:9999").await;

        let app = h.ssp_router();
        let (status, _) = post_json(app, "/ssp/heartbeat", &heartbeat_payload("ssp-1")).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn heartbeat_updates_metrics() {
        let h = TestHarness::new().await;
        h.add_ready_ssp("ssp-1", "http://localhost:9999").await;

        let app = h.ssp_router();
        let (status, _) = post_json(app, "/ssp/heartbeat", &heartbeat_payload("ssp-1")).await;
        assert_eq!(status, StatusCode::OK);

        let pool = h.ssp_pool.read().await;
        let ssp = pool.get("ssp-1").unwrap();
        assert_eq!(ssp.views, 5);
        assert_eq!(ssp.cpu_usage, Some(45.0));
        assert_eq!(ssp.memory_usage, Some(60.0));
    }

    #[tokio::test]
    async fn multiple_registrations_keep_frozen() {
        let h = TestHarness::new().await;

        // Register first SSP
        let app = h.ssp_router();
        let (status, _) = post_json(
            app,
            "/ssp/register",
            &register_payload("ssp-1", "http://localhost:9991"),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);

        // Register second SSP
        let app = h.ssp_router();
        let (status, _) = post_json(
            app,
            "/ssp/register",
            &register_payload("ssp-2", "http://localhost:9992"),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);

        // Status should still be frozen
        let current = *h.status.read().await;
        assert_eq!(current, SchedulerStatus::SnapshotFrozen);

        // Both SSPs should be bootstrapping
        let pool = h.ssp_pool.read().await;
        assert!(pool.has_active_bootstrap());
    }
}

// ===========================================================================
// Module 3: Proxy Tests
// ===========================================================================

mod proxy_tests {
    use super::*;

    #[tokio::test]
    async fn proxy_signin_always_ok() {
        let h = TestHarness::new().await;
        let app = h.proxy_router();

        let (status, _) = post_json(app, "/proxy/signin", &json!({})).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn proxy_use_always_ok() {
        let h = TestHarness::new().await;
        let app = h.proxy_router();

        let (status, _) = post_json(app, "/proxy/use", &json!({})).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn proxy_query_empty_table() {
        let h = TestHarness::new().await;
        let app = h.proxy_router();

        let (status, body) = post_json(
            app,
            "/proxy/query",
            &json!({"query": "SELECT * FROM nonexistent"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.is_array());
        assert_eq!(body.as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn proxy_query_returns_results() {
        let h = TestHarness::new().await;

        // Use INFO FOR DB — always returns structured data from SurrealDB
        let app = h.proxy_router();
        let (status, body) = post_json(
            app,
            "/proxy/query",
            &json!({"query": "INFO FOR DB"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        // INFO FOR DB returns database metadata (response shape depends on
        // the SurrealDB version: array of results or a single object).
        assert!(
            body.is_array() || body.is_object(),
            "INFO FOR DB should return metadata, got: {body}"
        );
        assert!(!body.is_null(), "INFO FOR DB should return metadata");
    }

    #[tokio::test]
    async fn proxy_query_invalid_returns_error() {
        let h = TestHarness::new().await;

        let app = h.proxy_router();
        let (status, _) = post_json(
            app,
            "/proxy/query",
            &json!({"query": "THIS IS NOT VALID SURQL !!!"}),
        )
        .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    }
}

// ===========================================================================
// Module 4: Query Tests
// ===========================================================================

mod query_tests {
    use super::*;

    #[tokio::test]
    async fn query_register_no_ssps() {
        let h = TestHarness::new().await;
        let app = h.query_router();

        let (status, _) = post_json(
            app,
            "/view/register",
            &json!({
                "id": "q1",
                "surql": "SELECT * FROM user",
                "clientId": "c1"
            }),
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn query_register_transport_failure() {
        let h = TestHarness::new().await;
        // SSP at unreachable URL
        h.add_ready_ssp("ssp-1", "http://127.0.0.1:1").await;

        let app = h.query_router();
        let (status, _) = post_json(
            app,
            "/view/register",
            &json!({
                "id": "q1",
                "surql": "SELECT * FROM user",
                "clientId": "c1"
            }),
        )
        .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);

        // Tracker should be cleaned up
        assert!(h.query_tracker.get_assignment("q1").await.is_none());
    }

    #[tokio::test]
    async fn query_register_success() {
        let mock = MockSsp::start().await;
        let h = TestHarness::new().await;
        h.add_ready_ssp("ssp-1", &mock.addr).await;

        let app = h.query_router();
        let (status, body) = post_json(
            app,
            "/view/register",
            &json!({
                "id": "q1",
                "surql": "SELECT * FROM user",
                "clientId": "c1"
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["query_id"], "q1");
        assert_eq!(body["ssp_id"], "ssp-1");

        // Mock should have received the forward
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(mock.received_count().await >= 1);
    }

    #[tokio::test]
    async fn query_reregister_is_sticky_and_count_stable() {
        let mock_a = MockSsp::start().await;
        let mock_b = MockSsp::start().await;
        let h = TestHarness::new().await;
        h.add_ready_ssp("ssp-0", &mock_a.addr).await;
        h.add_ready_ssp("ssp-1", &mock_b.addr).await;

        let payload = json!({
            "id": "q1",
            "surql": "SELECT * FROM user",
            "clientId": "c1"
        });
        let (status, body) = post_json(h.query_router(), "/view/register", &payload).await;
        assert_eq!(status, StatusCode::OK);
        let first_ssp = body["ssp_id"].as_str().unwrap().to_string();

        // Clients re-issue register on reconnect/keepalive. The assignment
        // must stay sticky on the owning SSP instead of round-robining onto
        // the other one (the old behavior ping-ponged ssp-0 ↔ ssp-1 every
        // few seconds under a client keepalive loop)...
        for _ in 0..3 {
            let (status, body) = post_json(h.query_router(), "/view/register", &payload).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(body["ssp_id"].as_str().unwrap(), first_ssp);
        }

        // ...and query_count must not drift upward (the old path incremented
        // the newly selected SSP on every call without decrementing the
        // previous owner, permanently skewing least-queries balancing).
        let pool = h.ssp_pool.read().await;
        assert_eq!(pool.get(&first_ssp).unwrap().query_count, 1);
        let other = if first_ssp == "ssp-0" { "ssp-1" } else { "ssp-0" };
        assert_eq!(pool.get(other).unwrap().query_count, 0);
    }

    #[tokio::test]
    async fn query_unregister_not_found() {
        let h = TestHarness::new().await;
        let app = h.query_router();

        // Unregister is idempotent: an untracked query id (e.g. a stale
        // `_00_dbsp_cleanup` row after a scheduler restart) returns OK.
        let (status, _) = post_json(
            app,
            "/view/unregister",
            &json!({"id": "nonexistent"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }
}

// ===========================================================================
// Module 5: Metrics Tests
// ===========================================================================

mod metrics_tests {
    use super::*;

    #[tokio::test]
    async fn health_no_ssps() {
        let h = TestHarness::new().await;
        let app = h.metrics_router();

        let (status, _) = get_json(app, "/health").await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn health_with_ready_ssp() {
        let h = TestHarness::new().await;
        h.add_ready_ssp("ssp-1", "http://localhost:9999").await;

        let app = h.metrics_router();
        let (status, body) = get_json(app, "/health").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], "healthy");
    }

    #[tokio::test]
    async fn health_bootstrapping_only() {
        let h = TestHarness::new().await;
        h.add_bootstrapping_ssp("ssp-1", "http://localhost:9999")
            .await;

        let app = h.metrics_router();
        let (status, _) = get_json(app, "/health").await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn metrics_returns_correct_counts() {
        let h = TestHarness::new().await;
        h.add_ready_ssp("ssp-1", "http://localhost:9991").await;
        h.add_ready_ssp("ssp-2", "http://localhost:9992").await;
        h.add_bootstrapping_ssp("ssp-3", "http://localhost:9993")
            .await;

        let app = h.metrics_router();
        let (status, body) = get_json(app, "/metrics").await;
        assert_eq!(status, StatusCode::OK);

        let scheduler = &body["scheduler"];
        assert_eq!(scheduler["total_ssps"], 3);
        assert_eq!(scheduler["ready_ssps"], 2);
    }
}

// ===========================================================================
// Module 6: Job Tests
// ===========================================================================

mod job_tests {
    use super::*;

    #[tokio::test]
    async fn job_dispatch_no_ssps() {
        let h = TestHarness::new().await;
        let app = h.job_router();

        let (status, _) = post_json(
            app,
            "/job/dispatch",
            &json!({
                "job_id": "j1",
                "table": "user",
                "payload": {"action": "compute"}
            }),
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn job_dispatch_transport_failure() {
        let h = TestHarness::new().await;
        h.add_ready_ssp("ssp-1", "http://127.0.0.1:1").await;

        let app = h.job_router();
        let (status, _) = post_json(
            app,
            "/job/dispatch",
            &json!({
                "job_id": "j1",
                "table": "user",
                "payload": {"action": "compute"}
            }),
        )
        .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn job_result_unknown_job() {
        let h = TestHarness::new().await;
        let app = h.job_router();

        let (status, _) = post_json(
            app,
            "/job/result",
            &json!({
                "job_id": "nonexistent",
                "status": "completed",
                "result": null,
                "error": null
            }),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn job_result_completes_job() {
        let h = TestHarness::new().await;

        // Pre-assign job in tracker
        h.job_tracker
            .assign("j1".to_string(), "ssp-1".to_string())
            .await;

        let app = h.job_router();
        let (status, _) = post_json(
            app,
            "/job/result",
            &json!({
                "job_id": "j1",
                "status": "completed",
                "result": {"data": 42},
                "error": null
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        // Job should be removed from tracker
        assert!(h.job_tracker.get_assignment("j1").await.is_none());
    }
}

// ===========================================================================
// Module 7: Bootstrap Protocol Tests (cross-cutting)
// ===========================================================================

mod bootstrap_protocol_tests {
    use super::*;

    #[tokio::test]
    async fn ingest_then_register_flow() {
        let h = TestHarness::new().await;

        // Ingest 3 events
        for i in 0..3 {
            let app = h.ingest_router();
            let (status, _) = post_json(
                app,
                "/ingest",
                &ingest_payload("user", "CREATE", &format!("u{}", i)),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
        }
        assert_eq!(h.seq_counter.load(Ordering::SeqCst), 3);

        // Register SSP (this triggers SnapshotFrozen)
        let app = h.ssp_router();
        let (status, _) = post_json(
            app,
            "/ssp/register",
            &json!({"ssp_id": "ssp-1", "url": "http://localhost:9999", "version": "test"}),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);

        // Ingest 1 more event (should buffer for bootstrapping SSP)
        let app = h.ingest_router();
        let (status, _) =
            post_json(app, "/ingest", &ingest_payload("user", "CREATE", "u3")).await;
        assert_eq!(status, StatusCode::OK);

        // seq_counter should be 4
        assert_eq!(h.seq_counter.load(Ordering::SeqCst), 4);

        // The new event should be buffered for the bootstrapping SSP
        let pool = h.ssp_pool.read().await;
        assert!(pool.buffer_size("ssp-1") >= 1);
    }

    #[tokio::test]
    async fn status_state_machine() {
        let h = TestHarness::with_status(SchedulerStatus::Cloning).await;

        // Cloning → Ready
        h.set_status(SchedulerStatus::Ready).await;
        assert_eq!(*h.status.read().await, SchedulerStatus::Ready);

        // Ready → SnapshotFrozen (via SSP registration)
        let app = h.ssp_router();
        let (status, _) = post_json(
            app,
            "/ssp/register",
            &json!({"ssp_id": "ssp-1", "url": "http://localhost:9999", "version": "test"}),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);
        assert_eq!(*h.status.read().await, SchedulerStatus::SnapshotFrozen);
    }

    #[tokio::test]
    async fn concurrent_ingest_seq_uniqueness() {
        let h = TestHarness::new().await;

        // Launch 20 concurrent ingests
        let mut handles = Vec::new();
        for i in 0..20 {
            let ingest_state = IngestState {
                replica: Arc::clone(&h.replica),
                transport: Arc::clone(&h.transport),
                ssp_pool: Arc::clone(&h.ssp_pool),
                status: Arc::clone(&h.status),
                event_buffer: Arc::clone(&h.event_buffer),
                seq_counter: Arc::clone(&h.seq_counter),
                wal: Arc::clone(&h.wal),
                drain_lock: Arc::clone(&h.drain_lock),
                db_config: Arc::new(h.config.db.clone()),
                job_tables: Arc::new(vec![]),
                observer_permits: Arc::new(tokio::sync::Semaphore::new(8)),
                snapshot_seq: Arc::clone(&h.snapshot_seq_cell),
            };
            let app = ingest::create_ingest_router(ingest_state);

            handles.push(tokio::spawn(async move {
                let payload = ingest_payload("user", "CREATE", &format!("u{}", i));
                let (status, _) = post_json(app, "/ingest", &payload).await;
                assert_eq!(status, StatusCode::OK);
            }));
        }

        for handle in handles {
            handle.await.unwrap();
        }

        // All 20 seqs should be unique
        assert_eq!(h.seq_counter.load(Ordering::SeqCst), 20);

        let buffer = h.event_buffer.read().await;
        assert_eq!(buffer.len(), 20);

        let mut seqs: Vec<u64> = buffer.iter().map(|e| e.seq).collect();
        seqs.sort();
        seqs.dedup();
        assert_eq!(seqs.len(), 20, "All 20 sequence numbers should be unique");
    }

    #[tokio::test]
    async fn full_app_all_routes_reachable() {
        let h = TestHarness::new().await;
        h.add_ready_ssp("ssp-1", "http://localhost:9999").await;

        // Pre-populate tracker state so unregister/result endpoints
        // return handler errors (not route-level 404)
        h.query_tracker
            .assign("q-pre".to_string(), "ssp-1".to_string())
            .await;
        h.job_tracker
            .assign("j-pre".to_string(), "ssp-1".to_string())
            .await;

        // Verify no endpoint returns 404 (other status codes are expected)
        let endpoints: Vec<(&str, &str, Value)> = vec![
            ("POST", "/ingest", ingest_payload("user", "CREATE", "u1")),
            (
                "POST",
                "/ssp/register",
                json!({"ssp_id": "s1", "url": "http://localhost:1234", "version": "test"}),
            ),
            (
                "POST",
                "/ssp/heartbeat",
                json!({"ssp_id": "ssp-1", "timestamp": 0, "active_queries": 0}),
            ),
            ("POST", "/proxy/signin", json!({})),
            ("POST", "/proxy/use", json!({})),
            (
                "POST",
                "/proxy/query",
                json!({"query": "SELECT * FROM user"}),
            ),
            (
                "POST",
                "/view/register",
                json!({"id": "q1", "surql": "SELECT * FROM user", "clientId": "c1"}),
            ),
            (
                "POST",
                "/view/unregister",
                json!({"id": "q-pre"}),
            ),
            (
                "POST",
                "/job/dispatch",
                json!({"job_id": "j1", "table": "user", "payload": {}}),
            ),
            (
                "POST",
                "/job/result",
                json!({"job_id": "j-pre", "status": "completed"}),
            ),
        ];

        for (method, path, body) in endpoints {
            // Rebuild app for each request (oneshot consumes router)
            let app = h.full_app();
            let request = Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", "application/json")
                .body(axum::body::Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap();

            let response = app.oneshot(request).await.unwrap();
            assert_ne!(
                response.status(),
                StatusCode::NOT_FOUND,
                "{} {} should not return 404",
                method,
                path
            );
        }

        // GET endpoints
        for path in &["/health", "/metrics"] {
            let app = h.full_app();
            let request = Request::builder()
                .method("GET")
                .uri(*path)
                .body(axum::body::Body::empty())
                .unwrap();

            let response = app.oneshot(request).await.unwrap();
            assert_ne!(
                response.status(),
                StatusCode::NOT_FOUND,
                "GET {} should not return 404",
                path
            );
        }
    }

    #[tokio::test]
    async fn updater_tick_recovers_latched_frozen_and_drains() {
        // The live-incident latch: SnapshotFrozen with no active bootstrap
        // (failed bootstrap whose error handler never ran). The tick must
        // self-recover to Ready and drain the pinned backlog.
        let h = TestHarness::with_status(SchedulerStatus::SnapshotFrozen).await;

        for i in 0..3 {
            let app = h.ingest_router();
            let (status, _) = post_json(
                app,
                "/ingest",
                &ingest_payload("user", "CREATE", &format!("u{}", i)),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
        }
        assert_eq!(h.event_buffer.read().await.len(), 3);

        scheduler::snapshot_updater_tick(
            &h.status,
            &h.event_buffer,
            &h.replica,
            &h.ssp_pool,
            &h.wal,
            &h.drain_lock,
            std::time::Duration::from_secs(300),
            None,
        )
        .await;

        assert_eq!(*h.status.read().await, SchedulerStatus::Ready);
        assert!(h.event_buffer.read().await.is_empty(), "backlog drained");
        let rep = h.replica.read().await;
        assert_eq!(rep.snapshot_seq(), 3, "snapshot advanced");
        drop(rep);
        // WAL truncated up to the drained seq — a restart must not refill.
        assert!(h.wal.read().await.recover().unwrap().is_empty());
    }

    #[tokio::test]
    async fn updater_tick_recovers_latched_updating() {
        // A prior tick that died mid-drain pins SnapshotUpdating.
        let h = TestHarness::with_status(SchedulerStatus::SnapshotUpdating).await;
        scheduler::snapshot_updater_tick(
            &h.status,
            &h.event_buffer,
            &h.replica,
            &h.ssp_pool,
            &h.wal,
            &h.drain_lock,
            std::time::Duration::from_secs(300),
            None,
        )
        .await;
        assert_eq!(*h.status.read().await, SchedulerStatus::Ready);
    }

    #[tokio::test]
    async fn updater_tick_noop_with_fresh_active_bootstrap() {
        let h = TestHarness::with_status(SchedulerStatus::SnapshotFrozen).await;
        h.add_bootstrapping_ssp("ssp-1", "http://localhost:9999").await;

        let app = h.ingest_router();
        let (status, _) =
            post_json(app, "/ingest", &ingest_payload("user", "CREATE", "u1")).await;
        assert_eq!(status, StatusCode::OK);

        scheduler::snapshot_updater_tick(
            &h.status,
            &h.event_buffer,
            &h.replica,
            &h.ssp_pool,
            &h.wal,
            &h.drain_lock,
            std::time::Duration::from_secs(300),
            None,
        )
        .await;

        // Bootstrap in flight: no recovery, no drain.
        assert_eq!(*h.status.read().await, SchedulerStatus::SnapshotFrozen);
        assert_eq!(h.event_buffer.read().await.len(), 1);
    }

    #[tokio::test]
    async fn updater_tick_evicts_stale_bootstrap_then_recovers() {
        let h = TestHarness::with_status(SchedulerStatus::SnapshotFrozen).await;
        h.add_bootstrapping_ssp("ssp-parked", "http://localhost:9999").await;
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        scheduler::snapshot_updater_tick(
            &h.status,
            &h.event_buffer,
            &h.replica,
            &h.ssp_pool,
            &h.wal,
            &h.drain_lock,
            std::time::Duration::ZERO, // everything parked is instantly stale
            None,
        )
        .await;

        let pool = h.ssp_pool.read().await;
        assert!(pool.get("ssp-parked").is_none(), "parked SSP evicted");
        assert!(!pool.has_active_bootstrap());
        drop(pool);
        assert_eq!(*h.status.read().await, SchedulerStatus::Ready);
    }

    #[tokio::test]
    async fn register_drains_pending_events_and_hands_fresh_hashes() {
        let h = TestHarness::new().await;

        for i in 0..3 {
            let app = h.ingest_router();
            let (status, _) = post_json(
                app,
                "/ingest",
                &ingest_payload("user", "CREATE", &format!("u{}", i)),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
        }
        assert_eq!(h.event_buffer.read().await.len(), 3);

        let app = h.ssp_router();
        let (status, body) = post_json(
            app,
            "/ssp/register",
            &json!({"ssp_id": "ssp-1", "url": "http://localhost:9999", "version": "test"}),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);

        // The backlog was applied before hashes were captured...
        assert!(h.event_buffer.read().await.is_empty());
        let rep = h.replica.read().await;
        assert_eq!(rep.snapshot_seq(), 3);
        assert_eq!(body["snapshot_seq"].as_u64(), Some(3));

        // ...so the handed-out hash matches the replica content the SSP will
        // bootstrap from — the invariant whose violation crash-loops SSPs.
        let fresh = rep.compute_table_hashes().await.unwrap();
        assert_eq!(
            body["table_hashes"]["user"].as_str(),
            fresh.get("user").map(|s| s.as_str()),
            "registration hash must match replica content"
        );
    }

    #[tokio::test]
    async fn register_skips_drain_while_sibling_bootstraps() {
        let h = TestHarness::new().await;
        h.add_bootstrapping_ssp("ssp-0", "http://localhost:9990").await;

        for i in 0..3 {
            let app = h.ingest_router();
            let (status, _) = post_json(
                app,
                "/ingest",
                &ingest_payload("user", "CREATE", &format!("u{}", i)),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
        }

        let app = h.ssp_router();
        let (status, _) = post_json(
            app,
            "/ssp/register",
            &json!({"ssp_id": "ssp-1", "url": "http://localhost:9991", "version": "test"}),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);

        // Draining now would invalidate the hashes ssp-0 already holds.
        assert_eq!(h.event_buffer.read().await.len(), 3, "backlog must survive");
    }

    #[tokio::test]
    async fn register_bumps_registration_generation() {
        let h = TestHarness::new().await;
        for _ in 0..2 {
            let app = h.ssp_router();
            let (status, _) = post_json(
                app,
                "/ssp/register",
                &json!({"ssp_id": "ssp-1", "url": "http://localhost:9999", "version": "test"}),
            )
            .await;
            assert_eq!(status, StatusCode::ACCEPTED);
        }
        assert_eq!(h.ssp_pool.read().await.registration_gen("ssp-1"), 2);
    }

    #[tokio::test]
    async fn bootstrap_failure_unfreezes_snapshot() {
        let h = TestHarness::new().await;
        let mock = MockSsp::start_with_health("failed").await;

        let app = h.ssp_router();
        let (status, _) = post_json(
            app,
            "/ssp/register",
            &json!({"ssp_id": "ssp-fail", "url": mock.addr, "version": "test"}),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);
        assert_eq!(*h.status.read().await, SchedulerStatus::SnapshotFrozen);

        // Poll task (100ms interval) sees "failed", bails, and the error
        // handler must remove the SSP AND unfreeze — the old code left the
        // status latched forever here.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            let unfrozen = *h.status.read().await == SchedulerStatus::Ready;
            let removed = h.ssp_pool.read().await.get("ssp-fail").is_none();
            if unfrozen && removed {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "bootstrap failure did not unfreeze the snapshot (status={:?})",
                *h.status.read().await
            );
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }

    #[tokio::test]
    async fn admin_resync_rehash_repairs_hash_drift() {
        let h = TestHarness::new().await;

        // Baseline content + persisted hashes, then drift the content
        // WITHOUT rehashing — exactly the stale-metadata corruption.
        {
            let mut rep = h.replica.write().await;
            rep.apply(
                "user",
                scheduler::replica::RecordOp::Create,
                "user:u1",
                Some(json!({"name": "a"})),
            )
            .await
            .unwrap();
            rep.set_snapshot_state(1, None).await.unwrap();
            rep.apply(
                "user",
                scheduler::replica::RecordOp::Update,
                "user:u1",
                Some(json!({"name": "b"})),
            )
            .await
            .unwrap();
        }
        {
            let rep = h.replica.read().await;
            assert_ne!(
                rep.snapshot_hashes(),
                &rep.compute_table_hashes().await.unwrap(),
                "test setup: hashes must be stale"
            );
        }

        h.add_ready_ssp("ssp-1", "http://localhost:9999").await;
        let app = h.ssp_router();
        let (status, body) = post_json(app, "/admin/resync", &json!({"mode": "rehash"})).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["mode"], "rehash");
        assert_eq!(body["marked_for_resync"], 1);

        let rep = h.replica.read().await;
        assert_eq!(
            rep.snapshot_hashes(),
            &rep.compute_table_hashes().await.unwrap(),
            "hashes repaired from content"
        );
    }

    #[tokio::test]
    async fn admin_resync_reclone_conflicts_while_running() {
        let h = TestHarness::new().await;
        let _guard = h.reclone_lock.lock().await; // simulate in-flight re-clone

        let app = h.ssp_router();
        let (status, _) = post_json(app, "/admin/resync", &json!({"mode": "reclone"})).await;
        assert_eq!(status, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn admin_resync_rejects_bad_mode_and_cloning() {
        let h = TestHarness::new().await;
        let app = h.ssp_router();
        let (status, _) = post_json(app, "/admin/resync", &json!({"mode": "nuke"})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let h = TestHarness::with_status(SchedulerStatus::Cloning).await;
        let app = h.ssp_router();
        let (status, _) = post_json(app, "/admin/resync", &json!({"mode": "rehash"})).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn health_reports_latched_stall_as_degraded() {
        let h = TestHarness::with_status(SchedulerStatus::SnapshotFrozen).await;
        h.add_ready_ssp("ssp-1", "http://localhost:9999").await;

        let (status, body) = get_json(h.metrics_router(), "/health").await;
        // Degraded, NOT 503: an orchestrator restart provably doesn't fix a
        // latched freeze (the WAL refills), and the updater self-heals.
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], "degraded");
        assert_eq!(body["scheduler"]["status"], "frozen");
        assert_eq!(body["scheduler"]["stalled"], true);

        // A justified freeze (bootstrap in flight) is not a stall.
        h.add_bootstrapping_ssp("ssp-2", "http://localhost:9998").await;
        let (status, body) = get_json(h.metrics_router(), "/health").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["scheduler"]["stalled"], false);
    }

    #[tokio::test]
    async fn pre_backup_skips_drain_during_bootstrap() {
        use maintenance::MaintenanceHost;

        let h = TestHarness::with_status(SchedulerStatus::SnapshotFrozen).await;
        h.add_bootstrapping_ssp("ssp-1", "http://localhost:9999").await;

        let app = h.ingest_router();
        let (status, _) =
            post_json(app, "/ingest", &ingest_payload("user", "CREATE", "u1")).await;
        assert_eq!(status, StatusCode::OK);

        let host = scheduler::maintenance_host::SchedulerHost {
            ingest: IngestState {
                replica: Arc::clone(&h.replica),
                transport: Arc::clone(&h.transport),
                ssp_pool: Arc::clone(&h.ssp_pool),
                status: Arc::clone(&h.status),
                event_buffer: Arc::clone(&h.event_buffer),
                seq_counter: Arc::clone(&h.seq_counter),
                wal: Arc::clone(&h.wal),
                drain_lock: Arc::clone(&h.drain_lock),
                db_config: Arc::new(h.config.db.clone()),
                job_tables: Arc::new(vec![]),
                observer_permits: Arc::new(tokio::sync::Semaphore::new(8)),
                snapshot_seq: Arc::clone(&h.snapshot_seq_cell),
            },
        };
        let seq = host.pre_backup().await.unwrap();
        assert_eq!(seq, Some(0), "returns current seq without draining");
        assert_eq!(
            h.event_buffer.read().await.len(),
            1,
            "must not mutate the replica mid-bootstrap"
        );
    }

    #[tokio::test]
    async fn startup_integrity_check_repairs_stale_hashes() {
        let replica_dir = TempDir::new().unwrap();
        let wal_dir = TempDir::new().unwrap();
        let config = SchedulerConfig {
            replica_db_path: replica_dir.path().join("replica_db"),
            wal_path: wal_dir.path().join("event_wal.log"),
            scheduler_id: "test-scheduler".to_string(),
            ..SchedulerConfig::default()
        };
        let sched = scheduler::Scheduler::new(config, Arc::new(HttpTransport::new()))
            .await
            .unwrap();

        {
            let mut rep = sched.replica.write().await;
            rep.apply(
                "user",
                scheduler::replica::RecordOp::Create,
                "user:u1",
                Some(json!({"name": "a"})),
            )
            .await
            .unwrap();
            rep.set_snapshot_state(1, None).await.unwrap();
            rep.apply(
                "user",
                scheduler::replica::RecordOp::Update,
                "user:u1",
                Some(json!({"name": "b"})),
            )
            .await
            .unwrap();
        }

        sched.startup_integrity_check().await.unwrap();

        let rep = sched.replica.read().await;
        assert_eq!(
            rep.snapshot_hashes(),
            &rep.compute_table_hashes().await.unwrap(),
            "startup check must repair, not just log"
        );
    }

    #[tokio::test]
    async fn snapshot_update_skips_during_bootstrap() {
        let h = TestHarness::new().await;

        // Register an SSP to trigger bootstrap
        let app = h.ssp_router();
        let (status, _) = post_json(
            app,
            "/ssp/register",
            &json!({"ssp_id": "ssp-1", "url": "http://localhost:9999", "version": "test"}),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);

        // Verify has_active_bootstrap() is true
        let pool = h.ssp_pool.read().await;
        assert!(
            pool.has_active_bootstrap(),
            "Should have active bootstrap after registration"
        );

        // The snapshot updater would skip because of this flag
        // (we don't actually run the updater, just verify the condition)
        drop(pool);

        let current_status = *h.status.read().await;
        assert_eq!(
            current_status,
            SchedulerStatus::SnapshotFrozen,
            "Status should be frozen during bootstrap"
        );
    }

    /// Regression for the 2026-08-08 wedge: while a writer holds the replica
    /// write lock for a long time (drain/rehash/reclone), every probe endpoint
    /// and /ingest must still answer promptly. Before the fix, /health parked
    /// on `replica.read()` inside `pending_events_snapshot` (holding the
    /// backend_health read guard across the await), and one queued writer
    /// then convoyed every reader — total HTTP silence.
    #[tokio::test]
    async fn probes_answer_while_replica_write_lock_held() {
        let h = TestHarness::new().await;
        h.add_ready_ssp("ssp-1", "http://localhost:1").await;

        // Hold the replica write lock like a long drain would.
        let replica = Arc::clone(&h.replica);
        let (locked_tx, locked_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
        let holder = tokio::spawn(async move {
            let _guard = replica.write().await;
            let _ = locked_tx.send(());
            let _ = release_rx.await;
        });
        locked_rx.await.expect("lock holder started");

        let budget = std::time::Duration::from_secs(1);
        for path in ["/health", "/health/live", "/health/ready", "/metrics", "/info/text"] {
            let app = h.metrics_router();
            let res = tokio::time::timeout(budget, get_json(app, path)).await;
            assert!(
                res.is_ok(),
                "{path} must answer within {budget:?} while the replica write lock is held"
            );
        }

        // /ingest must also stay live (it never needs the replica lock).
        let app = h.ingest_router();
        let payload = ingest_payload("user", "CREATE", "wedge1");
        let res = tokio::time::timeout(budget, post_json(app, "/ingest", &payload)).await;
        let (status, _) = res.expect("/ingest must answer while replica write lock is held");
        assert_eq!(status, StatusCode::OK);

        let _ = release_tx.send(());
        let _ = holder.await;
    }

    /// The chunked drain (write guard released between chunks, hashing under
    /// a read guard) must land on exactly the same state as the old
    /// single-guard drain: every event applied, seq advanced, and the cached
    /// hashes equal to a fresh full recompute of the replica content.
    #[tokio::test]
    async fn chunked_drain_matches_full_recompute() {
        let h = TestHarness::new().await;

        // >1 chunk (chunk size 256) spread over three tables.
        for i in 0..300 {
            let table = match i % 3 {
                0 => "user",
                1 => "game",
                _ => "comment",
            };
            let app = h.ingest_router();
            let (status, _) = post_json(
                app,
                "/ingest",
                &ingest_payload(table, "CREATE", &format!("rec{i}")),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
        }
        assert_eq!(h.event_buffer.read().await.len(), 300);

        let applied = {
            let _drain = h.drain_lock.lock().await;
            scheduler::drain_and_apply(&h.event_buffer, &h.replica, &h.wal).await.unwrap()
        };
        assert_eq!(applied, 300);

        let rep = h.replica.read().await;
        assert_eq!(rep.snapshot_seq(), 300);
        assert_eq!(
            rep.snapshot_hashes(),
            &rep.compute_table_hashes().await.unwrap(),
            "chunked drain must leave cached hashes equal to content"
        );
    }
}


// ── Replica drift detection ───────────────────────────────────────────────
//
// The replica is fed by ingest events only; rows written upstream while nothing
// was listening never reach it, and every SSP that bootstraps from it inherits
// the gap while every hash-based check passes (an empty table hashes the same
// on both sides). The drift check compares row counts against upstream and
// re-clones. These tests drive it with a stubbed upstream and a recording
// recloner; the decision table itself is unit-tested in `drift.rs`.
mod drift_tests {
    use super::*;
    use scheduler::drift::{self, Action, DriftConfig, DriftHook, DriftState, Recloner, UpstreamCounts};
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct StubUpstream(BTreeMap<String, Option<u64>>);

    #[async_trait::async_trait]
    impl UpstreamCounts for StubUpstream {
        async fn upstream_counts(&self) -> anyhow::Result<BTreeMap<String, Option<u64>>> {
            Ok(self.0.clone())
        }
    }

    struct RecordingRecloner {
        calls: AtomicUsize,
        ssp_pool: Arc<RwLock<SspPool>>,
    }

    #[async_trait::async_trait]
    impl Recloner for RecordingRecloner {
        async fn reclone_and_resync(&self) -> anyhow::Result<bool> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.ssp_pool.write().await.mark_all_for_resync();
            Ok(true)
        }
    }

    fn hook(h: &TestHarness, upstream: &[(&str, u64)], cfg: DriftConfig) -> (Arc<DriftHook>, Arc<RecordingRecloner>) {
        let recloner = Arc::new(RecordingRecloner {
            calls: AtomicUsize::new(0),
            ssp_pool: Arc::clone(&h.ssp_pool),
        });
        let hook = Arc::new(DriftHook {
            cfg,
            upstream: Arc::new(StubUpstream(
                upstream.iter().map(|(t, n)| (t.to_string(), Some(*n))).collect(),
            )),
            state: Arc::new(RwLock::new(DriftState::default())),
            reclone: recloner.clone(),
        });
        (hook, recloner)
    }

    #[tokio::test]
    async fn an_empty_replica_table_upstream_has_rows_for_reclones_and_flags_ssps() {
        let h = TestHarness::new().await;
        h.add_ready_ssp("ssp-a", "http://localhost:9999").await;
        // Upstream has contacts; the replica has never seen the table.
        let (hook, recloner) = hook(&h, &[("contact", 5386)], DriftConfig::default());

        let action = drift::run_check(&hook, &h.replica).await;
        assert!(matches!(action, Action::Reclone { ref tables } if tables == &["contact".to_string()]), "{action:?}");
        assert_eq!(recloner.calls.load(Ordering::SeqCst), 1);
        assert!(h.ssp_pool.write().await.take_resync_flag("ssp-a"), "SSP flagged for re-bootstrap");

        let st = hook.state.read().await;
        assert_eq!(st.auto_reclones, 1);
        let json = drift::state_json(&st, &hook.cfg);
        assert_eq!(json["mismatched"], serde_json::json!(["contact"]));
        assert_eq!(json["tables"]["contact"]["upstream"], 5386);
        assert_eq!(json["tables"]["contact"]["replica"], 0);
    }

    #[tokio::test]
    async fn reporting_only_never_reclones_but_shows_up_in_health_snapshot() {
        let h = TestHarness::new().await;
        let cfg = DriftConfig { auto_reclone: false, ..DriftConfig::default() };
        let (hook, recloner) = hook(&h, &[("contact", 5)], cfg);

        let action = drift::run_check(&hook, &h.replica).await;
        assert!(matches!(action, Action::Report { .. }), "{action:?}");
        assert_eq!(recloner.calls.load(Ordering::SeqCst), 0);

        // The metrics router renders the same state under `/health/snapshot`.
        let state = MetricsState {
            ssp_pool: Arc::clone(&h.ssp_pool),
            query_tracker: Arc::clone(&h.query_tracker),
            job_tracker: Arc::clone(&h.job_tracker),
            start_time: std::time::Instant::now(),
            scheduler_id: "test-scheduler".to_string(),
            status: Arc::clone(&h.status),
            backend_health: scheduler::backend_health::create_health_cache(&[]),
            shared_backend_configs: scheduler::backend_health::create_shared_configs(&[]),
            ingest: IngestState {
                replica: Arc::clone(&h.replica),
                transport: Arc::clone(&h.transport),
                ssp_pool: Arc::clone(&h.ssp_pool),
                status: Arc::clone(&h.status),
                event_buffer: Arc::clone(&h.event_buffer),
                seq_counter: Arc::clone(&h.seq_counter),
                wal: Arc::clone(&h.wal),
                drain_lock: Arc::clone(&h.drain_lock),
                db_config: Arc::new(h.config.db.clone()),
                job_tables: Arc::new(vec![]),
                observer_permits: Arc::new(tokio::sync::Semaphore::new(8)),
                snapshot_seq: Arc::clone(&h.snapshot_seq_cell),
            },
            replica: Arc::clone(&h.replica),
            surrealdb_version: Arc::new(RwLock::new("unknown".to_string())),
            heartbeat: scheduler::heartbeat::HeartbeatStats::new(),
            heartbeat_config: scheduler::heartbeat::Config {
                interval_secs: 30,
                timeout_secs: 25,
                fail_threshold: 3,
                ping_url: None,
                webhook_url: None,
            },
            drift: Arc::clone(&hook.state),
            drift_config: hook.cfg.clone(),
        };
        let app = metrics::create_metrics_router(state);
        let (status, body) = get_json(app, "/health/snapshot").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["drift"]["auto_reclone_enabled"], false);
        assert_eq!(body["drift"]["mismatched"], serde_json::json!(["contact"]));
    }

    #[tokio::test]
    async fn matching_counts_are_clean_and_touch_nothing() {
        let h = TestHarness::new().await;
        // Nothing upstream, nothing in the replica: agreement.
        let (hook, recloner) = hook(&h, &[("contact", 0)], DriftConfig::default());
        assert_eq!(drift::run_check(&hook, &h.replica).await, Action::Clean);
        assert_eq!(recloner.calls.load(Ordering::SeqCst), 0);
        let st = hook.state.read().await;
        assert!(drift::state_json(&st, &hook.cfg)["mismatched"].as_array().unwrap().is_empty());
    }
}

// ===========================================================================
// Admin plane (`/admin/api/*`)
// ===========================================================================

mod admin_plane {
    use super::*;
    use axum::extract::ConnectInfo;
    use scheduler::admin::{self, AdminConfig};
    use std::net::SocketAddr;

    /// The admin router over the shared harness.
    ///
    /// `dir` deliberately points at a path that does not exist in most tests:
    /// a scheduler built from a checkout has no dashboard bundle, and that
    /// must be a working API with a placeholder page rather than a failure.
    fn admin_app(h: &TestHarness, password: Option<&str>) -> Router {
        let config = AdminConfig {
            enabled: true,
            host: "127.0.0.1".to_string(),
            port: 9668,
            dir: std::path::PathBuf::from("/nonexistent/dashboard"),
            password: password.map(str::to_string),
            access: None,
            session_ttl: std::time::Duration::from_secs(3600),
        };
        let (_state, router) = admin::build(
            config,
            h.metrics_state(),
            Arc::clone(&h.transport),
            maintenance::log_ring::LogRing::new(128),
            h.config.db.clone(),
            // Never populated: these tests cover the plane's behaviour when the
            // scheduler has no database handle yet, which is also its behaviour
            // during the initial replica clone.
            admin::new_db_slot(),
            300,
            15,
        );
        router
    }

    /// `oneshot` carries no connection info, but the login handler rate-limits
    /// per peer address, so tests have to supply one.
    fn with_peer(mut req: Request<axum::body::Body>, ip: &str) -> Request<axum::body::Body> {
        let addr: SocketAddr = format!("{ip}:40000").parse().unwrap();
        req.extensions_mut().insert(ConnectInfo(addr));
        req
    }

    fn get(path: &str) -> Request<axum::body::Body> {
        Request::builder()
            .uri(path)
            .body(axum::body::Body::empty())
            .unwrap()
    }

    fn get_auth(path: &str, token: &str) -> Request<axum::body::Body> {
        Request::builder()
            .uri(path)
            .header("Authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap()
    }

    fn login_req(body: Value, ip: &str) -> Request<axum::body::Body> {
        with_peer(
            Request::builder()
                .method("POST")
                .uri("/admin/api/session")
                .header("Content-Type", "application/json")
                .body(axum::body::Body::from(body.to_string()))
                .unwrap(),
            ip,
        )
    }

    async fn body_json(res: axum::response::Response) -> Value {
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    }

    /// Sign in with the break-glass password and return the bearer token.
    ///
    /// Takes the router rather than the harness: each `admin_app` call builds
    /// its own `SessionStore`, so a token is only valid against the router that
    /// minted it. Tests must therefore build one router and clone it.
    async fn breakglass_token(app: &Router, password: &str) -> String {
        let res = app
            .clone()
            .oneshot(login_req(json!({ "password": password }), "10.0.0.1"))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        body_json(res).await["token"].as_str().unwrap().to_string()
    }

    #[tokio::test]
    async fn config_is_reachable_without_a_token() {
        let h = TestHarness::new().await;
        let res = admin_app(&h, None)
            .oneshot(get("/admin/api/config"))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        let body = body_json(res).await;
        assert_eq!(body["scheduler_id"], "test-scheduler");
        assert!(body["version"].is_string());
        // This is what tells the frontend whether to offer the break-glass
        // toggle at all.
        assert_eq!(body["breakglass_available"], false);
    }

    #[tokio::test]
    async fn config_reports_breakglass_when_a_password_is_set() {
        let h = TestHarness::new().await;
        let res = admin_app(&h, Some("pw"))
            .oneshot(get("/admin/api/config"))
            .await
            .unwrap();
        assert_eq!(body_json(res).await["breakglass_available"], true);
    }

    #[tokio::test]
    async fn every_other_endpoint_requires_a_token() {
        let h = TestHarness::new().await;
        for path in [
            "/admin/api/me",
            "/admin/api/overview",
            "/admin/api/backends",
            "/admin/api/backends/api",
            "/admin/api/logs",
            "/admin/api/workflows/runs",
            "/admin/api/workflows/runs/x",
            "/admin/api/workflows/stream",
            "/admin/api/schedules",
        ] {
            let res = admin_app(&h, None).oneshot(get(path)).await.unwrap();
            assert_eq!(
                res.status(),
                StatusCode::UNAUTHORIZED,
                "{path} must be behind auth"
            );
        }
    }

    #[tokio::test]
    async fn a_malformed_or_unknown_token_is_rejected() {
        let h = TestHarness::new().await;
        for token in ["", "nonsense", "Bearer nested"] {
            let res = admin_app(&h, None)
                .oneshot(get_auth("/admin/api/overview", token))
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::UNAUTHORIZED, "token {token:?}");
        }
    }

    #[tokio::test]
    async fn breakglass_login_succeeds_and_the_token_works() {
        let h = TestHarness::new().await;
        let app = admin_app(&h, Some("hunter2"));
        let token = breakglass_token(&app, "hunter2").await;

        let res = app
            .oneshot(get_auth("/admin/api/me", &token))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        let me = body_json(res).await;
        assert_eq!(me["mode"], "breakglass");
        assert_eq!(me["subject"], "breakglass");
    }

    #[tokio::test]
    async fn breakglass_rejects_a_wrong_password() {
        let h = TestHarness::new().await;
        let res = admin_app(&h, Some("hunter2"))
            .oneshot(login_req(json!({ "password": "wrong" }), "10.0.0.2"))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn password_only_login_is_refused_when_no_password_is_configured() {
        let h = TestHarness::new().await;
        // Not merely "wrong password" — with SPKY_ADMIN_PASSWORD unset the
        // whole path must be closed, never fall back to some default.
        let res = admin_app(&h, None)
            .oneshot(login_req(json!({ "password": "" }), "10.0.0.3"))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn an_empty_password_never_matches_an_unset_one() {
        let h = TestHarness::new().await;
        let res = admin_app(&h, None)
            .oneshot(login_req(json!({ "username": "alice", "password": "" }), "10.0.0.4"))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn a_roster_login_without_a_database_fails_closed() {
        let h = TestHarness::new().await;
        // No database handle, so no access method can be discovered. This must
        // deny, not fall through to the break-glass branch.
        let res = admin_app(&h, Some("hunter2"))
            .oneshot(login_req(
                json!({ "username": "alice", "password": "hunter2" }),
                "10.0.0.5",
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn failures_do_not_disclose_why() {
        let h = TestHarness::new().await;

        let wrong_password = admin_app(&h, Some("hunter2"))
            .oneshot(login_req(json!({ "password": "nope" }), "10.0.0.6"))
            .await
            .unwrap();
        let unknown_user = admin_app(&h, Some("hunter2"))
            .oneshot(login_req(
                json!({ "username": "ghost", "password": "nope" }),
                "10.0.0.7",
            ))
            .await
            .unwrap();

        assert_eq!(wrong_password.status(), unknown_user.status());
        assert_eq!(
            body_json(wrong_password).await,
            body_json(unknown_user).await,
            "a login failure must not reveal whether the account exists"
        );
    }

    #[tokio::test]
    async fn login_is_rate_limited_per_address() {
        let h = TestHarness::new().await;
        // One router across the attempts, so they share a rate limiter.
        let app = admin_app(&h, Some("hunter2"));

        for i in 0..10 {
            let res = app
                .clone()
                .oneshot(login_req(json!({ "password": "wrong" }), "10.9.9.9"))
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::UNAUTHORIZED, "attempt {i}");
        }

        let blocked = app
            .clone()
            .oneshot(login_req(json!({ "password": "wrong" }), "10.9.9.9"))
            .await
            .unwrap();
        assert_eq!(blocked.status(), StatusCode::TOO_MANY_REQUESTS);

        // A different address is unaffected — the limiter must not become a
        // denial of service against every other operator.
        let other = app
            .oneshot(login_req(json!({ "password": "hunter2" }), "10.9.9.10"))
            .await
            .unwrap();
        assert_eq!(other.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn logout_revokes_the_token() {
        let h = TestHarness::new().await;
        let app = admin_app(&h, Some("hunter2"));

        let res = app
            .clone()
            .oneshot(login_req(json!({ "password": "hunter2" }), "10.0.0.8"))
            .await
            .unwrap();
        let token = body_json(res).await["token"].as_str().unwrap().to_string();

        let logout = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/admin/api/logout")
                    .header("Authorization", format!("Bearer {token}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(logout.status(), StatusCode::NO_CONTENT);

        let after = app
            .oneshot(get_auth("/admin/api/overview", &token))
            .await
            .unwrap();
        assert_eq!(after.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn overview_carries_the_whole_cluster() {
        let h = TestHarness::new().await;
        h.add_ready_ssp("ssp-1", "http://10.0.0.1:8667").await;
        let app = admin_app(&h, Some("pw"));
        let token = breakglass_token(&app, "pw").await;

        let res = app
            .oneshot(get_auth("/admin/api/overview", &token))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        let body = body_json(res).await;
        assert_eq!(body["scheduler"]["entity"], "scheduler");
        assert_eq!(body["ssps"].as_array().unwrap().len(), 1);
        assert_eq!(body["totals"]["ssps"], 1);
        assert_eq!(body["totals"]["ssps_ready"], 1);
        // The dashboard draws its deadline bar against this.
        assert_eq!(body["bootstrap_timeout_secs"], 300);
        assert!(body["server_time_ms"].as_u64().unwrap() > 0);
    }

    #[tokio::test]
    async fn overview_and_info_agree_about_ssp_state() {
        let h = TestHarness::new().await;
        h.add_ready_ssp("ssp-1", "http://10.0.0.1:8667").await;
        let app = admin_app(&h, Some("pw"));
        let token = breakglass_token(&app, "pw").await;

        let admin = body_json(
            app.oneshot(get_auth("/admin/api/overview", &token))
                .await
                .unwrap(),
        )
        .await;
        let info = body_json(h.metrics_router().oneshot(get("/info")).await.unwrap()).await;

        let info_ssp = info
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["entity"] == "ssp")
            .unwrap();

        // Both are built from `metrics::build_entities`, and that must stay
        // true: an operator comparing the two must never see them disagree.
        assert_eq!(admin["ssps"][0]["id"], info_ssp["id"]);
        assert_eq!(admin["ssps"][0]["status"], info_ssp["status"]);
    }

    #[tokio::test]
    async fn ssp_entities_carry_the_progress_fields() {
        let h = TestHarness::new().await;
        h.add_ready_ssp("ssp-1", "http://10.0.0.1:8667").await;

        let info = body_json(h.metrics_router().oneshot(get("/info")).await.unwrap()).await;
        let ssp = info
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["entity"] == "ssp")
            .unwrap();

        assert!(ssp.get("state_seconds").is_some());
        assert_eq!(ssp["buffered_events"], 0);
        // A ready SSP has no bootstrap in flight.
        assert!(ssp["bootstrap"].is_null());
    }

    #[tokio::test]
    async fn backends_list_is_empty_but_well_formed() {
        let h = TestHarness::new().await;
        let app = admin_app(&h, Some("pw"));
        let token = breakglass_token(&app, "pw").await;

        let body = body_json(
            app.oneshot(get_auth("/admin/api/backends", &token))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(body["backends"].as_array().unwrap().len(), 0);
        assert_eq!(body["check_interval_secs"], 15);
    }

    #[tokio::test]
    async fn an_unknown_backend_is_a_404() {
        let h = TestHarness::new().await;
        let app = admin_app(&h, Some("pw"));
        let token = breakglass_token(&app, "pw").await;

        let res = app
            .oneshot(get_auth("/admin/api/backends/ghost", &token))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn workflow_endpoints_are_unavailable_until_the_database_handle_lands() {
        let h = TestHarness::new().await;
        let app = admin_app(&h, Some("pw"));
        let token = breakglass_token(&app, "pw").await;

        // The admin plane comes up with the HTTP servers, before
        // `Scheduler::start()` has built the shared handle. That window must
        // be a clean 503, matching how every other endpoint behaves while the
        // scheduler is still cloning.
        for path in [
            "/admin/api/workflows/runs",
            "/admin/api/workflows/runs/abc",
            "/admin/api/workflows/stream",
            "/admin/api/schedules",
        ] {
            let res = app
                .clone()
                .oneshot(get_auth(path, &token))
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE, "{path}");
        }
    }

    #[tokio::test]
    async fn an_unknown_log_source_is_rejected() {
        let h = TestHarness::new().await;
        let app = admin_app(&h, Some("pw"));
        let token = breakglass_token(&app, "pw").await;

        let res = app
            .oneshot(get_auth("/admin/api/logs?source=backend:api", &token))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn logs_for_an_unknown_ssp_are_a_404() {
        let h = TestHarness::new().await;
        let app = admin_app(&h, Some("pw"));
        let token = breakglass_token(&app, "pw").await;

        let res = app
            .oneshot(get_auth("/admin/api/logs?source=ssp:ghost", &token))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn the_scheduler_log_stream_replays_history() {
        let h = TestHarness::new().await;
        let app = admin_app(&h, Some("pw"));
        let token = breakglass_token(&app, "pw").await;

        let res = app
            .oneshot(get_auth(
                "/admin/api/logs?source=scheduler&tail=false",
                &token,
            ))
            .await
            .unwrap();

        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok()),
            Some("text/event-stream")
        );
        // `tail=false` closes on its own, so the body is finite and safe to
        // collect here — a tailing stream would hang this test forever.
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        assert!(String::from_utf8_lossy(&bytes).len() < 10_000);
    }

    #[tokio::test]
    async fn a_missing_dashboard_bundle_serves_a_placeholder_not_a_broken_api() {
        let h = TestHarness::new().await;

        let page = admin_app(&h, None).oneshot(get("/admin")).await.unwrap();
        assert_eq!(page.status(), StatusCode::NOT_FOUND);
        let body = page.into_body().collect().await.unwrap().to_bytes();
        let html = String::from_utf8_lossy(&body);
        assert!(html.contains("Dashboard not bundled"), "{html}");

        // The API on the same port keeps working, which is the whole point of
        // serving the bundle from disk rather than embedding it.
        let api = admin_app(&h, None)
            .oneshot(get("/admin/api/config"))
            .await
            .unwrap();
        assert_eq!(api.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn the_admin_plane_does_not_serve_the_ingest_routes() {
        let h = TestHarness::new().await;
        // The separation is the security boundary: publishing the admin port
        // must not publish `/proxy/query`, which runs arbitrary SurrealQL.
        for path in ["/proxy/query", "/ingest", "/metrics", "/health", "/info"] {
            let res = admin_app(&h, None).oneshot(get(path)).await.unwrap();
            assert_eq!(
                res.status(),
                StatusCode::NOT_FOUND,
                "{path} must not exist on the admin port"
            );
        }
    }

    #[tokio::test]
    async fn the_ingest_plane_does_not_serve_the_admin_api() {
        let h = TestHarness::new().await;
        for path in ["/admin/api/config", "/admin/api/overview"] {
            let res = h.full_app().oneshot(get(path)).await.unwrap();
            assert_eq!(
                res.status(),
                StatusCode::NOT_FOUND,
                "{path} must not exist on the ingest port"
            );
        }
    }

    #[tokio::test]
    async fn bootstrap_progress_is_recorded_and_cleared_on_ready() {
        let h = TestHarness::new().await;

        // A registered but still-bootstrapping SSP.
        {
            let mut pool = h.ssp_pool.write().await;
            pool.upsert(SspInfo {
                id: "ssp-boot".to_string(),
                url: "http://10.0.0.9:8667".to_string(),
                version: "test".to_string(),
                connected_at: std::time::Instant::now(),
                last_heartbeat: std::time::Instant::now(),
                query_count: 0,
                views: 0,
                cpu_usage: None,
                memory_usage: None,
                env: None,
                bootstrap: None,
            });
            pool.mark_bootstrapping("ssp-boot");
        }

        let res = h
            .ssp_router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/ssp/bootstrap-progress")
                    .header("Content-Type", "application/json")
                    .body(axum::body::Body::from(
                        json!({
                            "ssp_id": "ssp-boot",
                            "tables_done": 3,
                            "tables_total": 10,
                            "rows_loaded": 4200,
                            "current_table": "message"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        let info = body_json(h.metrics_router().oneshot(get("/info")).await.unwrap()).await;
        let ssp = info
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["id"] == "ssp-boot")
            .unwrap();
        assert_eq!(ssp["status"], "bootstrapping");
        assert_eq!(ssp["bootstrap"]["tables_done"], 3);
        assert_eq!(ssp["bootstrap"]["tables_total"], 10);
        assert_eq!(ssp["bootstrap"]["rows_loaded"], 4200);
        assert_eq!(ssp["bootstrap"]["current_table"], "message");

        // Going ready must drop the bar rather than leave a stale one behind.
        h.ssp_pool.write().await.mark_ready("ssp-boot");
        let info = body_json(h.metrics_router().oneshot(get("/info")).await.unwrap()).await;
        let ssp = info
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["id"] == "ssp-boot")
            .unwrap();
        assert_eq!(ssp["status"], "ready");
        assert!(ssp["bootstrap"].is_null());
    }

    #[tokio::test]
    async fn progress_for_an_unknown_ssp_is_accepted_and_ignored() {
        let h = TestHarness::new().await;
        // Advisory: a report can outlive the SSP it describes, and that must
        // never be an error the bootstrapping SSP has to handle.
        let res = h
            .ssp_router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/ssp/bootstrap-progress")
                    .header("Content-Type", "application/json")
                    .body(axum::body::Body::from(
                        json!({ "ssp_id": "ghost", "tables_done": 1, "tables_total": 2 })
                            .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }
}
