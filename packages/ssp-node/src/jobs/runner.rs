use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use super::dispatcher::JobDispatcher;
use super::types::{JobControl, JobEntry};
use crate::api::Method;
use crate::ports::{Db, HttpClient, HttpError, OutboundRequest, Scheduler, Spawner};

/// SQL boolean expression (for recovery-sweep `WHERE` clauses): is a pending job
/// DUE now? A job is due at `created_at + <delay>`; `??` falls back when `delay`
/// is unset (NONE), and `delay = 0` ⇒ ready immediately. Shared by the SSP
/// singlenode sweep and the cluster scheduler sweep so the two never drift, and
/// exercised directly by tests.
///
/// Recurring schedules used to add a `next_run_at ??` arm here. They are now
/// declarative and server-side (`schedules:` in sp00ky.yml): the schedule engine
/// keeps its own clock in `_00_schedule.next_fire_at` and spawns a NEW job row
/// per cycle, so an outbox row is once again always a single execution.
pub const PENDING_DUE_CLAUSE: &str =
    "(created_at + <duration>(string::concat(<string>(delay ?? 0), 'ms'))) <= time::now()";

/// Guard the `type::record($id)` conversion: SurrealDB record ids are
/// `table:key`. The former implementation used `RecordId::parse_simple`;
/// this keeps the same reject-garbage-early behavior without the SDK type.
fn validate_job_id(job_id: &str) -> Result<()> {
    let (table, key) = job_id
        .split_once(':')
        .with_context(|| format!("Invalid job ID: {}", job_id))?;
    if table.is_empty() || key.is_empty() {
        anyhow::bail!("Invalid job ID: {}", job_id);
    }
    Ok(())
}

/// Everything executing a job needs, minus the receiver. Split out from
/// [`JobRunner`] so the loop can hand an `Arc` of it to each spawned execution.
pub struct JobRunnerCtx {
    db: Arc<dyn Db>,
    http: Arc<dyn HttpClient>,
    scheduler: Arc<dyn Scheduler>,
    spawner: Arc<dyn Spawner>,
    dispatcher: Arc<JobDispatcher>,
    job_control: JobControl,
}

pub struct JobRunner {
    queue_rx: mpsc::Receiver<JobEntry>,
    ctx: Arc<JobRunnerCtx>,
}

impl JobRunner {
    pub fn new(
        queue_rx: mpsc::Receiver<JobEntry>,
        db: Arc<dyn Db>,
        http: Arc<dyn HttpClient>,
        scheduler: Arc<dyn Scheduler>,
        spawner: Arc<dyn Spawner>,
        dispatcher: Arc<JobDispatcher>,
    ) -> Self {
        let job_control = dispatcher.job_control().clone();
        Self {
            queue_rx,
            ctx: Arc::new(JobRunnerCtx {
                db,
                http,
                scheduler,
                spawner,
                dispatcher,
                job_control,
            }),
        }
    }

    /// The execution context, for callers that need to run a job directly.
    pub fn ctx(&self) -> &Arc<JobRunnerCtx> {
        &self.ctx
    }

    /// Run the job runner loop.
    ///
    /// The loop dispatches and moves on rather than awaiting each job: how many
    /// run at once is the [`JobDispatcher`]'s decision, and nothing reaches this
    /// receiver without a permit. Awaiting here would pin every table in the
    /// deployment to one concurrent job no matter what the policy says.
    pub async fn run(mut self) {
        info!("Job runner started");

        while let Some(job) = self.queue_rx.recv().await {
            debug!(job_id = %job.id, path = %job.path, "Processing job");

            let ctx = Arc::clone(&self.ctx);
            let table = job.table.clone();
            self.ctx.spawner.spawn(Box::pin(async move {
                if let Err(e) = ctx.execute_job(job).await {
                    error!(error = %e, "Error executing job");
                }
                // The permit went with the entry, so the slot is free by now.
                ctx.dispatcher.kick_drain(&table);
            }));
        }

        info!("Job runner stopped");
    }

    /// Execute one job on this runner's context, bypassing the queue.
    ///
    /// Test-only: production always arrives through `run()`, which is what
    /// guarantees a job carries the permit that bounds concurrency.
    #[cfg(test)]
    pub(crate) async fn execute_job(&self, job: JobEntry) -> Result<()> {
        self.ctx.execute_job(job).await
    }
}

impl JobRunnerCtx {
    /// Execute a single job
    pub(crate) async fn execute_job(&self, job: JobEntry) -> Result<()> {
        // Killed-while-pending: an operator called /job/kill before this job was
        // dequeued. Fail it terminally without ever firing the request. The runner
        // is the sole writer of `status`, so doing the write here (rather than in
        // the kill handler) avoids clobbering a job that may have just flipped to
        // 'processing'.
        if self.job_control.take_killed_pending(&job.id) {
            info!(job_id = %job.id, "Job killed while pending — failing without execution");
            let error_entry = json!({ "code": "killed", "reason": "killed by operator" });
            self.append_error(&job.id, error_entry).await.ok();
            // Release the in-flight mark even if the status write fails —
            // leaking it wedges the id against every future re-enqueue.
            let status = self.update_status(&job.id, "failed").await;
            self.job_control.clear_enqueued(&job.id);
            status?;
            return Ok(());
        }

        // Update status to "processing". On failure, release the in-flight
        // mark before bailing (same leak hazard as above): the row is still
        // pending in the DB, so the sweep can re-dispatch it cleanly.
        if let Err(e) = self.update_status(&job.id, "processing").await {
            self.job_control.clear_enqueued(&job.id);
            return Err(e);
        }

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

        // Register cancellation for the duration of the in-flight request so
        // `/job/kill` on a 'processing' job can abort it. The HttpClient port
        // guarantees cancel WINS ties against a response that completed in the
        // same poll (the old `select! { biased; … }` semantics).
        let cancel = self.job_control.register_inflight(&job.id);

        let outcome = self
            .http
            .send(
                OutboundRequest {
                    method: Method::Post,
                    url,
                    bearer: job.auth_token.clone(),
                    headers: vec![],
                    json_body: Some(payload),
                    timeout: job.timeout,
                },
                Some(cancel),
            )
            .await;

        // Always release the registration; see `JobControl::release_inflight`.
        self.job_control.release_inflight(&job.id);

        match outcome {
            Err(HttpError::Cancelled) => {
                info!(job_id = %job.id, "Job cancelled by operator — marking failed");
                let error_entry = json!({ "code": "cancelled", "reason": "killed by operator" });
                self.append_error(&job.id, error_entry).await.ok();
                // An operator kill is terminal: do NOT run handle_failure, so the
                // backoff-retry path never re-queues a job the operator killed.
                self.update_status(&job.id, "failed").await?;
                self.job_control.clear_enqueued(&job.id);
            }
            Ok(response) if (200..300).contains(&response.status) => {
                info!(job_id = %job.id, status = response.status, "Job completed successfully");
                self.complete_success(&job.id, &response.body).await?;
                self.job_control.clear_enqueued(&job.id);
            }
            Ok(response) => {
                warn!(
                    job_id = %job.id,
                    status = response.status,
                    error_body = %response.body,
                    "Job request failed with non-success status"
                );

                // Create error entry with code and reason
                let error_entry = json!({
                    "code": response.status,
                    "reason": response.body
                });

                self.handle_failure(job, Some(error_entry)).await?;
            }
            Err(e) => {
                warn!(job_id = %job.id, error = %e, "Job request failed");

                // Create error entry for request/timeout error
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
    ///
    /// The DB bookkeeping in here (error append, retry counter) is
    /// best-effort: a `?` on those writes used to abort the whole handler,
    /// which skipped BOTH the retry scheduling and `clear_enqueued` — the job
    /// then sat marked in-flight on this SSP forever, and every later
    /// `/job/recover` bounced off `mark_enqueued` as "already queued" (a
    /// permanently wedged job the sweep could never rescue). The retry budget
    /// is enforced from the in-memory `job.retries` either way.
    async fn handle_failure(&self, mut job: JobEntry, error_entry: Option<serde_json::Value>) -> Result<()> {
        job.retries += 1;

        // Append error to database if provided (best-effort, see above)
        if let Some(error) = error_entry {
            if let Err(e) = self.append_error(&job.id, error).await {
                warn!(job_id = %job.id, error = %e, "Failed to append job error (continuing)");
            }
        }

        // Persist the incremented attempt count regardless of outcome, so a job
        // that exhausts its budget ends at retries == max_retries. Best-effort:
        // the enforced budget is the in-memory job.retries.
        if let Err(e) = self.increment_retries(&job.id).await {
            warn!(job_id = %job.id, error = %e, "Failed to persist retry count (continuing)");
        }

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

            // Release the execution slot for the duration of the backoff. A job
            // asleep on a timer is not running, and holding its permit would
            // mean a table with `concurrency: 1` sits idle through every
            // backoff — a regression against the serial runner this replaces.
            job.permit = None;

            // Requeue with delay: a fire-and-forget task that sleeps through the
            // backoff. Lost on restart/eviction by design — the recovery sweep
            // re-picks the pending row (deadlines live in SurrealQL, not here).
            let dispatcher = Arc::clone(&self.dispatcher);
            let db = Arc::clone(&self.db);
            let scheduler = Arc::clone(&self.scheduler);
            let job_control = self.job_control.clone();
            let job_id = job.id.clone();
            let table = job.table.clone();
            self.spawner.spawn(Box::pin(async move {
                scheduler.sleep(delay).await;

                // Update status to pending before re-queueing
                if let Err(e) = update_status_helper(db.as_ref(), &job_id, "pending").await {
                    error!(job_id = %job_id, error = %e, "Failed to update status for retry");
                    return;
                }

                // Re-queue the job — through admission, so a retry cannot push
                // the table over its limit. Refused means the slots are full:
                // release the in-flight mark and let the row take its turn in
                // the backlog like any other pending row. The attempt count
                // survives on the row (`increment_retries` above), which is how
                // the recovery path has always rebuilt it.
                if !dispatcher.try_admit(job).await {
                    job_control.clear_enqueued(&job_id);
                    dispatcher.note_backlog(&table);
                    debug!(job_id = %job_id, "Retry deferred to the backlog");
                }
            }));
        } else {
            warn!(
                job_id = %job.id,
                retries = job.retries,
                "Job exceeded max retries - marking as failed"
            );
            // Terminal: the retry branch above deliberately keeps the enqueued
            // mark (the job stays in flight across the backoff), but here the
            // job is done, so release it for any future re-enqueue — even when
            // the status write fails (see doc comment).
            let status = self.update_status(&job.id, "failed").await;
            self.job_control.clear_enqueued(&job.id);
            status?;
        }

        Ok(())
    }

    async fn append_error(&self, job_id: &str, error: serde_json::Value) -> Result<()> {
        append_error_helper(self.db.as_ref(), job_id, error).await
    }

    async fn update_status(&self, job_id: &str, status: &str) -> Result<()> {
        update_status_helper(self.db.as_ref(), job_id, status).await
    }

    async fn complete_success(&self, job_id: &str, body: &str) -> Result<()> {
        complete_success_helper(self.db.as_ref(), job_id, body).await
    }

    async fn increment_retries(&self, job_id: &str) -> Result<()> {
        validate_job_id(job_id)?;
        self.db
            .query(
                "UPDATE type::record($id) SET retries = retries + 1, updated_at = time::now()",
                &[("id", json!(job_id))],
            )
            .await
            .context("Failed to increment retries")?;
        Ok(())
    }
}

/// Cap on the stored size of a job's captured `result`. A backend that streams
/// back a megabyte of rows should not bloat every SELECT of the outbox table (or
/// the payload of a workflow step that depends on it). Bodies past the cap are
/// stored as a marker object instead — the job still succeeds, and the operator
/// sees why the output is missing. A const rather than an env var because the
/// portable core reads the process environment zero times (see `NodeConfig`).
pub const JOB_RESULT_MAX_BYTES: usize = 64 * 1024;

/// Terminalize a successful job AND capture the backend's response body in
/// `result`.
///
/// The body is stored as parsed JSON when it parses (so `result.fileId` works in
/// a dependent workflow step) and as a plain string otherwise.
///
/// Outbox tables are user-owned and usually SCHEMAFULL, and `result` only
/// arrived with the platform-field injection in `build_outbox_platform_fields`.
/// A project that upgraded the stack without re-applying its schema therefore
/// has no such field, and SurrealDB rejects the WHOLE update — which would wedge
/// job completion, the one thing that must never happen. So a failed write falls
/// back to a status-only update: such a job completes normally, it just has no
/// captured output.
pub async fn complete_success_helper(db: &dyn Db, job_id: &str, body: &str) -> Result<()> {
    validate_job_id(job_id)?;
    let result = encode_job_result(body);
    let attempt = db
        .query(
            "UPDATE type::record($id) SET status = 'success', result = $result, \
             updated_at = time::now()",
            &[("id", json!(job_id)), ("result", result)],
        )
        .await;

    match attempt {
        Ok(_) => Ok(()),
        Err(crate::ports::DbError::Query(e)) => {
            warn!(
                job_id = %job_id,
                error = %e,
                "Could not store job result (is the outbox schema up to date?) — \
                 completing the job without it"
            );
            update_status_helper(db, job_id, "success").await
        }
        Err(e) => Err(e.into()),
    }
}

/// Response body → the value stored in `result`: parsed JSON when it parses, a
/// plain string otherwise, or a marker when it exceeds [`JOB_RESULT_MAX_BYTES`].
fn encode_job_result(body: &str) -> Value {
    if body.len() > JOB_RESULT_MAX_BYTES {
        return json!({
            "truncated": true,
            "bytes": body.len(),
            "limit": JOB_RESULT_MAX_BYTES,
        });
    }
    if body.trim().is_empty() {
        return Value::Null;
    }
    serde_json::from_str(body).unwrap_or_else(|_| json!(body))
}

/// Helper function to update status (used by both JobRunner and spawned tasks)
pub async fn update_status_helper(db: &dyn Db, job_id: &str, status: &str) -> Result<()> {
    validate_job_id(job_id)?;
    db.query(
        "UPDATE type::record($id) SET status = $status, updated_at = time::now()",
        &[("id", json!(job_id)), ("status", json!(status))],
    )
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
pub async fn set_assignee_helper(db: &dyn Db, job_id: &str, assignee: &str) -> Result<()> {
    validate_job_id(job_id)?;
    db.query(
        "UPDATE type::record($id) SET assignee = $assignee RETURN NONE",
        &[("id", json!(job_id)), ("assignee", json!(assignee))],
    )
    .await
    .context("Failed to set job assignee")?;
    Ok(())
}

/// Append an error object to a job's `errors` array. Shared by the runner and the
/// SSP's `/job/kill`/`/job/retry` handlers so every writer uses identical SQL.
pub async fn append_error_helper(db: &dyn Db, job_id: &str, error: Value) -> Result<()> {
    validate_job_id(job_id)?;
    db.query(
        "UPDATE type::record($id) SET errors = array::append(errors, $error), updated_at = time::now()",
        &[("id", json!(job_id)), ("error", error)],
    )
    .await
    .context("Failed to append error")?;
    Ok(())
}

/// Reset a terminal job for re-execution: `status='pending'`, `retries=0`, errors
/// cleared. Used by the SSP `/job/retry` handler before re-enqueueing the job.
pub async fn reset_for_retry_helper(db: &dyn Db, job_id: &str) -> Result<()> {
    validate_job_id(job_id)?;
    db.query(
        "UPDATE type::record($id) SET status = 'pending', retries = 0, errors = [], updated_at = time::now()",
        &[("id", json!(job_id))],
    )
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
pub async fn fail_if_pending_helper(db: &dyn Db, job_id: &str, error: Value) -> Result<bool> {
    validate_job_id(job_id)?;
    let results = db
        .query(
            "UPDATE type::record($id) SET status = 'failed', \
             errors = array::append(errors, $error), updated_at = time::now() \
             WHERE status = 'pending' RETURN AFTER",
            &[("id", json!(job_id)), ("error", error)],
        )
        .await
        .context("Failed to fail pending job")?;

    // First statement's flattened result: array of updated rows (possibly a
    // bare object for a single row, or null for none).
    let updated = match results.first() {
        Some(Value::Array(rows)) => !rows.is_empty(),
        Some(Value::Null) | None => false,
        Some(_) => true,
    };
    Ok(updated)
}

/// Load only the scalar fields of a job row we need. Returns `None` when the
/// job does not exist (or the id is malformed — mirrors the historical
/// RecordId-parse-to-None behavior); statement-level errors also map to
/// `None` (the row shape is wrong ⇒ treat as absent), transport/auth errors
/// surface as `Err`.
pub async fn load_job_record(db: &dyn Db, job_id: &str) -> Result<Option<Value>> {
    if validate_job_id(job_id).is_err() {
        return Ok(None);
    }
    let results = db
        .query(
            "SELECT status, path, payload, retries, max_retries, retry_strategy, timeout \
             FROM ONLY type::record($id)",
            &[("id", json!(job_id))],
        )
        .await;
    match results {
        Ok(values) => Ok(match values.into_iter().next() {
            Some(v @ Value::Object(_)) => Some(v),
            _ => None,
        }),
        Err(crate::ports::DbError::Query(_)) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Mark + enqueue a recovered/re-dispatched job. Returns `false` when the job
/// is already queued or in-flight (the `mark_enqueued` guard), when the queue is
/// closed, or when the table is at its concurrency limit. Shared by the
/// singlenode recovery sweep and `/job/recover`.
///
/// A `false` from the limit is not a failure: the row stays `pending`, which is
/// where the backlog lives, and the drain will admit it in `created_at` order.
pub async fn enqueue_recovered(
    dispatcher: &Arc<JobDispatcher>,
    backend_info: &super::types::BackendInfo,
    id: &str,
    row: &Value,
) -> bool {
    let timeout_override = row.get("timeout").and_then(|v| v.as_u64()).map(|v| v as u32);
    let job_entry = JobEntry::from_record(
        id.to_string(),
        backend_info.base_url.clone(),
        backend_info.auth_token.clone(),
        row,
        backend_info.effective_timeout(timeout_override),
    );
    dispatcher.try_admit(job_entry).await
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
    use super::*;

    #[test]
    fn validate_job_id_accepts_table_colon_key() {
        assert!(validate_job_id("job:abc").is_ok());
        assert!(validate_job_id("job:⟨weird key⟩").is_ok());
        assert!(validate_job_id("nojcolon").is_err());
        assert!(validate_job_id(":nokey").is_err());
        assert!(validate_job_id("notable:").is_err());
    }

    #[test]
    fn delay_strategies() {
        assert_eq!(calculate_delay(1, "linear"), Duration::from_millis(400));
        assert_eq!(calculate_delay(2, "linear"), Duration::from_millis(600));
        assert_eq!(calculate_delay(1, "exponential"), Duration::from_millis(400));
        assert_eq!(calculate_delay(3, "exponential"), Duration::from_millis(1600));
    }

    #[test]
    fn from_record_reads_the_dispatch_fields_and_defaults_the_rest() {
        let e = JobEntry::from_record(
            "job:a".into(),
            "http://b".into(),
            None,
            &json!({
                "path": "/run",
                "payload": { "x": 1 },
                "retries": 2,
                "max_retries": 5,
                "retry_strategy": "exponential",
            }),
            Duration::from_secs(7),
        );
        assert_eq!(e.path, "/run");
        assert_eq!(e.retries, 2);
        assert_eq!(e.max_retries, 5);
        assert_eq!(e.retry_strategy, "exponential");
        assert_eq!(e.timeout, Duration::from_secs(7));

        // A sparse row falls back to the documented defaults.
        let e = JobEntry::from_record(
            "job:b".into(),
            "http://b".into(),
            None,
            &json!({ "path": "/run" }),
            Duration::from_secs(5),
        );
        assert_eq!(e.retries, 0);
        assert_eq!(e.max_retries, 3);
        assert_eq!(e.retry_strategy, "linear");
    }

    #[test]
    fn encodes_results_by_shape_and_size() {
        assert_eq!(encode_job_result("{\"a\":1}"), json!({"a": 1}), "JSON stays addressable");
        assert_eq!(encode_job_result("done"), json!("done"), "non-JSON becomes a string");
        assert_eq!(encode_job_result("   "), Value::Null, "an empty body stores nothing");
        let huge = "x".repeat(JOB_RESULT_MAX_BYTES + 1);
        assert_eq!(encode_job_result(&huge)["truncated"], json!(true));
    }
}
