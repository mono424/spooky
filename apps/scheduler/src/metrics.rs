use anyhow::Result;
use axum::{extract::State, http::StatusCode, routing::get, Json, Router};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

use crate::job_scheduler::JobTracker;
use crate::query::QueryTracker;
use crate::router::SspPool;

/// Metrics snapshot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metrics {
    pub scheduler: SchedulerMetrics,
    pub ssps: Vec<SspMetrics>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchedulerMetrics {
    pub total_ssps: usize,
    pub ready_ssps: usize,
    pub total_queries: usize,
    pub running_jobs: usize,
    pub uptime_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SspMetrics {
    pub id: String,
    pub query_count: usize,
    pub active_jobs: usize,
    pub cpu_usage: Option<f64>,
    pub memory_usage: Option<f64>,
    pub last_heartbeat_seconds_ago: u64,
}

/// Metrics state
#[derive(Clone)]
pub struct MetricsState {
    pub ssp_pool: Arc<RwLock<SspPool>>,
    pub query_tracker: Arc<QueryTracker>,
    pub job_tracker: Arc<JobTracker>,
    pub start_time: std::time::Instant,
}

/// Create metrics router
pub fn create_metrics_router(state: MetricsState) -> Router {
    Router::new()
        .route("/metrics", get(get_metrics))
        .route("/health", get(health_check))
        .with_state(state)
}

/// Get metrics
async fn get_metrics(
    State(state): State<MetricsState>,
) -> Result<Json<Metrics>, (StatusCode, String)> {
    let pool = state.ssp_pool.read().await;
    let query_assignments = state.query_tracker.all().await;
    let running_jobs = state.job_tracker.running_count().await;

    let total_ssps = pool.count();
    let ready_ssps = pool
        .all()
        .iter()
        .filter(|ssp| pool.is_ready(&ssp.id))
        .count();

    let ssps: Vec<SspMetrics> = pool
        .all()
        .iter()
        .map(|ssp| {
            let now = std::time::Instant::now();
            let last_heartbeat_seconds_ago = now
                .duration_since(ssp.last_heartbeat)
                .as_secs();

            SspMetrics {
                id: ssp.id.clone(),
                query_count: ssp.query_count,
                active_jobs: ssp.active_jobs,
                cpu_usage: ssp.cpu_usage,
                memory_usage: ssp.memory_usage,
                last_heartbeat_seconds_ago,
            }
        })
        .collect();

    let metrics = Metrics {
        scheduler: SchedulerMetrics {
            total_ssps,
            ready_ssps,
            total_queries: query_assignments.len(),
            running_jobs,
            uptime_seconds: state.start_time.elapsed().as_secs(),
        },
        ssps,
    };

    Ok(Json(metrics))
}

/// Health check
async fn health_check(
    State(state): State<MetricsState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let pool = state.ssp_pool.read().await;
    let ready_ssps = pool
        .all()
        .iter()
        .filter(|ssp| pool.is_ready(&ssp.id))
        .count();

    if ready_ssps > 0 {
        Ok(Json(serde_json::json!({
            "status": "healthy",
            "ready_ssps": ready_ssps,
        })))
    } else {
        Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "No ready SSPs available".to_string(),
        ))
    }
}

/// Start query reassignment monitor
pub async fn start_query_reassignment_monitor(
    ssp_pool: Arc<RwLock<SspPool>>,
    query_tracker: Arc<QueryTracker>,
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

            info!(
                "Found {} stale SSPs, reassigning queries",
                stale_ssps.len()
            );

            // Get all query assignments
            let assignments = query_tracker.all().await;

            // For each stale SSP, unassign its queries
            for ssp_id in &stale_ssps {
                let affected_queries: Vec<_> = assignments
                    .iter()
                    .filter(|(_, sid)| *sid == ssp_id)
                    .map(|(qid, _)| qid.clone())
                    .collect();

                if !affected_queries.is_empty() {
                    info!(
                        "Unassigning {} queries from stale SSP {}",
                        affected_queries.len(),
                        ssp_id
                    );

                    for query_id in affected_queries {
                        query_tracker.unassign(&query_id).await;
                        // Client will need to re-register the query
                        info!("Query {} unassigned, client should re-register", query_id);
                    }
                }

                // Remove stale SSP
                let mut pool = ssp_pool.write().await;
                pool.remove(ssp_id);
                info!("Removed stale SSP {}", ssp_id);
            }
        }
    });
}
