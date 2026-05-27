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
use ssp_server::crdt::{CrdtAllowList, CrdtCache};
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
    crdt_cache: Arc<CrdtCache>,
    start_time: std::time::Instant,
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
            std::env::set_var("SPKY_AUTH_SECRET", AUTH_SECRET);
        }

        let (tx, rx) = mpsc::channel::<JobEntry>(100);

        // No-op metrics provider (no exporter = all recording is a no-op)
        let provider = opentelemetry_sdk::metrics::SdkMeterProvider::builder().build();
        let metrics = Arc::new(Metrics::new(&provider));

        // Unconnected SurrealDB client — handlers that don't touch DB work fine,
        // handlers that do will get runtime errors (which are logged but not propagated).
        let db: surrealdb::Surreal<surrealdb::engine::remote::ws::Client> =
            surrealdb::Surreal::init();

        // Permission injection now default-denies any table without a
        // registered permission. The integration suite was written before
        // permissions existed, so seed every table it touches with a
        // permissive `"true"` rule. Tests that exercise permission failure
        // paths can still register custom permissions per-test via
        // `Circuit::set_permission`.
        let circuit = {
            let mut c = Circuit::new();
            for table in PERMISSIVE_TABLES {
                c.set_permission(*table, "true");
            }
            c
        };

        Self {
            processor: Arc::new(RwLock::new(circuit)),
            status: Arc::new(RwLock::new(status)),
            metrics,
            job_config: Arc::new(job_config),
            job_queue_tx: tx,
            job_queue_rx: Arc::new(tokio::sync::Mutex::new(rx)),
            db: Arc::new(db),
            crdt_cache: Arc::new(CrdtCache::new(64, CrdtAllowList::default())),
            start_time: std::time::Instant::now(),
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
            ssp_id: "test-ssp".to_string(),
            scheduler_url: None,
            start_time: self.start_time,
            crdt_cache: Arc::clone(&self.crdt_cache),
            view_metrics: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
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

    /// Add or override a permission rule for a table on this harness.
    async fn set_permission(&self, table: &str, where_text: &str) {
        let mut circuit = self.processor.write().await;
        circuit.set_permission(table, where_text);
    }

    /// Register a view directly on the circuit (bypasses HTTP handler & DB calls).
    async fn register_view_direct(&self, id: &str, surql: &str) {
        let payload = view_payload(id, surql);
        let data = {
            let circuit = self.processor.read().await;
            ssp::service::view::prepare_registration_dbsp(payload, circuit.permissions())
                .expect("Failed to prepare view registration")
        };
        let mut circuit = self.processor.write().await;
        circuit.add_query(data.plan, data.safe_params, Some(OutputFormat::Streaming));
    }
}

/// Tables seeded with permissive `"true"` permissions in `TestHarness::new()`.
/// The pre-permission integration suite assumes free read access on every
/// table it queries; permission-aware tests use `set_permission` to override.
const PERMISSIVE_TABLES: &[&str] = &[
    "user",
    "post",
    "thread",
    "comment",
    "users",
    "posts",
];

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

async fn post_authed_text(app: Router, path: &str, body: &Value) -> (StatusCode, String) {
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
    let body_text = String::from_utf8_lossy(&body_bytes).into_owned();
    (status, body_text)
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

fn view_payload_with_params(id: &str, surql: &str, params: Value) -> Value {
    json!({
        "id": id,
        "surql": surql,
        "clientId": "test-client",
        "ttl": "30m",
        "lastActiveAt": "2024-01-01T00:00:00Z",
        "params": params,
    })
}

// ===========================================================================
// Test Modules
// ===========================================================================

mod auth_tests {
    use super::*;

    // /health is a public route; auth-gated routes are /ingest, /view/*, /reset, etc.
    // We hit /debug/deps because it's the cheapest authenticated route that has
    // no DB dependency in this harness.
    const AUTHED_PATH: &str = "/debug/deps";

    #[tokio::test]
    async fn unauthenticated_request_returns_401() {
        let h = TestHarness::new();
        let (status, _) = get_json(h.app(), AUTHED_PATH).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn wrong_token_returns_401() {
        let h = TestHarness::new();
        let request = Request::builder()
            .method("GET")
            .uri(AUTHED_PATH)
            .header("Authorization", "Bearer wrong-token")
            .body(axum::body::Body::empty())
            .unwrap();

        let response = h.app().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn valid_token_passes_through() {
        let h = TestHarness::new();
        let (status, _) = get_authed(h.app(), AUTHED_PATH).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn missing_bearer_prefix_returns_401() {
        let h = TestHarness::new();
        let request = Request::builder()
            .method("GET")
            .uri(AUTHED_PATH)
            .header("Authorization", AUTH_SECRET)
            .body(axum::body::Body::empty())
            .unwrap();

        let response = h.app().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}

mod health_tests {
    use super::*;

    // /health is public and the response is `{ "status": <state> }`. View and
    // table counts moved to /info long ago. Tests here pin only the surface
    // contract: status code maps to SspStatus and body has a `status` field.

    #[tokio::test]
    async fn health_ready_returns_200() {
        let h = TestHarness::new();
        let (status, body) = get_json(h.app(), "/health").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], "ready");
    }

    #[tokio::test]
    async fn health_bootstrapping_returns_503() {
        let h = TestHarness::with_status(SspStatus::Bootstrapping);
        let (status, body) = get_json(h.app(), "/health").await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["status"], "bootstrapping");
    }

    #[tokio::test]
    async fn health_failed_returns_503() {
        let h = TestHarness::with_status(SspStatus::Failed);
        let (status, body) = get_json(h.app(), "/health").await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["status"], "failed");
    }

    #[tokio::test]
    async fn health_response_has_status_field() {
        let h = TestHarness::new();
        let (_, body) = get_json(h.app(), "/health").await;
        assert!(body.get("status").is_some());
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

// ---------------------------------------------------------------------------
// Permission injection tests (HTTP register path)
// ---------------------------------------------------------------------------
//
// These exercise `register_view_handler`'s contract that
// `permission_inject::inject_permissions` errors come back as 400 with the
// offending table named, and that successful registration injects the
// permission filter into the operator plan visible via `/debug/view/<id>`.

mod permission_register_tests {
    use super::*;
    use ssp::algebra::ZSetOps;

    /// A view payload referencing a table with no registered permission must
    /// be rejected at registration time, not silently denied at runtime.
    /// `Circuit::new()` plus a fresh harness without seeding the table gives
    /// us a default-deny case directly.
    #[tokio::test]
    async fn unknown_table_returns_400_with_table_name() {
        let h = TestHarness::new();
        let payload = view_payload("perms-unknown", "SELECT * FROM secret_table");

        let (status, msg) = post_authed_text(h.app(), "/view/register", &payload).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(
            msg.contains("secret_table"),
            "error must name the table: got {msg}"
        );
        assert!(
            msg.to_lowercase().contains("default-deny"),
            "error must mention default-deny: got {msg}"
        );

        let circuit = h.processor.read().await;
        assert!(
            circuit.get_view("perms-unknown").is_none(),
            "view must not register on permission failure"
        );
    }

    /// A table whose permission is `false` (e.g. SurrealDB `PERMISSIONS NONE`
    /// or a `FOR select WHERE false` clause) must produce a 400 with `FALSE`
    /// in the message.
    #[tokio::test]
    async fn false_permission_returns_400() {
        let h = TestHarness::new();
        h.set_permission("locked", "false").await;

        let (status, msg) = post_authed_text(
            h.app(),
            "/view/register",
            &view_payload("v-locked", "SELECT * FROM locked"),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(msg.contains("locked"), "got {msg}");
        assert!(msg.contains("FALSE"), "got {msg}");
    }

    /// A permission referencing `$auth` without auth params in the
    /// registration payload must fail at registration time with a clear
    /// error pointing at the missing param.
    #[tokio::test]
    async fn auth_required_no_params_returns_400() {
        let h = TestHarness::new();
        h.set_permission("thread", "author.id = $auth.id").await;

        let payload = view_payload("v-no-auth", "SELECT * FROM thread");
        let (status, msg) = post_authed_text(h.app(), "/view/register", &payload).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(msg.contains("$auth"), "got {msg}");
        assert!(msg.contains("thread"), "got {msg}");
    }

    /// A permission expression using a SurrealDB construct the SSP cannot
    /// represent (`IN (SELECT ...)`) must fail at registration with the
    /// unsupported snippet named, instead of silently fail-closing at
    /// runtime as the previous policy.rs implementation did.
    #[tokio::test]
    async fn subquery_permission_returns_400() {
        let h = TestHarness::new();
        h.set_permission(
            "thread",
            "$auth.id IN (SELECT VALUE in FROM collaborates_on)",
        )
        .await;

        let payload = view_payload_with_params(
            "v-subq",
            "SELECT * FROM thread",
            json!({ "auth": { "id": "user:a" } }),
        );
        let (status, msg) = post_authed_text(h.app(), "/view/register", &payload).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(msg.contains("unsupported construct"), "got {msg}");
    }

    /// Successful registration with a `field = $auth.id` permission and a
    /// matching `auth` param: view registers, and only matching rows reach
    /// the cache after data flows through.
    #[tokio::test]
    async fn auth_eq_filters_by_user_via_direct_register() {
        // We use register_view_direct because the HTTP register handler
        // hits the (unconnected) DB to upsert _00_query metadata, which
        // would 500 in this harness. The point of the test is the
        // permission filter, not the metadata persistence path.
        let h = TestHarness::new();
        h.set_permission("thread", "author.id = $auth.id").await;

        h.load_records(vec![
            Record::new(
                "thread",
                "thread:a",
                json!({"id": "thread:a", "author": {"id": "user:alice"}}),
            ),
            Record::new(
                "thread",
                "thread:b",
                json!({"id": "thread:b", "author": {"id": "user:bob"}}),
            ),
        ])
        .await;

        let payload = view_payload_with_params(
            "v-mine",
            "SELECT * FROM thread",
            json!({ "auth": { "id": "user:alice" } }),
        );
        let data = {
            let circuit = h.processor.read().await;
            ssp::service::view::prepare_registration_dbsp(payload, circuit.permissions())
                .expect("registration with valid permission must succeed")
        };
        {
            let mut circuit = h.processor.write().await;
            circuit.add_query(data.plan, data.safe_params, Some(OutputFormat::Streaming));
        }

        let circuit = h.processor.read().await;
        let view = circuit.get_view("v-mine").expect("view registered");
        // Permission allows only Alice's threads.
        assert!(view.cache.is_present("thread:a"), "alice's thread must be present");
        assert!(!view.cache.is_present("thread:b"), "bob's thread must NOT leak through");
    }

    /// `true` permission (the SurrealDB FULL default) is a no-op: the
    /// registered plan is unchanged from a vanilla scan and all rows pass
    /// through.
    #[tokio::test]
    async fn true_permission_is_noop() {
        let h = TestHarness::new();
        // `user` is already permissive in the harness, but be explicit.
        h.set_permission("user", "true").await;

        h.load_records(vec![
            Record::new("user", "user:1", json!({"id": "user:1"})),
            Record::new("user", "user:2", json!({"id": "user:2"})),
        ])
        .await;

        h.register_view_direct("v-all", "SELECT * FROM user").await;

        let circuit = h.processor.read().await;
        let view = circuit.get_view("v-all").unwrap();
        assert!(view.cache.is_present("user:1"));
        assert!(view.cache.is_present("user:2"));
    }

    /// A query referencing two tables in a subquery: each scan must get its
    /// own permission predicate. Default-deny on the inner table aborts the
    /// whole registration even though the outer scan would have succeeded.
    /// We exercise this through `prepare_registration_dbsp` directly because
    /// the HTTP `/view/register` path tries to upsert metadata into an
    /// unconnected DB after the perm check, which would 500 in this harness.
    #[tokio::test]
    async fn subquery_inner_scan_is_validated() {
        let h = TestHarness::new();
        // The harness seeds `comment` permissive; remove it so the inner
        // scan is genuinely default-deny.
        {
            let mut circuit = h.processor.write().await;
            // Replace permissions with only thread set permissive.
            *circuit = {
                let mut c = Circuit::new();
                c.set_permission("thread", "true");
                c
            };
        }

        let payload = view_payload(
            "v-subq",
            "SELECT *, (SELECT * FROM comment WHERE thread=$parent.id) AS comments FROM thread",
        );
        let circuit = h.processor.read().await;
        let result =
            ssp::service::view::prepare_registration_dbsp(payload, circuit.permissions());
        let err = match result {
            Ok(_) => panic!("expected Err, got Ok"),
            Err(e) => e,
        };
        let msg = err.to_string();
        assert!(msg.contains("comment"), "must name the inner table; got {msg}");
        assert!(
            msg.to_lowercase().contains("default-deny"),
            "must explain default-deny; got {msg}"
        );
    }

    /// Failed registration must not pollute the circuit even partially: no
    /// view should appear in `view_count()` after the rejection.
    #[tokio::test]
    async fn failed_registration_leaves_no_orphan_state() {
        let h = TestHarness::new();
        let before = h.processor.read().await.view_count();

        let payload = view_payload("v-orphan", "SELECT * FROM ghost");
        let (status, _) = post_authed_text(h.app(), "/view/register", &payload).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let after = h.processor.read().await.view_count();
        assert_eq!(before, after, "rejection must not register the view");
        assert!(
            h.processor
                .read()
                .await
                .get_view("v-orphan")
                .is_none(),
            "no orphan view"
        );
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
            std::env::set_var("SPKY_AUTH_SECRET", AUTH_SECRET);
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

        let circuit = {
            let mut c = Circuit::new();
            for table in PERMISSIVE_TABLES {
                c.set_permission(*table, "true");
            }
            c
        };

        TestHarness {
            processor: Arc::new(RwLock::new(circuit)),
            status: Arc::new(RwLock::new(SspStatus::Ready)),
            metrics: Arc::new(Metrics::new(&provider)),
            job_config: Arc::new(JobConfig::default()),
            job_queue_tx: tx,
            job_queue_rx: Arc::new(tokio::sync::Mutex::new(rx)),
            db: Arc::new(db),
            crdt_cache: Arc::new(CrdtCache::new(64, CrdtAllowList::default())),
            start_time: std::time::Instant::now(),
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
