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
    transport: Arc<HttpTransport>,
    query_tracker: Arc<QueryTracker>,
    job_tracker: Arc<JobTracker>,
    config: Arc<SchedulerConfig>,
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
                url: "ws://localhost:8000".to_string(),
                namespace: "spooky".to_string(),
                database: "spooky".to_string(),
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
        };

        Self {
            replica: Arc::new(RwLock::new(replica)),
            ssp_pool: Arc::new(RwLock::new(SspPool::new(
                LoadBalanceStrategy::LeastQueries,
                max_buffer,
            ))),
            status: Arc::new(RwLock::new(status)),
            event_buffer: Arc::new(RwLock::new(VecDeque::new())),
            seq_counter: Arc::new(AtomicU64::new(0)),
            wal: Arc::new(RwLock::new(wal)),
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
        };
        ssp_management::create_ssp_router(state)
    }

    fn proxy_router(&self) -> Router {
        let state = ProxyState {
            replica: Arc::clone(&self.replica),
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
        let state = MetricsState {
            ssp_pool: Arc::clone(&self.ssp_pool),
            query_tracker: Arc::clone(&self.query_tracker),
            job_tracker: Arc::clone(&self.job_tracker),
            start_time: std::time::Instant::now(),
            scheduler_id: "test-scheduler".to_string(),
            status: Arc::clone(&self.status),
        };
        metrics::create_metrics_router(state)
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
            connected_at: std::time::Instant::now(),
            last_heartbeat: std::time::Instant::now(),
            query_count: 0,
            views: 0,
            cpu_usage: None,
            memory_usage: None,
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
            connected_at: std::time::Instant::now(),
            last_heartbeat: std::time::Instant::now(),
            query_count: 0,
            views: 0,
            cpu_usage: None,
            memory_usage: None,
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
                    axum::routing::get(|| async {
                        axum::Json(json!({"status": "ready"}))
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
            "url": url
        })
    }

    fn heartbeat_payload(ssp_id: &str) -> Value {
        json!({
            "ssp_id": ssp_id,
            "timestamp": 1000,
            "views": 5,
            "cpu_usage": 45.0,
            "memory_usage": 60.0
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
        // INFO FOR DB returns a non-empty result with database metadata
        assert!(body.is_array());
        let results = body.as_array().unwrap();
        assert!(!results.is_empty(), "INFO FOR DB should return metadata");
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
    async fn query_unregister_not_found() {
        let h = TestHarness::new().await;
        let app = h.query_router();

        let (status, _) = post_json(
            app,
            "/view/unregister",
            &json!({"id": "nonexistent"}),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
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
            &json!({"ssp_id": "ssp-1", "url": "http://localhost:9999"}),
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
            &json!({"ssp_id": "ssp-1", "url": "http://localhost:9999"}),
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
                json!({"ssp_id": "s1", "url": "http://localhost:1234"}),
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
    async fn snapshot_update_skips_during_bootstrap() {
        let h = TestHarness::new().await;

        // Register an SSP to trigger bootstrap
        let app = h.ssp_router();
        let (status, _) = post_json(
            app,
            "/ssp/register",
            &json!({"ssp_id": "ssp-1", "url": "http://localhost:9999"}),
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
}
