use crate::types::{JobControl, JobEntry};
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use surrealdb::types::RecordId;
use surrealdb::{Connection, Surreal};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

/// Result of awaiting a single job's HTTP request, distinguishing an operator
/// cancellation from a normal response so the loop can pick exactly one terminal
/// status write.
enum Outcome {
    Cancelled,
    Responded(reqwest::Result<reqwest::Response>),
}

/// SQL boolean expression (for recovery-sweep `WHERE` clauses): is a pending job
/// DUE now? A recurring schedule is due at `next_run_at`; a one-shot job (no
/// `next_run_at`) is due at `created_at + <delay>`. `??` falls back when
/// next_run_at/delay are unset (NONE); `delay = 0` ⇒ ready immediately. Shared by
/// the SSP singlenode sweep and the cluster scheduler sweep so the two never
/// drift, and exercised directly by tests.
pub const PENDING_DUE_CLAUSE: &str =
    "(next_run_at ?? (created_at + <duration>(string::concat(<string>(delay ?? 0), 'ms')))) <= time::now()";

/// SQL the SSP poke path uses to decide whether a recurring row is DUE now, from
/// the `next_run_at` value CARRIED in the ingest event (bound as `$nra`) rather
/// than a row read: the `_00_*_mutation` event's `http::post` fires inside the
/// writer's transaction, so reading the row back can see the pre-update value.
/// Comparing the carried value distinguishes a poke (now) from the runner's
/// re-arm (a future time). Returns a bool. Shared so tests hit the exact string.
pub const POKE_DUE_SQL: &str = "RETURN <datetime>$nra <= time::now()";

pub struct JobRunner<C: Connection> {
    queue_rx: mpsc::Receiver<JobEntry>,
    queue_tx: mpsc::Sender<JobEntry>,
    db: Arc<Surreal<C>>,
    http_client: reqwest::Client,
    job_control: JobControl,
}

impl<C: Connection> JobRunner<C> {
    pub fn new(
        queue_rx: mpsc::Receiver<JobEntry>,
        queue_tx: mpsc::Sender<JobEntry>,
        db: Arc<Surreal<C>>,
        job_control: JobControl,
    ) -> Self {
        let http_client = reqwest::Client::builder()
            .build()
            .expect("Failed to create HTTP client");

        Self {
            queue_rx,
            queue_tx,
            db,
            http_client,
            job_control,
        }
    }

    /// Run the job runner loop
    pub async fn run(mut self) {
        info!("Job runner started");

        while let Some(job) = self.queue_rx.recv().await {
            debug!(job_id = %job.id, path = %job.path, "Processing job");

            if let Err(e) = self.execute_job(job).await {
                error!(error = %e, "Error executing job");
            }
        }

        info!("Job runner stopped");
    }

    /// Execute a single job
    async fn execute_job(&self, job: JobEntry) -> Result<()> {
        // Killed-while-pending: an operator called /job/kill before this job was
        // dequeued. Fail it terminally without ever firing the request. The runner
        // is the sole writer of `status`, so doing the write here (rather than in
        // the kill handler) avoids clobbering a job that may have just flipped to
        // 'processing'.
        if self.job_control.killed_pending.remove(&job.id).is_some() {
            info!(job_id = %job.id, "Job killed while pending — failing without execution");
            let error_entry = json!({ "code": "killed", "reason": "killed by operator" });
            self.append_error(&job.id, error_entry).await.ok();
            self.update_status(&job.id, "failed").await?;
            self.job_control.clear_enqueued(&job.id);
            return Ok(());
        }

        // Update status to "processing"
        self.update_status(&job.id, "processing").await?;

        // Build URL
        let url = format!("{}{}", job.base_url, job.path);

        debug!(job_id = %job.id, url = %url, "Sending HTTP request");

        // Parse payload if it's a string containing JSON
        let payload = match &job.payload {
            serde_json::Value::String(s) => {
                serde_json::from_str(s).unwrap_or_else(|_| job.payload.clone())
            }
            _ => job.payload.clone(),
        };

        // Execute HTTP request with per-job timeout
        let mut request = self
            .http_client
            .post(&url)
            .timeout(job.timeout)
            .json(&payload);

        // Add authorization header if token is present
        if let Some(ref token) = job.auth_token {
            request = request.header("Authorization", format!("Bearer {}", token));
        }

        // Register a cancellation token for the duration of the in-flight request
        // so `/job/kill` on a 'processing' job can abort it. `biased` below makes
        // the select poll the cancellation branch first, so a token that already
        // fired wins ties against a response that completed in the same poll.
        let cancel = CancellationToken::new();
        self.job_control
            .inflight
            .insert(job.id.clone(), cancel.clone());

        let send_fut = request.send();
        tokio::pin!(send_fut);

        let outcome = tokio::select! {
            biased;
            _ = cancel.cancelled() => Outcome::Cancelled,
            res = &mut send_fut => Outcome::Responded(res),
        };

        // Always release the token. The runner is the single consumer, so only one
        // execution per job_id is ever in-flight; an unconditional remove cannot
        // drop a different (e.g. retried) execution's token.
        self.job_control.inflight.remove(&job.id);

        match outcome {
            Outcome::Cancelled => {
                info!(job_id = %job.id, "Job cancelled by operator — marking failed");
                let error_entry = json!({ "code": "cancelled", "reason": "killed by operator" });
                self.append_error(&job.id, error_entry).await.ok();
                // An operator kill is terminal: do NOT run handle_failure, so the
                // backoff-retry path never re-queues a job the operator killed.
                self.update_status(&job.id, "failed").await?;
                self.job_control.clear_enqueued(&job.id);
            }
            Outcome::Responded(Ok(response)) if response.status().is_success() => {
                info!(job_id = %job.id, status = %response.status(), "Job completed successfully");
                if job.recurring {
                    // Recurring schedule: never terminalize. Re-arm the clock from
                    // THIS completion (drift-free) and return to `pending` so the
                    // recovery sweep dispatches the next run when the interval elapses.
                    self.rearm_recurring(&job.id, job.interval_ms).await?;
                } else {
                    self.update_status(&job.id, "success").await?;
                }
                self.job_control.clear_enqueued(&job.id);
            }
            Outcome::Responded(Ok(response)) => {
                let status = response.status();
                let error_body = response
                    .text()
                    .await
                    .unwrap_or_else(|_| "Failed to read response body".to_string());
                warn!(
                    job_id = %job.id,
                    status = %status,
                    error_body = %error_body,
                    "Job request failed with non-success status"
                );

                // Create error entry with code and reason
                let error_entry = json!({
                    "code": status.as_u16(),
                    "reason": error_body
                });

                self.handle_failure(job, Some(error_entry)).await?;
            }
            Outcome::Responded(Err(e)) => {
                warn!(job_id = %job.id, error = %e, "Job request failed");

                // Create error entry for request error
                let error_entry = json!({
                    "code": 0,
                    "reason": e.to_string()
                });

                self.handle_failure(job, Some(error_entry)).await?;
            }
        }

        Ok(())
    }

    /// Handle job failure - retry or mark as failed
    async fn handle_failure(&self, mut job: JobEntry, error_entry: Option<serde_json::Value>) -> Result<()> {
        job.retries += 1;

        // Append error to database if provided
        if let Some(error) = error_entry {
            self.append_error(&job.id, error).await?;
        }

        // Persist the incremented attempt count regardless of outcome, so a job
        // that exhausts its budget ends at retries == max_retries. The terminal
        // branch below used to skip this, leaving it one short (e.g. 2/3 on a job
        // that actually used all its attempts).
        self.increment_retries(&job.id).await?;

        if job.retries < job.max_retries {
            // Calculate delay based on retry strategy
            let delay = calculate_delay(job.retries, &job.retry_strategy);

            info!(
                job_id = %job.id,
                retries = job.retries,
                max_retries = job.max_retries,
                delay_ms = delay.as_millis(),
                "Job will be retried"
            );

            // Requeue with delay
            let queue_tx = self.queue_tx.clone();
            let db = self.db.clone();
            let job_id = job.id.clone();
            tokio::spawn(async move {
                tokio::time::sleep(delay).await;

                // Update status to pending before re-queueing
                if let Err(e) = update_status_helper(&db, &job_id, "pending").await {
                    error!(job_id = %job_id, error = %e, "Failed to update status for retry");
                    return;
                }

                // Re-queue the job
                if let Err(e) = queue_tx.send(job).await {
                    error!(job_id = %job_id, error = %e, "Failed to re-queue job");
                }
            });
        } else if job.recurring {
            // A recurring schedule must survive a transient backend outage: it
            // exhausts its per-cycle retry budget, then re-arms for the next
            // interval instead of dying `failed`. Errors stay on the row for
            // visibility; retries reset for the next cycle.
            warn!(
                job_id = %job.id,
                retries = job.retries,
                "Recurring job exhausted retries this cycle - re-arming for next interval"
            );
            self.rearm_recurring(&job.id, job.interval_ms).await?;
            self.job_control.clear_enqueued(&job.id);
        } else {
            warn!(
                job_id = %job.id,
                retries = job.retries,
                "Job exceeded max retries - marking as failed"
            );
            self.update_status(&job.id, "failed").await?;
            // Terminal: the retry branch above deliberately keeps the enqueued
            // mark (the job stays in flight across the backoff), but here the
            // job is done, so release it for any future re-enqueue.
            self.job_control.clear_enqueued(&job.id);
        }

        Ok(())
    }

    /// Append error to the errors array in database
    async fn append_error(&self, job_id: &str, error: serde_json::Value) -> Result<()> {
        append_error_helper(&self.db, job_id, error).await
    }

    /// Update job status in database
    async fn update_status(&self, job_id: &str, status: &str) -> Result<()> {
        update_status_helper(&self.db, job_id, status).await
    }

    /// Re-arm a recurring job after a run completes (see `rearm_recurring_helper`).
    async fn rearm_recurring(&self, job_id: &str, interval_ms: u64) -> Result<()> {
        rearm_recurring_helper(&self.db, job_id, interval_ms).await
    }

    /// Increment retry count in database
    async fn increment_retries(&self, job_id: &str) -> Result<()> {
        let record_id = RecordId::parse_simple(job_id)
            .context(format!("Invalid job ID: {}", job_id))?;

        self.db
            .query("UPDATE $id SET retries = retries + 1, updated_at = time::now()")
            .bind(("id", record_id))
            .await
            .context("Failed to increment retries")?;

        Ok(())
    }
}

/// Helper function to update status (used by both JobRunner and spawned tasks)
pub async fn update_status_helper<C: Connection>(
    db: &Surreal<C>,
    job_id: &str,
    status: &str,
) -> Result<()> {
    let record_id = RecordId::parse_simple(job_id)
        .context(format!("Invalid job ID: {}", job_id))?;

    db.query("UPDATE $id SET status = $status, updated_at = time::now()")
        .bind(("id", record_id))
        .bind(("status", status.to_string()))
        .await
        .context("Failed to update status")?;

    Ok(())
}

/// Persist the owning SSP node id (`ssp_id`) onto a job row so cluster recovery
/// can tell which SSP accepted a job (and re-dispatch only if that SSP dies).
/// Deliberately does **not** touch `updated_at`: the recovery staleness clock
/// must keep measuring from the row's last real status change, not from this
/// ownership stamp (which happens right after create). Written server-side
/// (root) only.
pub async fn set_assignee_helper<C: Connection>(
    db: &Surreal<C>,
    job_id: &str,
    assignee: &str,
) -> Result<()> {
    let record_id = RecordId::parse_simple(job_id)
        .context(format!("Invalid job ID: {}", job_id))?;

    db.query("UPDATE $id SET assignee = $assignee RETURN NONE")
        .bind(("id", record_id))
        .bind(("assignee", assignee.to_string()))
        .await
        .context("Failed to set job assignee")?;

    Ok(())
}

/// Append an error object to a job's `errors` array. Shared by the runner and the
/// SSP's `/job/kill`/`/job/retry` handlers so every writer uses identical SQL.
pub async fn append_error_helper<C: Connection>(
    db: &Surreal<C>,
    job_id: &str,
    error: Value,
) -> Result<()> {
    let record_id = RecordId::parse_simple(job_id)
        .context(format!("Invalid job ID: {}", job_id))?;

    db.query("UPDATE $id SET errors = array::append(errors, $error), updated_at = time::now()")
        .bind(("id", record_id))
        .bind(("error", error))
        .await
        .context("Failed to append error")?;

    Ok(())
}

/// Re-arm a recurring job after a run completes: set `next_run_at = now + interval`,
/// reset `retries`, and return the row to `pending` so the recovery sweep dispatches
/// the next run once the interval elapses. A recurring row therefore never reaches a
/// terminal `success`/`failed` state; it cycles pending -> processing -> pending.
/// Guarded on `status = 'processing'` (the state the runner set before the run) to
/// preserve the single-writer invariant — it never clobbers a row an operator has
/// killed. Written server-side (root) only.
pub async fn rearm_recurring_helper<C: Connection>(
    db: &Surreal<C>,
    job_id: &str,
    interval_ms: u64,
) -> Result<()> {
    let record_id = RecordId::parse_simple(job_id)
        .context(format!("Invalid job ID: {}", job_id))?;

    db.query(
        "UPDATE $id SET status = 'pending', retries = 0, \
         next_run_at = time::now() + <duration>(string::concat(<string>$interval, 'ms')), \
         updated_at = time::now() WHERE status = 'processing' RETURN NONE",
    )
    .bind(("id", record_id))
    .bind(("interval", interval_ms))
    .await
    .context("Failed to re-arm recurring job")?;

    Ok(())
}

/// Reset a terminal job for re-execution: `status='pending'`, `retries=0`, errors
/// cleared. Used by the SSP `/job/retry` handler before re-enqueueing the job.
pub async fn reset_for_retry_helper<C: Connection>(db: &Surreal<C>, job_id: &str) -> Result<()> {
    let record_id = RecordId::parse_simple(job_id)
        .context(format!("Invalid job ID: {}", job_id))?;

    db.query("UPDATE $id SET status = 'pending', retries = 0, errors = [], updated_at = time::now()")
        .bind(("id", record_id))
        .await
        .context("Failed to reset job for retry")?;

    Ok(())
}

/// Atomically fail a job **only if it is still `pending`**, appending a `killed`
/// error. Used by `/job/kill` so an orphaned pending job (one that was never
/// enqueued — pickup is CREATE-only) is actually stopped, instead of merely
/// setting an in-memory flag that the runner can only honor at dequeue. The
/// `WHERE status = 'pending'` guard keeps the single-writer invariant intact:
/// it never clobbers a row the runner has already advanced to `processing`.
/// Returns `true` when a row was updated (it was pending), `false` otherwise.
pub async fn fail_if_pending_helper<C: Connection>(
    db: &Surreal<C>,
    job_id: &str,
    error: Value,
) -> Result<bool> {
    let record_id = RecordId::parse_simple(job_id)
        .context(format!("Invalid job ID: {}", job_id))?;

    let mut resp = db
        .query(
            "UPDATE $id SET status = 'failed', \
             errors = array::append(errors, $error), updated_at = time::now() \
             WHERE status = 'pending' RETURN AFTER",
        )
        .bind(("id", record_id))
        .bind(("error", error))
        .await
        .context("Failed to fail pending job")?;

    let updated: Vec<Value> = resp.take(0).context("Failed to read kill result")?;
    Ok(!updated.is_empty())
}

/// Calculate retry delay based on strategy
fn calculate_delay(retries: u32, strategy: &str) -> Duration {
    match strategy {
        "exponential" => {
            // Exponential backoff: 200ms * 2^retries (200ms, 400ms, 800ms, 1.6s...)
            let base_ms = 200u64;
            let multiplier = 2u64.saturating_pow(retries);
            Duration::from_millis(base_ms.saturating_mul(multiplier))
        }
        _ => {
            // Linear backoff: 200ms * (retries + 1) (200ms, 400ms, 600ms...)
            Duration::from_millis(200 * (retries as u64 + 1))
        }
    }
}

#[cfg(test)]
mod tests {
    //! Close-to-e2e tests for recurring jobs: a REAL embedded SurrealDB plus a
    //! REAL mock HTTP backend, driving the actual runner + re-arm SQL. Covers the
    //! whole recurring lifecycle (success re-arms, never terminalizes; failure
    //! re-arms; one-shot jobs still terminalize) and the helper/parse building
    //! blocks.
    use super::*;
    use surrealdb::engine::local::{Db, Mem};
    use wiremock::matchers::{method, path as match_path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn mem_db() -> Arc<Surreal<Db>> {
        let db = Surreal::new::<Mem>(()).await.expect("start mem db");
        db.use_ns("test").use_db("test").await.expect("use ns/db");
        Arc::new(db)
    }

    /// Insert a job row. `id_part` is the id after `job:`.
    async fn insert_job(
        db: &Surreal<Db>,
        id_part: &str,
        status: &str,
        recurring: bool,
        interval_ms: i64,
    ) {
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

    async fn select_string(db: &Surreal<Db>, sql: &str) -> Option<String> {
        db.query(sql).await.expect("query").take(0).expect("take")
    }
    async fn select_bool(db: &Surreal<Db>, sql: &str) -> Option<bool> {
        db.query(sql).await.expect("query").take(0).expect("take")
    }
    async fn select_i64(db: &Surreal<Db>, sql: &str) -> Option<i64> {
        db.query(sql).await.expect("query").take(0).expect("take")
    }

    fn make_runner(db: Arc<Surreal<Db>>) -> JobRunner<Db> {
        let (tx, rx) = mpsc::channel::<JobEntry>(16);
        JobRunner::new(rx, tx, db, JobControl::new())
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
        let db = mem_db().await;
        insert_job(&db, "r1", "processing", true, 300_000).await;

        rearm_recurring_helper(&*db, "job:r1", 300_000).await.unwrap();

        assert_eq!(
            select_string(&db, "SELECT VALUE status FROM ONLY job:r1").await.as_deref(),
            Some("pending"),
            "re-armed row returns to pending, not success/failed"
        );
        assert_eq!(
            select_i64(&db, "SELECT VALUE retries FROM ONLY job:r1").await,
            Some(0),
            "retries reset for the next cycle"
        );
        assert_eq!(
            select_bool(&db, "SELECT VALUE next_run_at > time::now() FROM ONLY job:r1").await,
            Some(true),
            "next_run_at pushed into the future by ~interval"
        );
    }

    #[tokio::test]
    async fn rearm_is_a_noop_when_not_processing() {
        // Guard: never clobber a row that isn't the one this runner just ran
        // (e.g. an operator killed it). Only a `processing` row is re-armed.
        let db = mem_db().await;
        insert_job(&db, "r2", "pending", true, 300_000).await;

        rearm_recurring_helper(&*db, "job:r2", 300_000).await.unwrap();

        // next_run_at was never set (still NONE) because the guard skipped it.
        assert_eq!(
            select_bool(&db, "SELECT VALUE next_run_at != NONE FROM ONLY job:r2").await,
            Some(false),
            "guarded WHERE status='processing' left the pending row untouched"
        );
    }

    // --- helper: JobEntry::from_record -------------------------------------

    #[test]
    fn from_record_parses_recurring_fields() {
        let rec = json!({
            "path": "/run",
            "payload": {},
            "recurring": true,
            "interval": 300_000,
        });
        let e = JobEntry::from_record("job:x".into(), "http://b".into(), None, &rec, Duration::from_secs(1));
        assert!(e.recurring);
        assert_eq!(e.interval_ms, 300_000);
    }

    #[test]
    fn from_record_defaults_to_one_shot() {
        let rec = json!({ "path": "/run", "payload": {} });
        let e = JobEntry::from_record("job:x".into(), "http://b".into(), None, &rec, Duration::from_secs(1));
        assert!(!e.recurring, "missing recurring => one-shot");
        assert_eq!(e.interval_ms, 0, "missing interval => 0");
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

        let db = mem_db().await;
        insert_job(&db, "run1", "pending", true, 300_000).await;
        let runner = make_runner(db.clone());

        runner
            .execute_job(job_entry("job:run1", backend.uri(), true, 300_000, 3))
            .await
            .unwrap();

        assert_eq!(backend.received_requests().await.unwrap().len(), 1, "backend was called");
        assert_eq!(
            select_string(&db, "SELECT VALUE status FROM ONLY job:run1").await.as_deref(),
            Some("pending"),
            "recurring row never reaches terminal success"
        );
        assert_eq!(
            select_bool(&db, "SELECT VALUE next_run_at > time::now() FROM ONLY job:run1").await,
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

        let db = mem_db().await;
        insert_job(&db, "run2", "pending", false, 0).await;
        let runner = make_runner(db.clone());

        runner
            .execute_job(job_entry("job:run2", backend.uri(), false, 0, 3))
            .await
            .unwrap();

        assert_eq!(
            select_string(&db, "SELECT VALUE status FROM ONLY job:run2").await.as_deref(),
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

        let db = mem_db().await;
        insert_job(&db, "run3", "pending", true, 300_000).await;
        let runner = make_runner(db.clone());

        // max_retries=0 => exhausts immediately (no backoff sleep), hits the
        // terminal branch, which for a recurring job re-arms rather than fails.
        runner
            .execute_job(job_entry("job:run3", backend.uri(), true, 300_000, 0))
            .await
            .unwrap();

        assert_eq!(
            select_string(&db, "SELECT VALUE status FROM ONLY job:run3").await.as_deref(),
            Some("pending"),
            "a backend outage must not kill the schedule"
        );
        assert_eq!(
            select_bool(&db, "SELECT VALUE next_run_at > time::now() FROM ONLY job:run3").await,
            Some(true),
            "re-armed for the next interval despite the failure"
        );
    }

    // --- the shared due-clause selects due rows and skips future ones -------

    #[tokio::test]
    async fn pending_due_clause_selects_due_and_skips_future() {
        let db = mem_db().await;
        // due recurring (next_run_at in the past)
        db.query("CREATE job:due SET status='pending', recurring=true, interval=300000, next_run_at = time::now() - 1s, created_at = time::now(), delay = 0").await.unwrap();
        // not-yet-due recurring (next_run_at in the future)
        db.query("CREATE job:future SET status='pending', recurring=true, interval=300000, next_run_at = time::now() + 1h, created_at = time::now(), delay = 0").await.unwrap();
        // one-shot still inside its delay window (created now + 1h delay)
        db.query("CREATE job:delayed SET status='pending', created_at = time::now(), delay = 3600000").await.unwrap();
        // one-shot ready (no delay, no next_run_at)
        db.query("CREATE job:ready SET status='pending', created_at = time::now() - 1s, delay = 0").await.unwrap();

        let sql = format!("SELECT VALUE type::string(id) FROM job WHERE status = 'pending' AND {} ORDER BY id", PENDING_DUE_CLAUSE);
        let ids: Vec<String> = db.query(&sql).await.unwrap().take(0).unwrap();

        assert!(ids.contains(&"job:due".to_string()), "due recurring selected");
        assert!(ids.contains(&"job:ready".to_string()), "ready one-shot selected");
        assert!(!ids.contains(&"job:future".to_string()), "future recurring skipped");
        assert!(!ids.contains(&"job:delayed".to_string()), "delayed one-shot skipped");
    }

    // --- the SSP poke due-check SQL (poke=now => run; re-arm=future => skip) -

    #[tokio::test]
    async fn poke_due_sql_true_for_past_false_for_future() {
        let db = mem_db().await;

        let past: Option<bool> = db
            .query(POKE_DUE_SQL)
            .bind(("nra", "2000-01-01T00:00:00Z".to_string()))
            .await
            .unwrap()
            .take(0)
            .unwrap();
        assert_eq!(past, Some(true), "a poke (next_run_at=now/past) is due -> runs");

        let future: Option<bool> = db
            .query(POKE_DUE_SQL)
            .bind(("nra", "2999-01-01T00:00:00Z".to_string()))
            .await
            .unwrap()
            .take(0)
            .unwrap();
        assert_eq!(future, Some(false), "a re-arm (next_run_at in future) is not due -> no busy loop");
    }
}
