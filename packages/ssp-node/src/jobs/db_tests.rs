//! Close-to-e2e tests for the job engine THROUGH THE PORTS: a real embedded
//! SurrealDB behind a `Db` adapter, a real mock HTTP backend behind an
//! `HttpClient` adapter, driving the actual runner + re-arm SQL. Migrated
//! from the former `packages/job-runner` — same coverage, but every DB write
//! now travels the same `type::record($id)` SQL the production port uses.

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
async fn insert_job(db: &Surreal<MemEngine>, id_part: &str, status: &str, recurring: bool, interval_ms: i64) {
    db.query(
        "CREATE type::record('job', $id) SET \
         status = $status, recurring = $recurring, interval = $interval, \
         retries = 3, max_retries = 3, path = '/run', payload = {}, \
         retry_strategy = 'linear', errors = [], \
         created_at = time::now(), updated_at = time::now()",
    )
    .bind(("id", id_part.to_string()))
    .bind(("status", status.to_string()))
    .bind(("recurring", recurring))
    .bind(("interval", interval_ms))
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

fn job_entry(id: &str, base_url: String, recurring: bool, interval_ms: u64, max_retries: u32) -> JobEntry {
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
        recurring,
        interval_ms,
    }
}

// --- helper: rearm_recurring_helper ------------------------------------

#[tokio::test]
async fn rearm_advances_next_run_and_resets_to_pending() {
    let (port, raw) = mem_db().await;
    insert_job(&raw, "r1", "processing", true, 300_000).await;

    rearm_recurring_helper(port.as_ref(), "job:r1", 300_000).await.unwrap();

    assert_eq!(
        select_string(&raw, "SELECT VALUE status FROM ONLY job:r1").await.as_deref(),
        Some("pending"),
        "re-armed row returns to pending, not success/failed"
    );
    assert_eq!(
        select_i64(&raw, "SELECT VALUE retries FROM ONLY job:r1").await,
        Some(0),
        "retries reset for the next cycle"
    );
    assert_eq!(
        select_bool(&raw, "SELECT VALUE next_run_at > time::now() FROM ONLY job:r1").await,
        Some(true),
        "next_run_at pushed into the future by ~interval"
    );
}

#[tokio::test]
async fn rearm_releases_the_assignee_claim() {
    // The claim marker means "this SSP holds the job in-memory right now".
    // A re-armed row waits for the recovery sweep — keeping the previous
    // run's assignee makes the sweep's is_orphaned check skip the row while
    // that SSP lives, so the next recurring run would never dispatch.
    let (port, raw) = mem_db().await;
    insert_job(&raw, "r3", "processing", true, 300_000).await;
    raw.query("UPDATE job:r3 SET assignee = 'ssp-0'").await.expect("stamp assignee");

    rearm_recurring_helper(port.as_ref(), "job:r3", 300_000).await.unwrap();

    assert_eq!(
        select_bool(&raw, "SELECT VALUE assignee = NONE FROM ONLY job:r3").await,
        Some(true),
        "re-arm must clear the claim so the sweep dispatches the next run"
    );
}

#[tokio::test]
async fn rearm_is_a_noop_when_not_processing() {
    // Guard: never clobber a row that isn't the one this runner just ran
    // (e.g. an operator killed it). Only a `processing` row is re-armed.
    let (port, raw) = mem_db().await;
    insert_job(&raw, "r2", "pending", true, 300_000).await;

    rearm_recurring_helper(port.as_ref(), "job:r2", 300_000).await.unwrap();

    assert_eq!(
        select_bool(&raw, "SELECT VALUE next_run_at != NONE FROM ONLY job:r2").await,
        Some(false),
        "guarded WHERE status='processing' left the pending row untouched"
    );
}

// --- helper: fail_if_pending through the port ---------------------------

#[tokio::test]
async fn fail_if_pending_updates_only_pending_rows() {
    let (port, raw) = mem_db().await;
    insert_job(&raw, "k1", "pending", false, 0).await;
    insert_job(&raw, "k2", "processing", false, 0).await;

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

// --- full runner cycle: recurring success re-arms ----------------------

#[tokio::test]
async fn recurring_success_rearms_instead_of_terminalizing() {
    let backend = MockServer::start().await;
    Mock::given(method("POST"))
        .and(match_path("/run"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&backend)
        .await;

    let (port, raw) = mem_db().await;
    insert_job(&raw, "run1", "pending", true, 300_000).await;
    let runner = make_runner(Arc::clone(&port));

    runner
        .execute_job(job_entry("job:run1", backend.uri(), true, 300_000, 3))
        .await
        .unwrap();

    assert_eq!(backend.received_requests().await.unwrap().len(), 1, "backend was called");
    assert_eq!(
        select_string(&raw, "SELECT VALUE status FROM ONLY job:run1").await.as_deref(),
        Some("pending"),
        "recurring row never reaches terminal success"
    );
    assert_eq!(
        select_bool(&raw, "SELECT VALUE next_run_at > time::now() FROM ONLY job:run1").await,
        Some(true),
        "clock re-armed to the future after completion"
    );
}

// --- full runner cycle: one-shot still terminalizes (regression) -------

#[tokio::test]
async fn one_shot_success_terminalizes() {
    let backend = MockServer::start().await;
    Mock::given(method("POST"))
        .and(match_path("/run"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&backend)
        .await;

    let (port, raw) = mem_db().await;
    insert_job(&raw, "run2", "pending", false, 0).await;
    let runner = make_runner(Arc::clone(&port));

    runner
        .execute_job(job_entry("job:run2", backend.uri(), false, 0, 3))
        .await
        .unwrap();

    assert_eq!(
        select_string(&raw, "SELECT VALUE status FROM ONLY job:run2").await.as_deref(),
        Some("success"),
        "one-shot job terminalizes as before"
    );
}

// --- full runner cycle: recurring failure re-arms (survives outage) ----

#[tokio::test]
async fn recurring_failure_rearms_instead_of_failing() {
    let backend = MockServer::start().await;
    Mock::given(method("POST"))
        .and(match_path("/run"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&backend)
        .await;

    let (port, raw) = mem_db().await;
    insert_job(&raw, "run3", "pending", true, 300_000).await;
    let runner = make_runner(Arc::clone(&port));

    // max_retries=0 => exhausts immediately (no backoff sleep), hits the
    // terminal branch, which for a recurring job re-arms rather than fails.
    runner
        .execute_job(job_entry("job:run3", backend.uri(), true, 300_000, 0))
        .await
        .unwrap();

    assert_eq!(
        select_string(&raw, "SELECT VALUE status FROM ONLY job:run3").await.as_deref(),
        Some("pending"),
        "a backend outage must not kill the schedule"
    );
    assert_eq!(
        select_bool(&raw, "SELECT VALUE next_run_at > time::now() FROM ONLY job:run3").await,
        Some(true),
        "re-armed for the next interval despite the failure"
    );
}

// --- the shared due-clause selects due rows and skips future ones -------

#[tokio::test]
async fn pending_due_clause_selects_due_and_skips_future() {
    let (_, raw) = mem_db().await;
    // due recurring (next_run_at in the past)
    raw.query("CREATE job:due SET status='pending', recurring=true, interval=300000, next_run_at = time::now() - 1s, created_at = time::now(), delay = 0").await.unwrap();
    // not-yet-due recurring (next_run_at in the future)
    raw.query("CREATE job:future SET status='pending', recurring=true, interval=300000, next_run_at = time::now() + 1h, created_at = time::now(), delay = 0").await.unwrap();
    // one-shot still inside its delay window (created now + 1h delay)
    raw.query("CREATE job:delayed SET status='pending', created_at = time::now(), delay = 3600000").await.unwrap();
    // one-shot ready (no delay, no next_run_at)
    raw.query("CREATE job:ready SET status='pending', created_at = time::now() - 1s, delay = 0").await.unwrap();

    let sql = format!(
        "SELECT VALUE type::string(id) FROM job WHERE status = 'pending' AND {} ORDER BY id",
        PENDING_DUE_CLAUSE
    );
    let ids: Vec<String> = raw.query(&sql).await.unwrap().take(0).unwrap();

    assert!(ids.contains(&"job:due".to_string()), "due recurring selected");
    assert!(ids.contains(&"job:ready".to_string()), "ready one-shot selected");
    assert!(!ids.contains(&"job:future".to_string()), "future recurring skipped");
    assert!(!ids.contains(&"job:delayed".to_string()), "delayed one-shot skipped");
}

// --- the SSP poke due-check SQL (poke=now => run; re-arm=future => skip) -

#[tokio::test]
async fn poke_due_sql_true_for_past_false_for_future() {
    let (port, _raw) = mem_db().await;

    let past = port
        .query(POKE_DUE_SQL, &[("nra", json!("2000-01-01T00:00:00Z"))])
        .await
        .unwrap();
    assert_eq!(past.first().and_then(|v| v.as_bool()), Some(true), "a poke (past) is due -> runs");

    let future = port
        .query(POKE_DUE_SQL, &[("nra", json!("2999-01-01T00:00:00Z"))])
        .await
        .unwrap();
    assert_eq!(
        future.first().and_then(|v| v.as_bool()),
        Some(false),
        "a re-arm (future) is not due -> no busy loop"
    );
}
