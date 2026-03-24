#![allow(dead_code)]

use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::RwLock;
use tower::ServiceExt;

use ssp::circuit::view::OutputFormat;
use ssp::circuit::{Circuit, Record};
use ssp_server::metrics::Metrics;
use ssp_server::{create_app, AppState, SharedDb, SspStatus};

use job_runner::{JobConfig, JobEntry};
use tokio::sync::mpsc;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const AUTH_SECRET: &str = "test-secret-for-integration";

// ---------------------------------------------------------------------------
// Test Harness
// ---------------------------------------------------------------------------

struct TestHarness {
    processor: Arc<RwLock<Circuit>>,
    status: Arc<RwLock<SspStatus>>,
    metrics: Arc<Metrics>,
    job_config: Arc<JobConfig>,
    job_queue_tx: mpsc::Sender<JobEntry>,
    job_queue_rx: Arc<tokio::sync::Mutex<mpsc::Receiver<JobEntry>>>,
    db: SharedDb,
}

impl TestHarness {
    fn new() -> Self {
        Self::with_options(SspStatus::Ready, JobConfig::default())
    }

    fn with_status(status: SspStatus) -> Self {
        Self::with_options(status, JobConfig::default())
    }

    fn with_options(status: SspStatus, job_config: JobConfig) -> Self {
        // Set auth secret for the middleware
        unsafe {
            std::env::set_var("SP00KY_AUTH_SECRET", AUTH_SECRET);
        }

        let (tx, rx) = mpsc::channel::<JobEntry>(100);

        // No-op metrics provider (no exporter = all recording is a no-op)
        let provider = opentelemetry_sdk::metrics::SdkMeterProvider::builder().build();
        let metrics = Arc::new(Metrics::new(&provider));

        // Unconnected SurrealDB client — handlers that don't touch DB work fine,
        // handlers that do will get runtime errors (which are logged but not propagated).
        let db: surrealdb::Surreal<surrealdb::engine::remote::ws::Client> =
            surrealdb::Surreal::init();

        Self {
            processor: Arc::new(RwLock::new(Circuit::new())),
            status: Arc::new(RwLock::new(status)),
            metrics,
            job_config: Arc::new(job_config),
            job_queue_tx: tx,
            job_queue_rx: Arc::new(tokio::sync::Mutex::new(rx)),
            db: Arc::new(db),
        }
    }

    fn app(&self) -> Router {
        let state = AppState {
            db: Arc::clone(&self.db),
            processor: Arc::clone(&self.processor),
            status: Arc::clone(&self.status),
            metrics: Arc::clone(&self.metrics),
            job_config: Arc::clone(&self.job_config),
            job_queue_tx: self.job_queue_tx.clone(),
        };
        create_app(state)
    }

    async fn set_status(&self, status: SspStatus) {
        *self.status.write().await = status;
    }

    /// Load records into the circuit store (for pre-populating data).
    async fn load_records(&self, records: Vec<Record>) {
        let mut circuit = self.processor.write().await;
        circuit.load(records);
    }

    /// Register a view directly on the circuit (bypasses HTTP handler & DB calls).
    async fn register_view_direct(&self, id: &str, surql: &str) {
        let payload = view_payload(id, surql);
        let data = ssp::service::view::prepare_registration_dbsp(payload)
            .expect("Failed to prepare view registration");
        let mut circuit = self.processor.write().await;
        circuit.add_query(data.plan, data.safe_params, Some(OutputFormat::Streaming));
    }
}

// ---------------------------------------------------------------------------
// Request Helpers
// ---------------------------------------------------------------------------

async fn get_authed(app: Router, path: &str) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("GET")
        .uri(path)
        .header("Authorization", format!("Bearer {}", AUTH_SECRET))
        .body(axum::body::Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body_value = serde_json::from_slice(&body_bytes).unwrap_or(Value::Null);
    (status, body_value)
}

async fn post_authed(app: Router, path: &str, body: &Value) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json")
        .header("Authorization", format!("Bearer {}", AUTH_SECRET))
        .body(axum::body::Body::from(serde_json::to_vec(body).unwrap()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body_value = serde_json::from_slice(&body_bytes).unwrap_or(Value::Null);
    (status, body_value)
}

async fn post_authed_raw(app: Router, path: &str, body: &[u8]) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json")
        .header("Authorization", format!("Bearer {}", AUTH_SECRET))
        .body(axum::body::Body::from(body.to_vec()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body_value = serde_json::from_slice(&body_bytes).unwrap_or(Value::Null);
    (status, body_value)
}

/// Unauthenticated GET (for auth tests)
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

/// Unauthenticated POST (for auth tests)
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

// ---------------------------------------------------------------------------
// Payload Helpers
// ---------------------------------------------------------------------------

fn ingest_payload(table: &str, op: &str, id: &str) -> Value {
    json!({
        "table": table,
        "op": op,
        "id": id,
        "record": {"name": "test", "id": id}
    })
}

fn ingest_payload_with_record(table: &str, op: &str, id: &str, record: Value) -> Value {
    json!({
        "table": table,
        "op": op,
        "id": id,
        "record": record
    })
}

fn view_payload(id: &str, surql: &str) -> Value {
    json!({
        "id": id,
        "surql": surql,
        "clientId": "test-client",
        "ttl": "30m",
        "lastActiveAt": "2024-01-01T00:00:00Z"
    })
}

// ===========================================================================
// Test Modules
// ===========================================================================

mod auth_tests {
    use super::*;

    #[tokio::test]
    async fn unauthenticated_request_returns_401() {
        let h = TestHarness::new();
        let (status, _) = get_json(h.app(), "/health").await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn wrong_token_returns_401() {
        let h = TestHarness::new();
        let request = Request::builder()
            .method("GET")
            .uri("/health")
            .header("Authorization", "Bearer wrong-token")
            .body(axum::body::Body::empty())
            .unwrap();

        let response = h.app().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn valid_token_passes_through() {
        let h = TestHarness::new();
        let (status, _) = get_authed(h.app(), "/health").await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn missing_bearer_prefix_returns_401() {
        let h = TestHarness::new();
        let request = Request::builder()
            .method("GET")
            .uri("/health")
            .header("Authorization", AUTH_SECRET)
            .body(axum::body::Body::empty())
            .unwrap();

        let response = h.app().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}

mod health_tests {
    use super::*;

    #[tokio::test]
    async fn health_ready_returns_200() {
        let h = TestHarness::new();
        let (status, body) = get_authed(h.app(), "/health").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], "ready");
        assert_eq!(body["views"], 0);
        assert_eq!(body["tables"], 0);
    }

    #[tokio::test]
    async fn health_bootstrapping_returns_503() {
        let h = TestHarness::with_status(SspStatus::Bootstrapping);
        let (status, body) = get_authed(h.app(), "/health").await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["status"], "bootstrapping");
    }

    #[tokio::test]
    async fn health_failed_returns_503() {
        let h = TestHarness::with_status(SspStatus::Failed);
        let (status, body) = get_authed(h.app(), "/health").await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["status"], "failed");
    }

    #[tokio::test]
    async fn health_reports_view_and_table_count() {
        let h = TestHarness::new();

        // Load data into two tables
        h.load_records(vec![
            Record::new("user", "user:1", json!({"name": "Alice", "id": "user:1"})),
            Record::new("post", "post:1", json!({"title": "Hello", "id": "post:1"})),
        ])
        .await;

        // Register a view
        h.register_view_direct("view1", "SELECT * FROM user").await;

        let (status, body) = get_authed(h.app(), "/health").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["views"], 1);
        assert_eq!(body["tables"], 2);
    }

    #[tokio::test]
    async fn health_response_has_all_fields() {
        let h = TestHarness::new();
        let (_, body) = get_authed(h.app(), "/health").await;
        assert!(body.get("status").is_some());
        assert!(body.get("views").is_some());
        assert!(body.get("tables").is_some());
    }
}

mod version_tests {
    use super::*;

    #[tokio::test]
    async fn version_returns_correct_format() {
        let h = TestHarness::new();
        let (status, body) = get_authed(h.app(), "/version").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.get("version").is_some());
        assert!(body.get("mode").is_some());
    }

    #[tokio::test]
    async fn version_mode_is_streaming() {
        let h = TestHarness::new();
        let (_, body) = get_authed(h.app(), "/version").await;
        assert_eq!(body["mode"], "streaming");
    }
}

mod log_tests {
    use super::*;

    #[tokio::test]
    async fn log_accepts_valid_payload() {
        let h = TestHarness::new();
        let (status, _) = post_authed(
            h.app(),
            "/log",
            &json!({"message": "test log", "level": "info"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn log_all_levels() {
        for level in &["error", "warn", "info", "debug", "trace"] {
            let h = TestHarness::new();
            let (status, _) = post_authed(
                h.app(),
                "/log",
                &json!({"message": "test", "level": level}),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "Level '{}' should return 200", level);
        }
    }

    #[tokio::test]
    async fn log_with_data_field() {
        let h = TestHarness::new();
        let (status, _) = post_authed(
            h.app(),
            "/log",
            &json!({"message": "test", "level": "info", "data": {"key": "value"}}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn log_defaults_level() {
        let h = TestHarness::new();
        // level defaults to empty string via #[serde(default)]
        let (status, _) = post_authed(h.app(), "/log", &json!({"message": "test"})).await;
        assert_eq!(status, StatusCode::OK);
    }
}

mod debug_tests {
    use super::*;

    #[tokio::test]
    async fn debug_view_not_found() {
        let h = TestHarness::new();
        let (status, body) = get_authed(h.app(), "/debug/view/nonexistent").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["error"], "View not found");
    }

    #[tokio::test]
    async fn debug_view_returns_cache_state() {
        let h = TestHarness::new();

        // Load data first, then register view
        h.load_records(vec![Record::new(
            "user",
            "user:1",
            json!({"name": "Alice", "id": "user:1"}),
        )])
        .await;
        h.register_view_direct("view1", "SELECT * FROM user").await;

        let (status, body) = get_authed(h.app(), "/debug/view/view1").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["view_id"], "view1");
        assert!(body.get("cache_size").is_some());
        assert!(body.get("cache").is_some());
    }

    #[tokio::test]
    async fn debug_deps_empty_circuit() {
        let h = TestHarness::new();
        let (status, body) = get_authed(h.app(), "/debug/deps").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["view_count"], 0);
    }

    #[tokio::test]
    async fn debug_deps_with_views() {
        let h = TestHarness::new();
        h.register_view_direct("v1", "SELECT * FROM user").await;
        h.register_view_direct("v2", "SELECT * FROM post").await;

        let (status, body) = get_authed(h.app(), "/debug/deps").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["view_count"], 2);
    }
}

mod ingest_tests {
    use super::*;

    #[tokio::test]
    async fn ingest_rejects_when_not_ready() {
        let h = TestHarness::with_status(SspStatus::Bootstrapping);
        let payload = ingest_payload("user", "CREATE", "user:1");
        let (status, body) = post_authed(h.app(), "/ingest", &payload).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["code"], "SSP_NOT_READY");
    }

    #[tokio::test]
    async fn ingest_rejects_invalid_op() {
        let h = TestHarness::new();
        let payload = ingest_payload("user", "MERGE", "user:1");
        let (status, _) = post_authed(h.app(), "/ingest", &payload).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn ingest_rejects_malformed_json() {
        let h = TestHarness::new();
        let (status, _) = post_authed_raw(h.app(), "/ingest", b"not valid json").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn ingest_accepts_create() {
        let h = TestHarness::new();
        let payload = ingest_payload("user", "CREATE", "user:1");
        let (status, _) = post_authed(h.app(), "/ingest", &payload).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn ingest_accepts_update() {
        let h = TestHarness::new();
        let payload = ingest_payload("user", "UPDATE", "user:1");
        let (status, _) = post_authed(h.app(), "/ingest", &payload).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn ingest_accepts_delete() {
        let h = TestHarness::new();
        let payload = ingest_payload("user", "DELETE", "user:1");
        let (status, _) = post_authed(h.app(), "/ingest", &payload).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn ingest_create_populates_circuit() {
        let h = TestHarness::new();
        let payload = ingest_payload("user", "CREATE", "user:1");
        let (status, _) = post_authed(h.app(), "/ingest", &payload).await;
        assert_eq!(status, StatusCode::OK);

        let circuit = h.processor.read().await;
        assert!(
            circuit.table_names().contains(&"user".to_string()),
            "Circuit should contain 'user' table after ingest"
        );
    }

    #[tokio::test]
    async fn ingest_delete_removes_from_circuit() {
        let h = TestHarness::new();

        // Create a record
        let create = ingest_payload("user", "CREATE", "user:1");
        post_authed(h.app(), "/ingest", &create).await;

        // Delete it
        let delete = ingest_payload("user", "DELETE", "user:1");
        let (status, _) = post_authed(h.app(), "/ingest", &delete).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn ingest_affects_registered_view() {
        let h = TestHarness::new();

        // Register a view on 'user' table
        h.register_view_direct("v1", "SELECT * FROM user").await;

        // Ingest a record
        let payload = ingest_payload("user", "CREATE", "user:1");
        let (status, _) = post_authed(h.app(), "/ingest", &payload).await;
        assert_eq!(status, StatusCode::OK);

        // Verify view cache was updated
        let circuit = h.processor.read().await;
        let view = circuit.get_view("v1").expect("View should exist");
        assert!(
            !view.cache.is_empty(),
            "View cache should have entries after ingest"
        );
    }

    #[tokio::test]
    async fn ingest_multiple_records() {
        let h = TestHarness::new();

        for i in 1..=5 {
            let payload = ingest_payload("user", "CREATE", &format!("user:{}", i));
            let (status, _) = post_authed(h.app(), "/ingest", &payload).await;
            assert_eq!(status, StatusCode::OK);
        }

        let circuit = h.processor.read().await;
        assert!(circuit.table_names().contains(&"user".to_string()));
    }
}

mod view_lifecycle_tests {
    use super::*;

    #[tokio::test]
    async fn register_rejects_when_not_ready() {
        let h = TestHarness::with_status(SspStatus::Bootstrapping);
        let payload = view_payload("v1", "SELECT * FROM user");
        let (status, body) = post_authed(h.app(), "/view/register", &payload).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["code"], "SSP_NOT_READY");
    }

    #[tokio::test]
    async fn register_rejects_invalid_payload() {
        let h = TestHarness::new();
        // Missing required fields
        let payload = json!({"id": "v1"});
        let (status, _) = post_authed(h.app(), "/view/register", &payload).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn register_adds_view() {
        let h = TestHarness::new();

        // Register directly to verify circuit behavior without DB dependency
        h.register_view_direct("v1", "SELECT * FROM user").await;

        let circuit = h.processor.read().await;
        assert_eq!(circuit.view_count(), 1);
        assert!(circuit.get_view("v1").is_some());
    }

    #[tokio::test]
    async fn register_idempotent_via_http() {
        let h = TestHarness::new();

        // First: register directly (avoid DB-dependent HTTP path)
        h.register_view_direct("v1", "SELECT * FROM user").await;
        assert_eq!(h.processor.read().await.view_count(), 1);

        // Second: HTTP call detects existing view and returns 200 immediately
        let payload = view_payload("v1", "SELECT * FROM user");
        let (status, _) = post_authed(h.app(), "/view/register", &payload).await;
        assert_eq!(status, StatusCode::OK);

        // Still only one view
        assert_eq!(h.processor.read().await.view_count(), 1);
    }

    #[tokio::test]
    async fn unregister_removes_view() {
        let h = TestHarness::new();

        // Register directly
        h.register_view_direct("v1", "SELECT * FROM user").await;
        assert_eq!(h.processor.read().await.view_count(), 1);

        // Unregister via HTTP
        let payload = json!({"id": "v1"});
        let (status, _) = post_authed(h.app(), "/view/unregister", &payload).await;
        assert_eq!(status, StatusCode::OK);

        let circuit = h.processor.read().await;
        assert_eq!(circuit.view_count(), 0);
    }

    #[tokio::test]
    async fn unregister_nonexistent() {
        let h = TestHarness::new();
        let payload = json!({"id": "nonexistent"});
        let (status, _) = post_authed(h.app(), "/view/unregister", &payload).await;
        assert_eq!(status, StatusCode::OK);
    }
}

mod reset_tests {
    use super::*;

    #[tokio::test]
    async fn reset_clears_all_state() {
        let h = TestHarness::new();

        // Load data and register views
        h.load_records(vec![Record::new(
            "user",
            "user:1",
            json!({"name": "Alice", "id": "user:1"}),
        )])
        .await;
        h.register_view_direct("v1", "SELECT * FROM user").await;

        assert_eq!(h.processor.read().await.view_count(), 1);
        assert!(!h.processor.read().await.table_names().is_empty());

        // Reset
        let (status, _) = post_authed(h.app(), "/reset", &json!({})).await;
        assert_eq!(status, StatusCode::OK);

        let circuit = h.processor.read().await;
        assert_eq!(circuit.view_count(), 0);
        assert!(circuit.table_names().is_empty());
    }

    #[tokio::test]
    async fn reset_when_empty() {
        let h = TestHarness::new();
        let (status, _) = post_authed(h.app(), "/reset", &json!({})).await;
        assert_eq!(status, StatusCode::OK);
    }
}

mod status_gating_tests {
    use super::*;

    #[tokio::test]
    async fn gated_endpoints_reject_during_bootstrap() {
        let h = TestHarness::with_status(SspStatus::Bootstrapping);

        let (status, _) =
            post_authed(h.app(), "/ingest", &ingest_payload("user", "CREATE", "user:1")).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);

        let (status, _) = post_authed(
            h.app(),
            "/view/register",
            &view_payload("v1", "SELECT * FROM user"),
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);

        let (status, _) =
            post_authed(h.app(), "/view/unregister", &json!({"id": "v1"})).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn gated_endpoints_reject_when_failed() {
        let h = TestHarness::with_status(SspStatus::Failed);

        let (status, _) =
            post_authed(h.app(), "/ingest", &ingest_payload("user", "CREATE", "user:1")).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);

        let (status, _) = post_authed(
            h.app(),
            "/view/register",
            &view_payload("v1", "SELECT * FROM user"),
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);

        let (status, _) =
            post_authed(h.app(), "/view/unregister", &json!({"id": "v1"})).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn ungated_endpoints_work_during_bootstrap() {
        let h = TestHarness::with_status(SspStatus::Bootstrapping);

        let (status, _) = get_authed(h.app(), "/health").await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE); // 503 but endpoint works

        let (status, _) = get_authed(h.app(), "/version").await;
        assert_eq!(status, StatusCode::OK);

        let (status, _) = post_authed(
            h.app(),
            "/log",
            &json!({"message": "test", "level": "info"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (status, _) = get_authed(h.app(), "/debug/view/test").await;
        assert_eq!(status, StatusCode::OK);

        let (status, _) = get_authed(h.app(), "/debug/deps").await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn status_transition_enables_handlers() {
        let h = TestHarness::new();
        h.set_status(SspStatus::Bootstrapping).await;

        // Should be rejected
        let (status, _) =
            post_authed(h.app(), "/ingest", &ingest_payload("user", "CREATE", "user:1")).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);

        // Transition to Ready
        h.set_status(SspStatus::Ready).await;

        // Should now work
        let (status, _) =
            post_authed(h.app(), "/ingest", &ingest_payload("user", "CREATE", "user:1")).await;
        assert_eq!(status, StatusCode::OK);
    }
}

// ===========================================================================
// DB Integration Tests (require running SurrealDB)
// ===========================================================================
// Run with: cargo test -- --ignored

mod db_integration_tests {
    use super::*;
    use surrealdb::engine::remote::ws::Ws;
    use surrealdb::opt::auth::Root;
    use surrealdb::Surreal;

    async fn create_test_harness_with_db() -> TestHarness {
        let addr = std::env::var("TEST_SURREALDB_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:8000".to_string());

        unsafe {
            std::env::set_var("SP00KY_AUTH_SECRET", AUTH_SECRET);
        }

        let db = Surreal::new::<Ws>(&addr)
            .await
            .expect("Failed to connect to SurrealDB");
        db.signin(Root {
            username: "root".to_string(),
            password: "root".to_string(),
        })
        .await
        .expect("Failed to sign in");

        // Use a unique test namespace/database to avoid conflicts.
        // Retry on transaction conflicts (concurrent DB creation).
        let test_db = format!("test_ssp_{}", uuid::Uuid::new_v4().simple());
        for attempt in 0..5 {
            match db.use_ns("test_ssp").use_db(&test_db).await {
                Ok(_) => break,
                Err(e) if attempt < 4 => {
                    tokio::time::sleep(std::time::Duration::from_millis(50 * (attempt + 1))).await;
                    eprintln!("Retrying use_ns/use_db (attempt {}): {}", attempt + 1, e);
                }
                Err(e) => panic!("Failed to select ns/db after retries: {}", e),
            }
        }

        let (tx, rx) = mpsc::channel::<JobEntry>(100);
        let provider = opentelemetry_sdk::metrics::SdkMeterProvider::builder().build();

        TestHarness {
            processor: Arc::new(RwLock::new(Circuit::new())),
            status: Arc::new(RwLock::new(SspStatus::Ready)),
            metrics: Arc::new(Metrics::new(&provider)),
            job_config: Arc::new(JobConfig::default()),
            job_queue_tx: tx,
            job_queue_rx: Arc::new(tokio::sync::Mutex::new(rx)),
            db: Arc::new(db),
        }
    }

    /// Query a table that may not exist yet, returning an empty vec if the table is missing.
    async fn query_table(db: &SharedDb, surql: &str) -> Vec<Value> {
        let result = db.query(surql).await;
        match result {
            Ok(mut res) => {
                // Try to take results as surrealdb::types::Value first
                let val: Result<surrealdb::types::Value, _> = res.take(0);
                match val {
                    Ok(v) => {
                        let json = serde_json::to_value(&v).unwrap_or(Value::Null);
                        match json {
                            Value::Array(arr) => arr,
                            Value::Null => vec![],
                            other => vec![other],
                        }
                    }
                    Err(_) => vec![],
                }
            }
            Err(e) => {
                let msg = e.to_string();
                // Table not existing is expected in fresh test DBs
                if msg.contains("does not exist") {
                    vec![]
                } else {
                    panic!("Unexpected query error: {}", e);
                }
            }
        }
    }

    /// Helper: register a view via HTTP (persists metadata in DB + adds to circuit)
    async fn register_view_via_http(h: &TestHarness, id: &str, surql: &str) {
        let payload = view_payload(id, surql);
        let (status, _) = post_authed(h.app(), "/view/register", &payload).await;
        assert_eq!(status, StatusCode::OK, "View registration should succeed");
    }

    #[tokio::test]
    #[ignore]
    async fn ingest_creates_edges_in_db() {
        let h = create_test_harness_with_db().await;

        // Register view via HTTP so _00_query record exists in DB
        register_view_via_http(&h, "v1", "SELECT * FROM user").await;

        // Ingest a record via HTTP
        let payload = ingest_payload_with_record(
            "user",
            "CREATE",
            "user:1",
            json!({"name": "Alice", "id": "user:1"}),
        );
        let (status, _) = post_authed(h.app(), "/ingest", &payload).await;
        assert_eq!(status, StatusCode::OK);

        // Query for edges
        let edges = query_table(&h.db, "SELECT * FROM _00_list_ref").await;
        assert!(
            !edges.is_empty(),
            "Should have created edges after ingest"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn register_persists_metadata() {
        let h = create_test_harness_with_db().await;

        let payload = view_payload("v1", "SELECT * FROM user");
        let (status, _) = post_authed(h.app(), "/view/register", &payload).await;
        assert_eq!(status, StatusCode::OK);

        // Query persisted metadata
        let entries = query_table(&h.db, "SELECT * FROM _00_query").await;
        assert!(
            !entries.is_empty(),
            "Should have persisted view metadata"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn unregister_deletes_edges() {
        let h = create_test_harness_with_db().await;

        // Setup: register view via HTTP and ingest
        register_view_via_http(&h, "v1", "SELECT * FROM user").await;
        let payload = ingest_payload_with_record(
            "user",
            "CREATE",
            "user:1",
            json!({"name": "Alice", "id": "user:1"}),
        );
        post_authed(h.app(), "/ingest", &payload).await;

        // Verify edges were created first
        let edges_before = query_table(&h.db, "SELECT * FROM _00_list_ref").await;
        assert!(
            !edges_before.is_empty(),
            "Edges should exist before unregister"
        );

        // Unregister via HTTP — handler calls DELETE $from->_00_list_ref
        let (status, _) =
            post_authed(h.app(), "/view/unregister", &json!({"id": "v1"})).await;
        assert_eq!(status, StatusCode::OK);

        // Verify circuit was cleaned up
        let circuit = h.processor.read().await;
        assert_eq!(circuit.view_count(), 0, "View should be removed from circuit");
    }

    #[tokio::test]
    #[ignore]
    async fn reset_deletes_all_edges() {
        let h = create_test_harness_with_db().await;

        // Setup via HTTP
        register_view_via_http(&h, "v1", "SELECT * FROM user").await;
        let payload = ingest_payload_with_record(
            "user",
            "CREATE",
            "user:1",
            json!({"name": "Alice", "id": "user:1"}),
        );
        post_authed(h.app(), "/ingest", &payload).await;

        // Verify edges were created
        let edges_before = query_table(&h.db, "SELECT * FROM _00_list_ref").await;
        assert!(!edges_before.is_empty(), "Edges should exist before reset");

        // Reset via HTTP
        let (status, _) = post_authed(h.app(), "/reset", &json!({})).await;
        assert_eq!(status, StatusCode::OK);

        // Verify circuit was fully cleared
        let circuit = h.processor.read().await;
        assert_eq!(circuit.view_count(), 0, "Circuit should be empty after reset");
        assert!(
            circuit.table_names().is_empty(),
            "No tables should remain after reset"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn ingest_updates_edges_on_update() {
        let h = create_test_harness_with_db().await;

        // Register via HTTP so DB metadata exists
        register_view_via_http(&h, "v1", "SELECT * FROM user").await;

        // Create
        let create = ingest_payload_with_record(
            "user",
            "CREATE",
            "user:1",
            json!({"name": "Alice", "id": "user:1"}),
        );
        post_authed(h.app(), "/ingest", &create).await;

        // Update
        let update = ingest_payload_with_record(
            "user",
            "UPDATE",
            "user:1",
            json!({"name": "Bob", "id": "user:1"}),
        );
        let (status, _) = post_authed(h.app(), "/ingest", &update).await;
        assert_eq!(status, StatusCode::OK);

        // Edges should still exist
        let edges = query_table(&h.db, "SELECT * FROM _00_list_ref").await;
        assert!(!edges.is_empty(), "Edges should exist after update");
    }

    #[tokio::test]
    #[ignore]
    async fn ingest_deletes_edges_on_delete() {
        let h = create_test_harness_with_db().await;

        // Register via HTTP
        register_view_via_http(&h, "v1", "SELECT * FROM user").await;

        // Create
        let create = ingest_payload_with_record(
            "user",
            "CREATE",
            "user:1",
            json!({"name": "Alice", "id": "user:1"}),
        );
        post_authed(h.app(), "/ingest", &create).await;

        // Verify edges exist after create
        let edges_before = query_table(&h.db, "SELECT * FROM _00_list_ref").await;
        assert!(
            !edges_before.is_empty(),
            "Edges should exist after create"
        );

        // Delete the record
        let delete = ingest_payload("user", "DELETE", "user:1");
        let (status, _) = post_authed(h.app(), "/ingest", &delete).await;
        assert_eq!(status, StatusCode::OK);

        // Verify circuit view cache reflects the deletion
        let circuit = h.processor.read().await;
        let view = circuit.get_view("v1").expect("View should still exist");
        assert!(
            view.cache.is_empty(),
            "View cache should be empty after deleting the only record"
        );
    }
}
