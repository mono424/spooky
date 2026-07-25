//! Close-to-e2e tests for the job engine THROUGH THE PORTS: a real embedded
//! SurrealDB behind a `Db` adapter, a real mock HTTP backend behind an
//! `HttpClient` adapter, driving the actual runner SQL. Migrated from the former
//! `packages/job-runner` — same coverage, but every DB write now travels the
//! same `type::record($id)` SQL the production port uses.

use super::runner::*;
use super::types::{JobControl, JobEntry};
use crate::api::Method;
use crate::ports::{
    CancelWatch, Db, DbError, HttpClient, HttpError, OutboundRequest, OutboundResponse,
    Scheduler, Spawner, TimerKind,
};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use surrealdb::engine::local::{Db as MemEngine, Mem};
use surrealdb::Surreal;
use tokio::sync::mpsc;
use wiremock::matchers::{method, path as match_path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// --- Port adapters over test infrastructure ---------------------------------

struct MemDb(Arc<Surreal<MemEngine>>);

#[async_trait::async_trait]
impl Db for MemDb {
    async fn query(
        &self,
        surql: &str,
        binds: &[(&str, Value)],
    ) -> Result<Vec<Value>, DbError> {
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
        Ok("mem".to_string())
    }
}

struct TestHttp(reqwest::Client);

#[async_trait::async_trait]
impl HttpClient for TestHttp {
    async fn send(
        &self,
        req: OutboundRequest,
        _cancel: Option<CancelWatch>,
    ) -> Result<OutboundResponse, HttpError> {
        let mut builder = match req.method {
            Method::Post => self.0.post(&req.url),
            _ => unimplemented!("job dispatch is POST-only"),
        }
        .timeout(req.timeout);
        if let Some(b) = &req.bearer {
            builder = builder.bearer_auth(b);
        }
        if let Some(j) = &req.json_body {
            builder = builder.json(j);
        }
        match builder.send().await {
            Ok(r) => Ok(OutboundResponse {
                status: r.status().as_u16(),
                body: r.text().await.unwrap_or_default(),
            }),
            Err(e) if e.is_timeout() => Err(HttpError::Timeout(req.timeout)),
            Err(e) => Err(HttpError::Transport(e.to_string())),
        }
    }
}

struct TestScheduler;

#[async_trait::async_trait]
impl Scheduler for TestScheduler {
    async fn schedule(&self, _kind: TimerKind, _at: u64) {}
    async fn cancel(&self, _kind: &TimerKind) {}
    async fn sleep(&self, dur: Duration) {
        tokio::time::sleep(dur).await;
    }
}

struct TestSpawner;
impl Spawner for TestSpawner {
    fn spawn(&self, fut: crate::ports::LocalBoxFuture) {
        tokio::spawn(fut);
    }
}

// --- Harness -----------------------------------------------------------------

async fn mem_db() -> (Arc<dyn Db>, Arc<Surreal<MemEngine>>) {
    let db = Surreal::new::<Mem>(()).await.expect("start mem db");
    db.use_ns("test").use_db("test").await.expect("use ns/db");
    let raw = Arc::new(db);
    (Arc::new(MemDb(Arc::clone(&raw))), raw)
}

/// Insert a job row. `id_part` is the id after `job:`.
async fn insert_job(db: &Surreal<MemEngine>, id_part: &str, status: &str) {
    db.query(
        "CREATE type::record('job', $id) SET \
         status = $status, retries = 3, max_retries = 3, path = '/run', payload = {}, \
         retry_strategy = 'linear', errors = [], \
         created_at = time::now(), updated_at = time::now()",
    )
    .bind(("id", id_part.to_string()))
    .bind(("status", status.to_string()))
    .await
    .expect("insert job");
}

async fn select_string(db: &Surreal<MemEngine>, sql: &str) -> Option<String> {
    db.query(sql).await.expect("query").take(0).expect("take")
}
async fn select_bool(db: &Surreal<MemEngine>, sql: &str) -> Option<bool> {
    db.query(sql).await.expect("query").take(0).expect("take")
}
async fn select_i64(db: &Surreal<MemEngine>, sql: &str) -> Option<i64> {
    db.query(sql).await.expect("query").take(0).expect("take")
}

fn make_runner(db: Arc<dyn Db>) -> JobRunner {
    let (tx, rx) = mpsc::channel::<JobEntry>(16);
    JobRunner::new(
        rx,
        tx,
        db,
        Arc::new(TestHttp(reqwest::Client::new())),
        Arc::new(TestScheduler),
        Arc::new(TestSpawner),
        JobControl::new(),
    )
}

fn job_entry(id: &str, base_url: String, max_retries: u32) -> JobEntry {
    JobEntry {
        id: id.to_string(),
        base_url,
        path: "/run".to_string(),
        payload: Value::Null,
        retries: 0,
        max_retries,
        retry_strategy: "linear".to_string(),
        auth_token: None,
        timeout: Duration::from_secs(5),
    }
}

// --- helper: fail_if_pending through the port ---------------------------

#[tokio::test]
async fn fail_if_pending_updates_only_pending_rows() {
    let (port, raw) = mem_db().await;
    insert_job(&raw, "k1", "pending").await;
    insert_job(&raw, "k2", "processing").await;

    let killed = fail_if_pending_helper(port.as_ref(), "job:k1", json!({"code": "killed"}))
        .await
        .unwrap();
    assert!(killed, "pending row was terminalized");
    assert_eq!(
        select_string(&raw, "SELECT VALUE status FROM ONLY job:k1").await.as_deref(),
        Some("failed")
    );

    let killed = fail_if_pending_helper(port.as_ref(), "job:k2", json!({"code": "killed"}))
        .await
        .unwrap();
    assert!(!killed, "processing row must not be clobbered");
    assert_eq!(
        select_string(&raw, "SELECT VALUE status FROM ONLY job:k2").await.as_deref(),
        Some("processing")
    );
}

// --- result capture -----------------------------------------------------

#[tokio::test]
async fn success_captures_the_backend_response_as_json() {
    let backend = MockServer::start().await;
    Mock::given(method("POST"))
        .and(match_path("/run"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"fileId": "f_8123"})))
        .mount(&backend)
        .await;

    let (port, raw) = mem_db().await;
    insert_job(&raw, "res1", "pending").await;
    let runner = make_runner(Arc::clone(&port));

    runner
        .execute_job(job_entry("job:res1", backend.uri(), 3))
        .await
        .unwrap();

    assert_eq!(
        select_string(&raw, "SELECT VALUE status FROM ONLY job:res1").await.as_deref(),
        Some("success")
    );
    assert_eq!(
        select_string(&raw, "SELECT VALUE result.fileId FROM ONLY job:res1").await.as_deref(),
        Some("f_8123"),
        "a JSON body is stored parsed, so a dependent workflow step can address its fields"
    );
}

#[tokio::test]
async fn success_stores_a_non_json_body_as_a_string() {
    let backend = MockServer::start().await;
    Mock::given(method("POST"))
        .and(match_path("/run"))
        .respond_with(ResponseTemplate::new(200).set_body_string("done"))
        .mount(&backend)
        .await;

    let (port, raw) = mem_db().await;
    insert_job(&raw, "res2", "pending").await;
    let runner = make_runner(Arc::clone(&port));

    runner
        .execute_job(job_entry("job:res2", backend.uri(), 3))
        .await
        .unwrap();

    assert_eq!(
        select_string(&raw, "SELECT VALUE result FROM ONLY job:res2").await.as_deref(),
        Some("done")
    );
}

#[tokio::test]
async fn oversized_results_are_replaced_by_a_marker() {
    let backend = MockServer::start().await;
    Mock::given(method("POST"))
        .and(match_path("/run"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string("x".repeat(JOB_RESULT_MAX_BYTES + 1)),
        )
        .mount(&backend)
        .await;

    let (port, raw) = mem_db().await;
    insert_job(&raw, "res3", "pending").await;
    let runner = make_runner(Arc::clone(&port));

    runner
        .execute_job(job_entry("job:res3", backend.uri(), 3))
        .await
        .unwrap();

    assert_eq!(
        select_string(&raw, "SELECT VALUE status FROM ONLY job:res3").await.as_deref(),
        Some("success"),
        "an oversized body must not stop the job from completing"
    );
    assert_eq!(
        select_bool(&raw, "SELECT VALUE result.truncated FROM ONLY job:res3").await,
        Some(true),
        "the operator can see WHY the output is missing"
    );
}

#[tokio::test]
async fn success_still_terminalizes_when_the_table_has_no_result_field() {
    // A project that upgraded the stack without re-applying its schema has a
    // SCHEMAFULL outbox table with no `result` field. SurrealDB rejects the
    // whole UPDATE — completing the job matters far more than storing output.
    let backend = MockServer::start().await;
    Mock::given(method("POST"))
        .and(match_path("/run"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .mount(&backend)
        .await;

    let (port, raw) = mem_db().await;
    raw.query(
        "DEFINE TABLE job SCHEMAFULL; \
         DEFINE FIELD status ON job TYPE string; \
         DEFINE FIELD path ON job TYPE option<string>; \
         DEFINE FIELD payload ON job TYPE option<object> FLEXIBLE; \
         DEFINE FIELD retries ON job TYPE option<int>; \
         DEFINE FIELD max_retries ON job TYPE option<int>; \
         DEFINE FIELD retry_strategy ON job TYPE option<string>; \
         DEFINE FIELD errors ON job TYPE option<array>; \
         DEFINE FIELD created_at ON job TYPE option<datetime>; \
         DEFINE FIELD updated_at ON job TYPE option<datetime>;",
    )
    .await
    .expect("define schemafull job table without a result field");
    insert_job(&raw, "res4", "pending").await;
    let runner = make_runner(Arc::clone(&port));

    runner
        .execute_job(job_entry("job:res4", backend.uri(), 3))
        .await
        .expect("job completion must not fail on a missing result field");

    assert_eq!(
        select_string(&raw, "SELECT VALUE status FROM ONLY job:res4").await.as_deref(),
        Some("success"),
        "fell back to a status-only write"
    );
}

// --- the shared due-clause selects due rows and skips future ones -------

#[tokio::test]
async fn pending_due_clause_selects_due_and_skips_delayed() {
    let (_, raw) = mem_db().await;
    // Ready: no delay.
    raw.query("CREATE job:ready SET status='pending', created_at = time::now() - 1s, delay = 0")
        .await
        .unwrap();
    // Ready: its delay window has elapsed.
    raw.query("CREATE job:elapsed SET status='pending', created_at = time::now() - 10s, delay = 1000")
        .await
        .unwrap();
    // Still inside its delay window.
    raw.query("CREATE job:delayed SET status='pending', created_at = time::now(), delay = 3600000")
        .await
        .unwrap();
    // No delay field at all — the `?? 0` fallback makes it ready.
    raw.query("CREATE job:nodelay SET status='pending', created_at = time::now() - 1s")
        .await
        .unwrap();

    let sql = format!(
        "SELECT VALUE type::string(id) FROM job WHERE status = 'pending' AND {} ORDER BY id",
        PENDING_DUE_CLAUSE
    );
    let ids: Vec<String> = raw.query(&sql).await.unwrap().take(0).unwrap();

    assert!(ids.contains(&"job:ready".to_string()));
    assert!(ids.contains(&"job:elapsed".to_string()));
    assert!(ids.contains(&"job:nodelay".to_string()), "an unset delay means ready now");
    assert!(!ids.contains(&"job:delayed".to_string()), "a job inside its delay window waits");
}
