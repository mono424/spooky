use crate::types::JobEntry;
use anyhow::{Context, Result};
use std::sync::Arc;
use std::time::Duration;
use surrealdb::{Connection, Surreal};
use surrealdb::types::RecordId;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

pub struct JobRunner<C: Connection> {
    queue_rx: mpsc::Receiver<JobEntry>,
    db: Arc<Surreal<C>>,
    http_client: reqwest::Client,
}

impl<C: Connection> JobRunner<C> {
    pub fn new(queue_rx: mpsc::Receiver<JobEntry>, db: Arc<Surreal<C>>) -> Self {
        let http_client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .expect("Failed to create HTTP client");

        Self {
            queue_rx,
            db,
            http_client,
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
        // Update status to "processing"
        self.update_status(&job.id, "processing").await?;

        // Build URL
        let url = format!("{}{}", job.base_url, job.path);

        debug!(job_id = %job.id, url = %url, "Sending HTTP request");

        // Execute HTTP request
        let result = self
            .http_client
            .post(&url)
            .json(&job.payload)
            .send()
            .await;

        match result {
            Ok(response) if response.status().is_success() => {
                info!(job_id = %job.id, status = %response.status(), "Job completed successfully");
                self.update_status(&job.id, "success").await?;
            }
            Ok(response) => {
                warn!(
                    job_id = %job.id,
                    status = %response.status(),
                    "Job request failed with non-success status"
                );
                self.handle_failure(job).await?;
            }
            Err(e) => {
                warn!(job_id = %job.id, error = %e, "Job request failed");
                self.handle_failure(job).await?;
            }
        }

        Ok(())
    }

    /// Handle job failure - retry or mark as failed
    async fn handle_failure(&self, mut job: JobEntry) -> Result<()> {
        job.retries += 1;

        if job.retries < job.max_retries {
            // Increment retries in database
            self.increment_retries(&job.id).await?;

            // Calculate delay based on retry strategy
            let delay = calculate_delay(job.retries, &job.retry_strategy);

            info!(
                job_id = %job.id,
                retries = job.retries,
                max_retries = job.max_retries,
                delay_secs = delay.as_secs(),
                "Job will be retried"
            );

            // Requeue with delay
            let db = self.db.clone();
            let job_id = job.id.clone();
            tokio::spawn(async move {
                tokio::time::sleep(delay).await;
                // Note: In a real implementation, we'd need to re-send to the queue
                // For now, we'll just update the status back to pending
                if let Err(e) = update_status_helper(&db, &job_id, "pending").await {
                    error!(job_id = %job_id, error = %e, "Failed to update status for retry");
                }
            });
        } else {
            warn!(
                job_id = %job.id,
                retries = job.retries,
                "Job exceeded max retries - marking as failed"
            );
            self.update_status(&job.id, "failed").await?;
        }

        Ok(())
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
            .query("UPDATE $id SET retries = retries + 1")
            .bind(("id", record_id))
            .await
            .context("Failed to increment retries")?;

        Ok(())
    }
}

/// Helper function to update status (used by both JobRunner and spawned tasks)
async fn update_status_helper<C: Connection>(
    db: &Surreal<C>,
    job_id: &str,
    status: &str,
) -> Result<()> {
    let record_id = RecordId::parse_simple(job_id)
        .context(format!("Invalid job ID: {}", job_id))?;

    db.query("UPDATE $id SET status = $status")
        .bind(("id", record_id))
        .bind(("status", status.to_string()))
        .await
        .context("Failed to update status")?;

    Ok(())
}

/// Calculate retry delay based on strategy
fn calculate_delay(retries: u32, strategy: &str) -> Duration {
    match strategy {
        "exponential" => {
            // Exponential backoff: 2^retries seconds (2s, 4s, 8s, 16s...)
            let secs = 2u64.saturating_pow(retries);
            Duration::from_secs(secs)
        }
        _ => {
            // Linear backoff: 5 * (retries + 1) seconds (5s, 10s, 15s...)
            Duration::from_secs(5 * (retries as u64 + 1))
        }
    }
}
