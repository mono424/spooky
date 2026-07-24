use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use super::types::{JobControl, JobEntry};
use crate::api::Method;
use crate::ports::{Db, HttpClient, HttpError, OutboundRequest, Scheduler, Spawner};

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

pub struct JobRunner {
    queue_rx: mpsc::Receiver<JobEntry>,
    queue_tx: mpsc::Sender<JobEntry>,
    db: Arc<dyn Db>,
    http: Arc<dyn HttpClient>,
    scheduler: Arc<dyn Scheduler>,
    spawner: Arc<dyn Spawner>,
    job_control: JobControl,
}

impl JobRunner {
    pub fn new(
        queue_rx: mpsc::Receiver<JobEntry>,
        queue_tx: mpsc::Sender<JobEntry>,
        db: Arc<dyn Db>,
        http: Arc<dyn HttpClient>,
        scheduler: Arc<dyn Scheduler>,
        spawner: Arc<dyn Spawner>,
        job_control: JobControl,
    ) -> Self {
        Self {
            queue_rx,
            queue_tx,
            db,
            http,
            scheduler,
            spawner,
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

            // Requeue with delay: a fire-and-forget task that sleeps through the
            // backoff. Lost on restart/eviction by design — the recovery sweep
            // re-picks the pending row (deadlines live in SurrealQL, not here).
            let queue_tx = self.queue_tx.clone();
            let db = Arc::clone(&self.db);
            let scheduler = Arc::clone(&self.scheduler);
            let job_id = job.id.clone();
            self.spawner.spawn(Box::pin(async move {
                scheduler.sleep(delay).await;

                // Update status to pending before re-queueing
                if let Err(e) = update_status_helper(db.as_ref(), &job_id, "pending").await {
                    error!(job_id = %job_id, error = %e, "Failed to update status for retry");
                    return;
                }

                // Re-queue the job
                if let Err(e) = queue_tx.send(job).await {
                    error!(job_id = %job_id, error = %e, "Failed to re-queue job");
                }
            }));
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
            // Release the in-flight mark even if the terminal DB write fails —
            // leaking it wedges the job against every future recover.
            let rearm = self.rearm_recurring(&job.id, job.interval_ms).await;
            self.job_control.clear_enqueued(&job.id);
            rearm?;
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

    async fn rearm_recurring(&self, job_id: &str, interval_ms: u64) -> Result<()> {
        rearm_recurring_helper(self.db.as_ref(), job_id, interval_ms).await
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

/// Re-arm a recurring job after a run completes: set `next_run_at = now + interval`,
/// reset `retries`, and return the row to `pending` so the recovery sweep dispatches
/// the next run once the interval elapses. A recurring row therefore never reaches a
/// terminal `success`/`failed` state; it cycles pending -> processing -> pending.
/// Guarded on `status = 'processing'` (the state the runner set before the run) to
/// preserve the single-writer invariant — it never clobbers a row an operator has
/// killed. Written server-side (root) only.
pub async fn rearm_recurring_helper(db: &dyn Db, job_id: &str, interval_ms: u64) -> Result<()> {
    validate_job_id(job_id)?;
    // `assignee = NONE`: the claim marker means "this SSP holds the job
    // in-memory right now". A re-armed row is waiting for the recovery sweep
    // to dispatch its next run — nobody holds it. Leaving the previous run's
    // assignee in place makes the sweep's is_orphaned check skip the row
    // whenever that SSP is still alive, so the next run never fires.
    db.query(
        "UPDATE type::record($id) SET status = 'pending', retries = 0, assignee = NONE, \
         next_run_at = time::now() + <duration>(string::concat(<string>$interval, 'ms')), \
         updated_at = time::now() WHERE status = 'processing' RETURN NONE",
        &[("id", json!(job_id)), ("interval", json!(interval_ms))],
    )
    .await
    .context("Failed to re-arm recurring job")?;
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
            "SELECT status, path, payload, retries, max_retries, retry_strategy, timeout, recurring, interval \
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
/// is already queued or in-flight (the `mark_enqueued` guard) or the queue is
/// closed. Shared by the singlenode recovery sweep, the recurring-poke path,
/// and `/job/recover`.
pub async fn enqueue_recovered(
    job_control: &super::types::JobControl,
    job_queue_tx: &tokio::sync::mpsc::Sender<JobEntry>,
    backend_info: &super::types::BackendInfo,
    id: &str,
    row: &Value,
) -> bool {
    if !job_control.mark_enqueued(id) {
        return false; // already queued or in-flight
    }
    let timeout_override = row.get("timeout").and_then(|v| v.as_u64()).map(|v| v as u32);
    let job_entry = JobEntry::from_record(
        id.to_string(),
        backend_info.base_url.clone(),
        backend_info.auth_token.clone(),
        row,
        backend_info.effective_timeout(timeout_override),
    );
    if let Err(e) = job_queue_tx.send(job_entry).await {
        job_control.clear_enqueued(id);
        warn!(target: "ssp::job_recovery", job_id = %id, error = %e, "Failed to enqueue recovered job");
        return false;
    }
    true
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
    fn from_record_parses_recurring_fields() {
        let rec = serde_json::json!({
            "path": "/run",
            "payload": {},
            "recurring": true,
            "interval": 300_000,
        });
        let e = JobEntry::from_record(
            "job:x".into(),
            "http://b".into(),
            None,
            &rec,
            Duration::from_secs(1),
        );
        assert!(e.recurring);
        assert_eq!(e.interval_ms, 300_000);
    }

    #[test]
    fn from_record_defaults_to_one_shot() {
        let rec = serde_json::json!({ "path": "/run", "payload": {} });
        let e = JobEntry::from_record(
            "job:x".into(),
            "http://b".into(),
            None,
            &rec,
            Duration::from_secs(1),
        );
        assert!(!e.recurring, "missing recurring => one-shot");
        assert_eq!(e.interval_ms, 0, "missing interval => 0");
    }
}
