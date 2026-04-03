use anyhow::Result;
use axum::{extract::State, http::StatusCode, routing::get, Json, Router};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

use crate::job_scheduler::JobTracker;
use crate::query::QueryTracker;
use crate::router::{SspPool, SspState};

/// Get the local IP address from network interfaces (first non-loopback IPv4)
fn get_local_ip() -> Option<String> {
    // Try reading from /proc/net/fib_trie or use a simpler approach
    // Parse IP from the hostname command or network config
    if let Ok(output) = std::process::Command::new("hostname").arg("-I").output() {
        let ips = String::from_utf8_lossy(&output.stdout);
        return ips.split_whitespace().next().map(|s| s.to_string());
    }
    None
}
use crate::SchedulerStatus;

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
    pub views: usize,
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
    pub scheduler_id: String,
    pub status: Arc<RwLock<SchedulerStatus>>,
}

/// Create metrics router
pub fn create_metrics_router(state: MetricsState) -> Router {
    Router::new()
        .route("/metrics", get(get_metrics))
        .route("/health", get(health_check))
        .route("/info", get(info_handler))
        .route("/info/text", get(info_text_handler))
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
                views: ssp.views,
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
) -> (StatusCode, Json<serde_json::Value>) {
    let pool = state.ssp_pool.read().await;
    let ready_ssps = pool
        .all()
        .iter()
        .filter(|ssp| pool.is_ready(&ssp.id))
        .count();

    if ready_ssps > 0 {
        (StatusCode::OK, Json(serde_json::json!({ "status": "healthy" })))
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({ "status": "unavailable" })))
    }
}

/// Info handler — returns entity list with identity and status
async fn info_handler(
    State(state): State<MetricsState>,
) -> Json<serde_json::Value> {
    let scheduler_status = match *state.status.read().await {
        SchedulerStatus::Cloning => "cloning",
        SchedulerStatus::Ready => "ready",
        SchedulerStatus::SnapshotFrozen => "frozen",
        SchedulerStatus::SnapshotUpdating => "updating",
    };

    let pool = state.ssp_pool.read().await;
    let total_views: usize = pool.all().iter().map(|ssp| ssp.views).sum();

    let now = std::time::Instant::now();

    // Collect scheduler environment variables
    let env_vars: serde_json::Map<String, serde_json::Value> = [
        "SP00KY_SCHEDULER_DB_URL", "SP00KY_SCHEDULER_DB_NAMESPACE",
        "SP00KY_SCHEDULER_DB_DATABASE", "SP00KY_SCHEDULER_DB_USERNAME",
        "SCHEDULER_ID",
    ].iter().filter_map(|&key| {
        std::env::var(key).ok().map(|val| (key.to_string(), serde_json::Value::String(val)))
    }).collect();

    // Get scheduler's own IP from network interfaces or env
    let scheduler_ip = get_local_ip();

    let mut entities = vec![serde_json::json!({
        "entity": "scheduler",
        "id": state.scheduler_id,
        "ip": scheduler_ip,
        "status": scheduler_status,
        "views": total_views,
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_seconds": state.start_time.elapsed().as_secs(),
        "last_heartbeat_seconds_ago": null,
        "env": env_vars,
    })];

    for ssp in pool.all() {
        let ssp_status = match pool.get_state(&ssp.id) {
            Some(SspState::Bootstrapping) => "bootstrapping",
            Some(SspState::Replaying) => "replaying",
            Some(SspState::Ready) => "ready",
            None => "unknown",
        };
        let last_heartbeat_seconds_ago = now
            .duration_since(ssp.last_heartbeat)
            .as_secs();
        // Extract IP from SSP's registered URL (e.g. "http://10.100.1.30:8667" -> "10.100.1.30")
        let ssp_ip = ssp.url.trim_start_matches("http://")
            .split(':').next()
            .map(|s| s.to_string());
        entities.push(serde_json::json!({
            "entity": "ssp",
            "id": ssp.id,
            "ip": ssp_ip,
            "status": ssp_status,
            "views": ssp.views,
            "version": ssp.version,
            "uptime_seconds": now.duration_since(ssp.connected_at).as_secs(),
            "last_heartbeat_seconds_ago": last_heartbeat_seconds_ago,
            "env": ssp.env,
        }));
    }

    Json(serde_json::Value::Array(entities))
}

/// Info handler that returns plain text JSON (for SurrealDB DEFINE API consumption)
async fn info_text_handler(
    State(state): State<MetricsState>,
) -> (axum::http::StatusCode, [(axum::http::header::HeaderName, &'static str); 1], String) {
    let json_resp = info_handler(State(state)).await;
    let json_string = serde_json::to_string(&json_resp.0).unwrap_or_else(|_| "[]".to_string());
    (
        axum::http::StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "text/plain")],
        json_string,
    )
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
