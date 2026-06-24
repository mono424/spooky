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
                self.update_status(&job.id, "success").await?;
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
