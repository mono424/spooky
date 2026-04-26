use anyhow::Result;
use axum::{
    extract::State,
    http::StatusCode,
    routing::post,
    Json, Router,
};
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
use ssp_protocol::{SspRegistration, SspRegistrationResponse};

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
        .route("/admin/ssp/resync-all", post(handle_resync_all))
        .with_state(state)
}

/// Force every connected SSP to re-bootstrap on its next heartbeat.
/// Used by `spky verify --fix` and the integrity-check pipeline when an SSP's
/// circuit hashes have drifted from the scheduler's frozen snapshot.
async fn handle_resync_all(
    State(state): State<SspManagementState>,
) -> Json<serde_json::Value> {
    let count = {
        let mut pool = state.ssp_pool.write().await;
        pool.mark_all_for_resync()
    };
    info!(count, "Flagged SSPs for forced re-bootstrap");
    Json(serde_json::json!({ "marked_for_resync": count }))
}

/// Handle SSP registration — freezes snapshot, returns snapshot_seq, spawns poll task
async fn handle_register(
    State(state): State<SspManagementState>,
    Json(request): Json<SspRegistration>,
) -> Result<(StatusCode, Json<SspRegistrationResponse>), (StatusCode, String)> {
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

    // Check scheduler is not Cloning or Restoring
    {
        let scheduler_status = *state.status.read().await;
        match scheduler_status {
            SchedulerStatus::Cloning => {
                return Err((
                    StatusCode::SERVICE_UNAVAILABLE,
                    "Scheduler is still cloning database".to_string(),
                ));
            }
            SchedulerStatus::Restoring => {
                return Err((
                    StatusCode::SERVICE_UNAVAILABLE,
                    "Scheduler is restoring from backup".to_string(),
                ));
            }
            _ => {}
        }
    }

    // Get snapshot_seq + hashes from replica. The hashes ride alongside the
    // seq so the SSP can verify its bootstrap matches the frozen snapshot.
    let (snapshot_seq, table_hashes) = {
        let replica = state.replica.read().await;
        (replica.snapshot_seq(), replica.snapshot_hashes().clone())
    };

    // Freeze snapshot
    *state.status.write().await = SchedulerStatus::SnapshotFrozen;
    info!(snapshot_seq, "Snapshot frozen for SSP bootstrap");

    // Create SspInfo
    let ssp_info = SspInfo {
        id: request.ssp_id.clone(),
        url: request.url.clone(),
        version: request.version.clone(),
        connected_at: std::time::Instant::now(),
        last_heartbeat: std::time::Instant::now(),
        query_count: 0,
        views: 0,
        cpu_usage: None,
        memory_usage: None,
        env: request.env.clone(),
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

    let replica = state.replica.clone();
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
            replica,
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

    info!(
        tables = table_hashes.len(),
        "SSP registration accepted, polling for bootstrap completion"
    );
    Ok((
        StatusCode::ACCEPTED,
        Json(SspRegistrationResponse {
            snapshot_seq,
            table_hashes,
        }),
    ))
}

/// Handle SSP heartbeat
async fn handle_heartbeat(
    State(state): State<SspManagementState>,
    Json(heartbeat): Json<SspHeartbeat>,
) -> Result<StatusCode, (StatusCode, String)> {
    // Reject heartbeats during restore so SSPs back off instead of spamming
    if *state.status.read().await == SchedulerStatus::Restoring {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "Scheduler is restoring from backup".to_string(),
        ));
    }

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

    // Update SSP with heartbeat data, then check both forced-resync and
    // overflow flags under a single write lock so we can clear the resync
    // flag atomically with returning 409.
    let resync_requested;
    let has_overflow;
    {
        let mut pool = state.ssp_pool.write().await;
        pool.update_ssp(
            &heartbeat.ssp_id,
            heartbeat.views,
            heartbeat.cpu_usage,
            heartbeat.memory_usage,
            heartbeat.version.clone(),
        );
        resync_requested = pool.take_resync_flag(&heartbeat.ssp_id);
        has_overflow = pool.has_buffer_overflow(&heartbeat.ssp_id);
    }

    if resync_requested {
        warn!(ssp_id = %heartbeat.ssp_id, "Forced resync requested by integrity check");
        return Err((
            StatusCode::CONFLICT,
            "Integrity-check resync requested. SSP must re-bootstrap.".to_string(),
        ));
    }

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
#[allow(clippy::too_many_arguments)]
async fn poll_and_replay_ssp(
    ssp_id: String,
    ssp_url: String,
    snapshot_seq: u64,
    ssp_pool: Arc<RwLock<SspPool>>,
    transport: Arc<HttpTransport>,
    event_buffer: Arc<RwLock<VecDeque<BufferedEvent>>>,
    scheduler_status: Arc<RwLock<SchedulerStatus>>,
    config: Arc<SchedulerConfig>,
    replica: Arc<RwLock<Replica>>,
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

        // If the heartbeat-driven stale-SSP sweep has already evicted us
        // from the pool (e.g. the SSP container died mid-bootstrap), stop
        // polling — otherwise we burn the full `bootstrap_timeout_secs`
        // hammering an unreachable URL.
        if ssp_pool.read().await.get(&ssp_id).is_none() {
            anyhow::bail!(
                "SSP '{}' removed from pool during bootstrap — aborting poll",
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

    // Phase 5b: Post-replay integrity check. The SSP has reported itself
    // ready and the replay queue is drained, so its circuit hashes must
    // now agree with the scheduler's frozen snapshot for tables that were
    // *not* touched after the snapshot. Mismatch ⇒ flag for forced
    // re-bootstrap; the SSP exits on the next 409 and re-registers clean.
    if let Err(e) = post_replay_integrity_check(&ssp_id, &ssp_url, &transport, &replica, &ssp_pool).await {
        warn!(ssp_id = %ssp_id, error = %e, "Post-replay integrity check skipped");
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

/// Fetch the SSP's `/info` `circuit_hashes`, compare against the scheduler's
/// current `snapshot_hashes`, and on any mismatch flag the SSP for forced
/// re-bootstrap. Skips comparison for tables the scheduler doesn't have a
/// hash for (e.g. tables created entirely after the snapshot).
async fn post_replay_integrity_check(
    ssp_id: &str,
    ssp_url: &str,
    transport: &Arc<HttpTransport>,
    replica: &Arc<RwLock<Replica>>,
    ssp_pool: &Arc<RwLock<SspPool>>,
) -> Result<()> {
    let info_resp = transport
        .get_from_ssp(ssp_url, "/info")
        .await
        .map_err(|e| anyhow::anyhow!("GET /info failed: {}", e))?;
    if !info_resp.status().is_success() {
        anyhow::bail!("SSP /info returned HTTP {}", info_resp.status());
    }
    let info_json: serde_json::Value = info_resp
        .json()
        .await
        .map_err(|e| anyhow::anyhow!("Parse /info JSON failed: {}", e))?;

    let ssp_hashes: std::collections::BTreeMap<String, String> = info_json
        .as_array()
        .and_then(|arr| arr.first())
        .and_then(|entry| entry.get("circuit_hashes"))
        .and_then(|v| v.as_object())
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default();

    if ssp_hashes.is_empty() {
        // Older SSP build without circuit_hashes — log and skip.
        warn!(ssp_id = %ssp_id, "SSP /info has no circuit_hashes (old build?), skipping integrity check");
        return Ok(());
    }

    let scheduler_hashes = {
        let r = replica.read().await;
        r.snapshot_hashes().clone()
    };

    // Only compare tables present on both sides — tables created after the
    // snapshot may exist on the SSP via replay but not in the scheduler's
    // snapshot_hashes yet, and vice versa. The drain loop will catch up
    // those hashes on its next cycle.
    let mut diffs = Vec::new();
    for (table, sched_hash) in &scheduler_hashes {
        if let Some(ssp_hash) = ssp_hashes.get(table) {
            if sched_hash != ssp_hash {
                diffs.push((table.clone(), sched_hash.clone(), ssp_hash.clone()));
            }
        }
    }

    if diffs.is_empty() {
        info!(ssp_id = %ssp_id, tables = scheduler_hashes.len(), "Post-replay integrity check passed");
        return Ok(());
    }

    for (table, sched, ssp_h) in &diffs {
        error!(
            ssp_id = %ssp_id,
            table = %table,
            scheduler = %sched,
            ssp = %ssp_h,
            "Post-replay hash mismatch"
        );
    }
    error!(
        ssp_id = %ssp_id,
        diffs = diffs.len(),
        "SSP circuit disagrees with scheduler snapshot — flagging for forced re-bootstrap"
    );
    let mut pool = ssp_pool.write().await;
    pool.mark_for_resync(ssp_id);
    Ok(())
}
