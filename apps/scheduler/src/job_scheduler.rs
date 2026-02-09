use anyhow::Result;
use axum::{
    extract::State,
    http::StatusCode,
    routing::post,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

use crate::router::SspPool;
use crate::transport::Transport;

/// Job status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

impl std::fmt::Display for JobStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JobStatus::Pending => write!(f, "pending"),
            JobStatus::Running => write!(f, "running"),
            JobStatus::Completed => write!(f, "completed"),
            JobStatus::Failed => write!(f, "failed"),
        }
    }
}

/// Job dispatch request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobDispatch {
    pub job_id: String,
    pub table: String,
    pub payload: Value,
}

/// Job result from SSP
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobResult {
    pub job_id: String,
    pub status: JobStatus,
    pub result: Option<Value>,
    pub error: Option<String>,
}

/// Job tracker
#[derive(Clone)]
pub struct JobTracker {
    /// Map job_id -> (ssp_id, status)
    jobs: Arc<RwLock<HashMap<String, (String, JobStatus)>>>,
}

impl JobTracker {
    pub fn new() -> Self {
        Self {
            jobs: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Assign job to SSP
    pub async fn assign(&self, job_id: String, ssp_id: String) {
        let mut jobs = self.jobs.write().await;
        jobs.insert(job_id, (ssp_id, JobStatus::Running));
    }

    /// Get SSP assigned to job
    pub async fn get_assignment(&self, job_id: &str) -> Option<(String, JobStatus)> {
        let jobs = self.jobs.read().await;
        jobs.get(job_id).cloned()
    }

    /// Update job status
    pub async fn update_status(&self, job_id: &str, status: JobStatus) {
        let mut jobs = self.jobs.write().await;
        if let Some((ssp_id, _)) = jobs.get(job_id).cloned() {
            jobs.insert(job_id.to_string(), (ssp_id, status));
        }
    }

    /// Complete job (remove from tracker)
    pub async fn complete(&self, job_id: &str) {
        let mut jobs = self.jobs.write().await;
        jobs.remove(job_id);
    }

    /// Get all jobs assigned to an SSP
    pub async fn get_ssp_jobs(&self, ssp_id: &str) -> Vec<String> {
        let jobs = self.jobs.read().await;
        jobs.iter()
            .filter(|(_, (sid, _))| sid == ssp_id)
            .map(|(jid, _)| jid.clone())
            .collect()
    }

    /// Get running jobs count
    pub async fn running_count(&self) -> usize {
        let jobs = self.jobs.read().await;
        jobs.values()
            .filter(|(_, status)| *status == JobStatus::Running)
            .count()
    }
}

/// Job service state
#[derive(Clone)]
pub struct JobState {
    pub ssp_pool: Arc<RwLock<SspPool>>,
    pub transport: Arc<dyn Transport>,
    pub job_tracker: Arc<JobTracker>,
}

/// Create job router
pub fn create_job_router(state: JobState) -> Router {
    Router::new()
        .route("/job/dispatch", post(dispatch_job))
        .route("/job/result", post(handle_job_result))
        .with_state(state)
}

/// Dispatch job to SSP
async fn dispatch_job(
    State(state): State<JobState>,
    Json(request): Json<JobDispatch>,
) -> Result<Json<String>, (StatusCode, String)> {
    info!("Dispatching job: {}", request.job_id);

    // Select SSP for job execution
    let ssp_id = {
        let mut pool = state.ssp_pool.write().await;
        match pool.select_for_query() {
            Some(id) => id,
            None => {
                error!("No ready SSP available for job {}", request.job_id);
                return Err((
                    StatusCode::SERVICE_UNAVAILABLE,
                    "No SSP available".to_string(),
                ));
            }
        }
    };

    // Track job assignment
    state.job_tracker.assign(request.job_id.clone(), ssp_id.clone()).await;

    // Send job to SSP
    let payload = match serde_json::to_vec(&request) {
        Ok(p) => p,
        Err(e) => {
            error!("Failed to serialize job dispatch: {}", e);
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Serialization error: {}", e),
            ));
        }
    };

    if let Err(e) = state
        .transport
        .send_to(&ssp_id, "job.dispatch", &payload)
        .await
    {
        error!("Failed to send job to SSP: {}", e);
        // Remove from tracker on send failure
        state.job_tracker.complete(&request.job_id).await;
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to send to SSP: {}", e),
        ));
    }

    info!("Dispatched job {} to SSP {}", request.job_id, ssp_id);
    Ok(Json(ssp_id))
}

/// Handle job result from SSP
async fn handle_job_result(
    State(state): State<JobState>,
    Json(result): Json<JobResult>,
) -> Result<StatusCode, (StatusCode, String)> {
    info!("Received job result: {} - {}", result.job_id, result.status);

    // Verify job exists in tracker
    let _assignment = match state.job_tracker.get_assignment(&result.job_id).await {
        Some(a) => a,
        None => {
            warn!("Received result for unknown job: {}", result.job_id);
            return Err((
                StatusCode::NOT_FOUND,
                format!("Job {} not found", result.job_id),
            ));
        }
    };

    // Update status
    state.job_tracker.update_status(&result.job_id, result.status.clone()).await;

    // If completed or failed, remove from tracker
    if result.status == JobStatus::Completed || result.status == JobStatus::Failed {
        state.job_tracker.complete(&result.job_id).await;
        info!("Job {} finished with status: {}", result.job_id, result.status);
    }

    // TODO: Update job status in SurrealDB
    // This would require a DB connection in JobState
    // For now, just log the result

    if let Some(error) = &result.error {
        error!("Job {} failed: {}", result.job_id, error);
    }

    Ok(StatusCode::OK)
}

/// Start background job failover monitor
pub async fn start_job_failover_monitor(
    ssp_pool: Arc<RwLock<SspPool>>,
    job_tracker: Arc<JobTracker>,
    _transport: Arc<dyn Transport>,
) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        
        loop {
            interval.tick().await;
            
            // Check for stale SSPs
            let stale_ssps = {
                let pool = ssp_pool.read().await;
                pool.get_stale_ssps(30000) // 30s timeout
            };
            
            if stale_ssps.is_empty() {
                continue;
            }
            
            warn!("Found {} stale SSPs, handling job failover", stale_ssps.len());
            
            // For each stale SSP, reassign its jobs
            for ssp_id in stale_ssps {
                let jobs = job_tracker.get_ssp_jobs(&ssp_id).await;
                
                if jobs.is_empty() {
                    continue;
                }
                
                info!("Reassigning {} jobs from stale SSP {}", jobs.len(), ssp_id);
                
                // Select new SSP for each job
                for job_id in jobs {
                    let new_ssp = {
                        let mut pool = ssp_pool.write().await;
                        pool.select_for_query()
                    };
                    
                    if let Some(new_ssp_id) = new_ssp {
                        // Update assignment
                        job_tracker.assign(job_id.clone(), new_ssp_id.clone()).await;
                        
                        // TODO: Resend job to new SSP
                        // This requires storing job payloads in tracker
                        info!("Reassigned job {} from {} to {}", job_id, ssp_id, new_ssp_id);
                    } else {
                        error!("No SSP available to reassign job {}", job_id);
                        // Mark as failed
                        job_tracker.update_status(&job_id, JobStatus::Failed).await;
                    }
                }
                
                // Remove stale SSP from pool
                let mut pool = ssp_pool.write().await;
                pool.remove(&ssp_id);
                info!("Removed stale SSP {}", ssp_id);
            }
        }
    });
}
