use anyhow::Result;
use axum::{
    extract::State,
    http::StatusCode,
    routing::post,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

use crate::config::SchedulerConfig;
use crate::messages::{BufferedEvent, SspHeartbeat};
use crate::replica::Replica;
use crate::router::SspPool;
use crate::transport::{HttpTransport, SspInfo};
use crate::SchedulerStatus;

/// SSP registration request
#[derive(Debug, Deserialize)]
pub struct SspRegistration {
    pub ssp_id: String,
    pub url: String,
}

/// SSP registration response
#[derive(Debug, Serialize)]
pub struct RegistrationResponse {
    pub snapshot_seq: u64,
}

/// Shared state for SSP management handlers
#[derive(Clone)]
pub struct SspManagementState {
    pub ssp_pool: Arc<RwLock<SspPool>>,
    pub replica: Arc<RwLock<Replica>>,
    pub transport: Arc<HttpTransport>,
    pub config: Arc<SchedulerConfig>,
    pub status: Arc<RwLock<SchedulerStatus>>,
    pub event_buffer: Arc<RwLock<VecDeque<BufferedEvent>>>,
}

/// Create SSP management router
pub fn create_ssp_router(state: SspManagementState) -> Router {
    Router::new()
        .route("/ssp/register", post(handle_register))
        .route("/ssp/heartbeat", post(handle_heartbeat))
        .with_state(state)
}

/// Handle SSP registration — freezes snapshot, returns snapshot_seq, spawns poll task
async fn handle_register(
    State(state): State<SspManagementState>,
    Json(request): Json<SspRegistration>,
) -> Result<(StatusCode, Json<RegistrationResponse>), (StatusCode, String)> {
    info!("SSP registration: {} at {}", request.ssp_id, request.url);

    // Validate ssp_id (non-empty)
    if request.ssp_id.trim().is_empty() {
        error!("Invalid SSP ID: empty");
        return Err((StatusCode::BAD_REQUEST, "SSP ID cannot be empty".to_string()));
    }

    // Validate URL (basic check for http/https)
    if !request.url.starts_with("http://") && !request.url.starts_with("https://") {
        error!("Invalid SSP URL: {}", request.url);
        return Err((
            StatusCode::BAD_REQUEST,
            "SSP URL must start with http:// or https://".to_string(),
        ));
    }

    // Check scheduler is not Cloning
    {
        let scheduler_status = *state.status.read().await;
        if scheduler_status == SchedulerStatus::Cloning {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "Scheduler is still cloning database".to_string(),
            ));
        }
    }

    // Get snapshot_seq from replica
    let snapshot_seq = {
        let replica = state.replica.read().await;
        replica.snapshot_seq()
    };

    // Freeze snapshot
    *state.status.write().await = SchedulerStatus::SnapshotFrozen;
    info!(snapshot_seq, "Snapshot frozen for SSP bootstrap");

    // Create SspInfo
    let ssp_info = SspInfo {
        id: request.ssp_id.clone(),
        url: request.url.clone(),
        connected_at: std::time::Instant::now(),
        last_heartbeat: std::time::Instant::now(),
        query_count: 0,
        active_jobs: 0,
        cpu_usage: None,
        memory_usage: None,
    };

    // Add to pool, mark as bootstrapping, record snapshot_seq
    {
        let mut pool = state.ssp_pool.write().await;
        pool.upsert(ssp_info);
        pool.mark_bootstrapping(&request.ssp_id);
        pool.set_bootstrap_seq(&request.ssp_id, snapshot_seq);
    }

    // Spawn polling + replay task
    let ssp_id = request.ssp_id.clone();
    let ssp_url = request.url.clone();
    let ssp_pool = state.ssp_pool.clone();
    let transport = state.transport.clone();
    let event_buffer = state.event_buffer.clone();
    let scheduler_status = state.status.clone();
    let config = state.config.clone();

    tokio::spawn(async move {
        if let Err(e) = poll_and_replay_ssp(
            ssp_id.clone(),
            ssp_url,
            snapshot_seq,
            ssp_pool.clone(),
            transport,
            event_buffer,
            scheduler_status,
            config,
        )
        .await
        {
            error!("Bootstrap/replay failed for SSP '{}': {}", ssp_id, e);
            let mut pool = ssp_pool.write().await;
            pool.remove(&ssp_id);

            // Check if snapshot can be unfrozen
            if !pool.has_active_bootstrap() {
                drop(pool);
                // Note: we can't unfreeze here since we don't have scheduler_status
                // The periodic snapshot updater will handle it
            }
        }
    });

    info!("SSP registration accepted, polling for bootstrap completion");
    Ok((
        StatusCode::ACCEPTED,
        Json(RegistrationResponse { snapshot_seq }),
    ))
}

/// Handle SSP heartbeat
async fn handle_heartbeat(
    State(state): State<SspManagementState>,
    Json(heartbeat): Json<SspHeartbeat>,
) -> Result<StatusCode, (StatusCode, String)> {
    // Check if SSP exists in pool
    let ssp_exists = {
        let pool = state.ssp_pool.read().await;
        pool.get(&heartbeat.ssp_id).is_some()
    };

    if !ssp_exists {
        warn!("Heartbeat from unregistered SSP: {}", heartbeat.ssp_id);
        return Err((
            StatusCode::NOT_FOUND,
            "SSP not registered. Please re-register.".to_string(),
        ));
    }

    // Update SSP with heartbeat data
    {
        let mut pool = state.ssp_pool.write().await;
        pool.update_ssp(
            &heartbeat.ssp_id,
            heartbeat.active_queries,
            heartbeat.cpu_usage,
            heartbeat.memory_usage,
        );
    }

    // Check buffer overflow
    let has_overflow = {
        let pool = state.ssp_pool.read().await;
        pool.has_buffer_overflow(&heartbeat.ssp_id)
    };

    if has_overflow {
        error!("Buffer overflow detected for SSP: {}", heartbeat.ssp_id);
        return Err((
            StatusCode::CONFLICT,
            "Buffer overflow detected. SSP needs to re-bootstrap.".to_string(),
        ));
    }

    Ok(StatusCode::OK)
}

/// Poll SSP health until ready, then replay missed events
async fn poll_and_replay_ssp(
    ssp_id: String,
    ssp_url: String,
    snapshot_seq: u64,
    ssp_pool: Arc<RwLock<SspPool>>,
    transport: Arc<HttpTransport>,
    event_buffer: Arc<RwLock<VecDeque<BufferedEvent>>>,
    scheduler_status: Arc<RwLock<SchedulerStatus>>,
    config: Arc<SchedulerConfig>,
) -> Result<()> {
    let poll_interval = std::time::Duration::from_millis(config.ssp_poll_interval_ms);
    let timeout = std::time::Duration::from_secs(config.bootstrap_timeout_secs);
    let start = std::time::Instant::now();

    info!(
        ssp_id = %ssp_id,
        snapshot_seq,
        "Polling SSP health, waiting for bootstrap completion"
    );

    // Phase 1: Poll SSP health until it reports Ready
    loop {
        if start.elapsed() > timeout {
            anyhow::bail!(
                "Bootstrap timeout ({:?}) exceeded for SSP '{}'",
                timeout,
                ssp_id
            );
        }

        tokio::time::sleep(poll_interval).await;

        match transport.check_ssp_health_status(&ssp_url).await {
            Some(status) if status == "ready" => {
                info!("SSP '{}' reports ready, starting event replay", ssp_id);
                break;
            }
            Some(status) if status == "failed" => {
                anyhow::bail!("SSP '{}' reported bootstrap failure", ssp_id);
            }
            Some(status) => {
                debug!("SSP '{}' status: {}", ssp_id, status);
            }
            None => {
                warn!("Cannot reach SSP '{}', retrying...", ssp_id);
            }
        }
    }

    // Phase 2: Mark SSP as Replaying (ingest will buffer per-SSP during replay)
    {
        let mut pool = ssp_pool.write().await;
        pool.mark_replaying(&ssp_id);
    }

    // Phase 3: Collect and replay events from global buffer
    let events_to_replay: Vec<BufferedEvent> = {
        let buffer = event_buffer.read().await;
        buffer
            .iter()
            .filter(|e| e.seq > snapshot_seq)
            .cloned()
            .collect()
    };

    if !events_to_replay.is_empty() {
        info!(
            "Replaying {} events to SSP '{}' (seq > {})",
            events_to_replay.len(),
            ssp_id,
            snapshot_seq
        );

        for event in &events_to_replay {
            let ingest_payload = serde_json::json!({
                "table": event.update.table,
                "op": event.update.operation.to_string(),
                "id": event.update.record_id,
                "record": event.update.data.clone().unwrap_or(serde_json::json!({}))
            });

            if let Err(e) = transport.post_to_ssp(&ssp_url, "/ingest", &ingest_payload).await {
                warn!(
                    "Failed to replay event seq={} to SSP '{}': {}",
                    event.seq, ssp_id, e
                );
            }
        }

        info!(
            "Replayed {} global buffer events to SSP '{}'",
            events_to_replay.len(),
            ssp_id
        );
    }

    // Phase 4: Drain and replay per-SSP buffered events (accumulated during replay)
    loop {
        let buffered = {
            let mut pool = ssp_pool.write().await;
            pool.drain_buffer(&ssp_id)
        };

        if buffered.is_empty() {
            break;
        }

        info!(
            "Replaying {} per-SSP buffered events to SSP '{}'",
            buffered.len(),
            ssp_id
        );

        for message in &buffered {
            let ingest_payload = serde_json::json!({
                "table": message.table,
                "op": message.operation.to_string(),
                "id": message.record_id,
                "record": message.data.clone().unwrap_or(serde_json::json!({}))
            });

            if let Err(e) = transport.post_to_ssp(&ssp_url, "/ingest", &ingest_payload).await {
                warn!(
                    "Failed to replay buffered event to SSP '{}': {}",
                    ssp_id, e
                );
            }
        }
    }

    // Phase 5: Mark SSP as Ready (atomic with final buffer drain)
    {
        let mut pool = ssp_pool.write().await;
        let remaining = pool.mark_ready(&ssp_id);

        // Replay any events that snuck in between last drain and mark_ready
        if !remaining.is_empty() {
            info!(
                "Replaying {} final buffered events to SSP '{}'",
                remaining.len(),
                ssp_id
            );
            for message in &remaining {
                let ingest_payload = serde_json::json!({
                    "table": message.table,
                    "op": message.operation.to_string(),
                    "id": message.record_id,
                    "record": message.data.clone().unwrap_or(serde_json::json!({}))
                });

                // Drop pool lock before making HTTP call
                drop(pool);

                if let Err(e) = transport
                    .post_to_ssp(&ssp_url, "/ingest", &ingest_payload)
                    .await
                {
                    warn!(
                        "Failed to replay final event to SSP '{}': {}",
                        ssp_id, e
                    );
                }

                // Re-acquire for next iteration (but mark_ready already called)
                pool = ssp_pool.write().await;
            }
        }
    }

    // Phase 6: Unfreeze snapshot if no other SSPs are bootstrapping/replaying
    {
        let has_active = {
            let pool = ssp_pool.read().await;
            pool.has_active_bootstrap()
        };

        if !has_active {
            let mut status = scheduler_status.write().await;
            if *status == SchedulerStatus::SnapshotFrozen {
                *status = SchedulerStatus::Ready;
                info!("Snapshot unfrozen: all SSPs caught up");
            }
        }
    }

    info!("SSP '{}' is now fully caught up and ready", ssp_id);
    Ok(())
}
