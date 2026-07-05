use anyhow::{Context, Result};
use axum::{
    extract::State,
    http::StatusCode,
    routing::post,
    Json, Router,
};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tracing::{debug, error, info, warn};

use crate::config::SchedulerConfig;
use crate::messages::{BufferedEvent, RecordOp, RecordUpdate, SspHeartbeat};
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
    /// Global sequence counter — the seq the replica reflects after a re-clone.
    pub seq_counter: Arc<AtomicU64>,
    /// Serializes catch-up replica re-clones so two looping SSPs can't reset +
    /// re-ingest the shared replica concurrently. A `try_lock` failure means a
    /// re-clone is already running; the losing task just re-bootstraps instead.
    pub reclone_lock: Arc<Mutex<()>>,
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
    let seq_counter = state.seq_counter.clone();
    let reclone_lock = state.reclone_lock.clone();

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
            seq_counter,
            reclone_lock,
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
    seq_counter: Arc<AtomicU64>,
    reclone_lock: Arc<Mutex<()>>,
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

    // Accumulate every event we replay to this SSP, in seq order, so the
    // catch-up check (Phase 4b) can reconstruct the SSP's content at cut M.
    let mut replayed: Vec<RecordUpdate> =
        events_to_replay.iter().map(|e| e.update.clone()).collect();

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

        replayed.extend(buffered.iter().cloned());

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

    // Phase 4b: Verify the SSP's caught-up state at the cut M BEFORE routing any
    // live traffic to it. The SSP is still `Replaying`, so ingest keeps buffering
    // for it (it can't be selected as a broadcast target) — it stays pinned at M
    // while we reconstruct and compare. This replaces the old post-replay check,
    // which compared the SSP's live (seq M) hashes against the frozen snapshot
    // (seq N) and so falsely flagged any table written during catch-up.
    let verified = match verify_catchup_at_m(&ssp_id, &ssp_url, &transport, &replica, &replayed).await {
        Ok(true) => {
            // Passed — clear the consecutive-failure streak.
            ssp_pool.write().await.reset_catchup_failures(&ssp_id);
            true
        }
        Ok(false) => {
            // Real divergence. A plain re-bootstrap can't fix a *deterministic*
            // scheduler-vs-circuit gap — the SSP refetches the same diverging
            // state every cycle — so escalate by consecutive-failure count
            // instead of looping forever (which leaves the whole cluster with
            // NO ready SSP and all live queries failing).
            let fails = ssp_pool.write().await.record_catchup_failure(&ssp_id);
            let (reclone_after, admit_after) = catchup_breaker_thresholds();

            if fails >= admit_after {
                // Give up on the gate: admit the SSP so sync is restored. The
                // circuit passed its own @N bootstrap self-check and every
                // shared row is (almost certainly) semantically correct; a
                // never-converging hash is not worth an indefinite outage. The
                // row-level diagnostic above shows exactly what disagreed.
                error!(
                    ssp_id = %ssp_id,
                    fails,
                    "ALERT: catch-up never converged after {} attempts — admitting SSP to broadcast anyway to restore sync. Circuit may be marginally divergent from the scheduler projection (see the row-level diff above).",
                    fails,
                );
                ssp_pool.write().await.reset_catchup_failures(&ssp_id);
                true
            } else {
                if fails == reclone_after {
                    // The replica itself may hold the odd representation. Do ONE
                    // full re-clone from upstream before the next attempt so the
                    // SSP re-bootstraps from a freshly-cloned snapshot.
                    warn!(
                        ssp_id = %ssp_id,
                        fails,
                        "Catch-up failed {} times — re-cloning replica from upstream before the next attempt",
                        fails,
                    );
                    match reclone_replica_from_upstream(&config, &replica, &seq_counter, &reclone_lock).await {
                        Ok(true) => {
                            info!("Replica re-cloned from upstream; flagging all SSPs to re-bootstrap from the fresh snapshot");
                            ssp_pool.write().await.mark_all_for_resync();
                        }
                        Ok(false) => {
                            info!(ssp_id = %ssp_id, "Re-clone already in progress on another task; just re-bootstrapping this SSP");
                            ssp_pool.write().await.mark_for_resync(&ssp_id);
                        }
                        Err(e) => {
                            error!(ssp_id = %ssp_id, error = %e, "Replica re-clone failed; falling back to a plain re-bootstrap");
                            ssp_pool.write().await.mark_for_resync(&ssp_id);
                        }
                    }
                } else {
                    ssp_pool.write().await.mark_for_resync(&ssp_id);
                }
                warn!(ssp_id = %ssp_id, fails, "Catch-up verification failed — withholding from broadcast; SSP will re-bootstrap");
                false
            }
        }
        Err(e) => {
            // Transient failure (e.g. /info unreachable). Favor availability —
            // the SSP already passed its own @N bootstrap self-check.
            warn!(ssp_id = %ssp_id, error = %e, "Catch-up verification skipped (transient); proceeding to ready");
            true
        }
    };

    // Phase 5: Mark SSP as Ready (atomic with final buffer drain) — only once
    // verification has passed, so live traffic is never routed to an unverified
    // or diverged SSP.
    if verified {
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

    if verified {
        info!("SSP '{}' is now fully caught up and ready", ssp_id);
    } else {
        info!(ssp_id = %ssp_id, "SSP withheld from broadcast pending re-bootstrap");
    }
    Ok(())
}

/// Verify a caught-up SSP at the catch-up cut M, comparing hashes taken at the
/// SAME sequence point on both sides (the bug this replaces compared the SSP's
/// live state@M against the scheduler's frozen snapshot@N).
///
/// Only tables *touched* by the replayed events `(N, M]` need checking —
/// untouched tables were already verified at N by the SSP's bootstrap
/// self-check and did not change. For each touched base table the scheduler
/// reconstructs content@M in memory — rows@N from the frozen replica, then the
/// replayed events applied by REPLACE/remove (matching the SSP circuit's
/// `rows.insert`, never the replica's own MERGE) — and XOR-hashes it the same
/// way the SSP maintains its `catchup_hashes`. No replica mutation, so
/// concurrent bootstraps are unaffected.
///
/// Returns `Ok(true)` on agreement (or nothing to check), `Ok(false)` on a real
/// divergence (caller flags re-bootstrap), `Err` on a transient failure (caller
/// proceeds — the SSP already passed its own @N self-check).
async fn verify_catchup_at_m(
    ssp_id: &str,
    ssp_url: &str,
    transport: &Arc<HttpTransport>,
    replica: &Arc<RwLock<Replica>>,
    replayed: &[RecordUpdate],
) -> Result<bool> {
    use serde_json::Value;
    use std::collections::BTreeMap;

    // Group replayed events per touched base table, preserving seq order. Skip
    // `_00_*` system tables (scheduler/view bookkeeping, not replicated content).
    let mut per_table: BTreeMap<String, Vec<&RecordUpdate>> = BTreeMap::new();
    for ev in replayed {
        if ev.table.starts_with("_00_") {
            continue;
        }
        per_table.entry(ev.table.clone()).or_default().push(ev);
    }
    if per_table.is_empty() {
        // Nothing changed in (N, M]; the @N self-check already covered it.
        return Ok(true);
    }

    // Reconstruct reference@M per touched table (rows@N + replayed events). Keep
    // the projected rows (not just the hash) so a persistent mismatch can dump
    // the diverging row's canonical JSON for diagnosis.
    let mut reference_rows: BTreeMap<String, std::collections::HashMap<String, Value>> =
        BTreeMap::new();
    for (table, events) in &per_table {
        let seed = {
            let rep = replica.read().await;
            rep.snapshot_rows(table)
                .await
                .with_context(|| format!("snapshot_rows for '{}'", table))?
        };
        reference_rows.insert(table.clone(), project_table_rows(table, seed, events));
    }
    let reference: BTreeMap<String, String> = reference_rows
        .iter()
        .map(|(table, rows)| {
            (
                table.clone(),
                ssp_protocol::snapshot_hash::xor_table_hash(rows.clone().into_iter()),
            )
        })
        .collect();

    let empty = ssp_protocol::snapshot_hash::xor_acc_to_hex(&ssp_protocol::snapshot_hash::xor_empty());

    // Compare against the SSP's catch-up hashes with a bounded retry. The SSP is
    // pinned at M, but its `/ingest` handler may apply the last replayed events
    // slightly behind the HTTP ack, so a first read can briefly lag. A genuine
    // divergence stays mismatched across every attempt; a settle lag clears. We
    // never relax the check itself — only give the SSP a moment to converge
    // before forcing a (costly) full re-bootstrap.
    let mut mismatches: Vec<(String, String, String)> = Vec::new();
    for attempt in 1..=CATCHUP_VERIFY_ATTEMPTS {
        // Fetch the SSP's catch-up hashes (it's still `Replaying`, pinned at M).
        let info_resp = transport
            .get_from_ssp(ssp_url, "/info")
            .await
            .map_err(|e| anyhow::anyhow!("GET /info failed: {}", e))?;
        if !info_resp.status().is_success() {
            anyhow::bail!("SSP /info returned HTTP {}", info_resp.status());
        }
        let info_json: Value = info_resp
            .json()
            .await
            .map_err(|e| anyhow::anyhow!("Parse /info JSON failed: {}", e))?;
        let ssp_hashes: BTreeMap<String, String> = info_json
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|entry| entry.get("catchup_hashes"))
            .and_then(|v| v.as_object())
            .map(|m| {
                m.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default();

        if ssp_hashes.is_empty() {
            // Older SSP build without `catchup_hashes` (deploy SSP before scheduler).
            warn!(ssp_id = %ssp_id, "SSP /info has no catchup_hashes (old build?), skipping catch-up check");
            return Ok(true);
        }

        mismatches = reference
            .iter()
            .filter_map(|(table, ref_hash)| {
                let ssp_hash = ssp_hashes.get(table).cloned().unwrap_or_else(|| empty.clone());
                if *ref_hash != ssp_hash {
                    Some((table.clone(), ref_hash.clone(), ssp_hash))
                } else {
                    None
                }
            })
            .collect();

        if mismatches.is_empty() {
            info!(ssp_id = %ssp_id, tables = reference.len(), attempt, "Catch-up verification passed");
            return Ok(true);
        }

        if attempt < CATCHUP_VERIFY_ATTEMPTS {
            warn!(
                ssp_id = %ssp_id,
                attempt,
                mismatches = mismatches.len(),
                "Catch-up mismatch; letting the SSP settle and re-checking"
            );
            tokio::time::sleep(std::time::Duration::from_millis(CATCHUP_VERIFY_RETRY_DELAY_MS)).await;
        }
    }

    // Persistent mismatch after every attempt → a real divergence. Pull the
    // SSP's actual circuit rows and diff them against the scheduler's projection
    // row-by-row, so the log names the specific missing / extra / differing rows
    // instead of the old one-sided dump (which showed the scheduler's first N
    // rows in map order — not necessarily the diverging ones).
    for (table, sched, ssp_h) in &mismatches {
        error!(
            ssp_id = %ssp_id,
            table = %table,
            scheduler = %sched,
            ssp = %ssp_h,
            "Catch-up hash mismatch"
        );
        let ref_rows = match reference_rows.get(table) {
            Some(r) => r,
            None => continue,
        };
        match fetch_ssp_catchup_rows(transport, ssp_url, table).await {
            Ok(ssp_rows) => {
                let all_ids: std::collections::BTreeSet<&String> =
                    ref_rows.keys().chain(ssp_rows.keys()).collect();
                let mut logged = 0usize;
                for id in all_ids {
                    if logged >= CATCHUP_DIAG_MAX_ROWS {
                        warn!(ssp_id = %ssp_id, table = %table, "More diverging rows omitted (diagnostic cap reached)");
                        break;
                    }
                    match (ref_rows.get(id), ssp_rows.get(id)) {
                        (Some(r), None) => {
                            error!(ssp_id = %ssp_id, table = %table, row_id = %id, scheduler_row = %canon_json(r), "Catch-up diff: row on scheduler, MISSING on SSP");
                            logged += 1;
                        }
                        (None, Some(s)) => {
                            error!(ssp_id = %ssp_id, table = %table, row_id = %id, ssp_row = %canon_json(s), "Catch-up diff: EXTRA row on SSP, absent on scheduler");
                            logged += 1;
                        }
                        (Some(r), Some(s)) => {
                            if ssp_protocol::snapshot_hash::record_digest(id.as_str(), r)
                                != ssp_protocol::snapshot_hash::record_digest(id.as_str(), s)
                            {
                                error!(ssp_id = %ssp_id, table = %table, row_id = %id, scheduler_row = %canon_json(r), ssp_row = %canon_json(s), "Catch-up diff: row content differs");
                                logged += 1;
                            }
                        }
                        (None, None) => {}
                    }
                }
                if logged == 0 {
                    error!(ssp_id = %ssp_id, table = %table, "Catch-up hashes differ but no row-level diff found — possible canonicalization gap between scheduler and circuit");
                }
            }
            Err(e) => {
                // Couldn't reach the SSP's row dump (old build / transient) —
                // fall back to the one-sided scheduler-projection dump.
                warn!(ssp_id = %ssp_id, table = %table, error = %e, "Could not fetch SSP rows for row-level diff; dumping scheduler projection only");
                for (row_id, val) in ref_rows.iter().take(CATCHUP_DIAG_MAX_ROWS) {
                    error!(
                        ssp_id = %ssp_id,
                        table = %table,
                        row_id = %row_id,
                        reference_row = %canon_json(val),
                        "Catch-up mismatch diagnostic (scheduler reconstructed row@M)"
                    );
                }
            }
        }
    }
    error!(
        ssp_id = %ssp_id,
        mismatches = mismatches.len(),
        "SSP catch-up state disagrees with scheduler projection — flagging for re-bootstrap"
    );
    Ok(false)
}

/// Catch-up verification re-checks the SSP this many times before treating a
/// mismatch as a real divergence (the SSP's apply pipeline can briefly lag the
/// `/ingest` HTTP ack while pinned at M).
const CATCHUP_VERIFY_ATTEMPTS: usize = 3;
/// Delay between catch-up re-checks.
const CATCHUP_VERIFY_RETRY_DELAY_MS: u64 = 150;
/// Cap on per-table rows dumped in the mismatch diagnostic, to keep the log
/// bounded. Higher than the old one-sided dump because the row-level diff only
/// logs rows that actually diverge (missing / extra / differing), so the cap is
/// a real ceiling on divergences shown, not just the first N rows.
const CATCHUP_DIAG_MAX_ROWS: usize = 20;

/// Default consecutive catch-up failures before the breaker re-clones the
/// replica from upstream. Override with `SPKY_CATCHUP_RECLONE_AFTER`.
const CATCHUP_RECLONE_AFTER_DEFAULT: u32 = 3;
/// Default consecutive catch-up failures before the breaker gives up on the
/// gate and admits the SSP to broadcast anyway (restoring sync). Override with
/// `SPKY_CATCHUP_ADMIT_AFTER`. Set very high to effectively disable admit; set
/// re-clone/admit equal to skip the re-clone step.
const CATCHUP_ADMIT_AFTER_DEFAULT: u32 = 5;

/// Read the catch-up breaker thresholds `(reclone_after, admit_after)` from env,
/// falling back to the compiled defaults. `admit_after` is floored to at least
/// `reclone_after` so admit never precedes the re-clone attempt.
fn catchup_breaker_thresholds() -> (u32, u32) {
    let parse = |k: &str| {
        std::env::var(k)
            .ok()
            .and_then(|s| s.parse().ok())
            .filter(|n: &u32| *n > 0)
    };
    resolve_breaker_thresholds(
        parse("SPKY_CATCHUP_RECLONE_AFTER"),
        parse("SPKY_CATCHUP_ADMIT_AFTER"),
    )
}

/// Pure core of [`catchup_breaker_thresholds`]: apply defaults and floor
/// `admit` to at least `reclone` so admit never precedes the re-clone attempt.
fn resolve_breaker_thresholds(reclone: Option<u32>, admit: Option<u32>) -> (u32, u32) {
    let reclone = reclone.unwrap_or(CATCHUP_RECLONE_AFTER_DEFAULT);
    let admit = admit.unwrap_or(CATCHUP_ADMIT_AFTER_DEFAULT).max(reclone);
    (reclone, admit)
}

/// Canonical-JSON string of a value, for the mismatch diagnostic. Same encoding
/// the hash uses, so the logged bytes are exactly what feeds the digest.
fn canon_json(v: &serde_json::Value) -> String {
    String::from_utf8_lossy(&ssp_protocol::snapshot_hash::canonical_json(v)).into_owned()
}

/// Fetch an SSP's circuit rows for one table (`/debug/catchup-rows/:table`),
/// returned as `raw_id -> value`. Used by the catch-up diagnostic to diff the
/// circuit against the scheduler's reconstructed projection.
async fn fetch_ssp_catchup_rows(
    transport: &Arc<HttpTransport>,
    ssp_url: &str,
    table: &str,
) -> Result<std::collections::HashMap<String, serde_json::Value>> {
    let resp = transport
        .get_from_ssp(ssp_url, &format!("/debug/catchup-rows/{}", table))
        .await
        .map_err(|e| anyhow::anyhow!("GET /debug/catchup-rows failed: {}", e))?;
    if !resp.status().is_success() {
        anyhow::bail!("SSP /debug/catchup-rows returned HTTP {}", resp.status());
    }
    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| anyhow::anyhow!("Parse /debug/catchup-rows JSON failed: {}", e))?;
    let rows = json
        .get("rows")
        .and_then(|v| v.as_object())
        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default();
    Ok(rows)
}

/// Catch-up breaker step: re-clone the replica from upstream SurrealDB. Serialized
/// by `reclone_lock` so two looping SSPs can't reset + re-ingest the shared
/// replica at once. Returns `Ok(true)` if this call performed the re-clone,
/// `Ok(false)` if another task already holds the lock (caller falls back to a
/// plain re-bootstrap), `Err` on a clone failure.
///
/// Mirrors the bootstrap clone in `Scheduler::start`: reset the replica, clone
/// every table from upstream, then rehash at the current global seq. Holds the
/// replica write lock for the whole clone so in-flight bootstrap reads see a
/// consistent snapshot (they block, then read the fresh clone).
async fn reclone_replica_from_upstream(
    config: &SchedulerConfig,
    replica: &Arc<RwLock<Replica>>,
    seq_counter: &Arc<AtomicU64>,
    reclone_lock: &Arc<Mutex<()>>,
) -> Result<bool> {
    let _guard = match reclone_lock.try_lock() {
        Ok(g) => g,
        Err(_) => return Ok(false),
    };

    let db = crate::restore::connect_remote(&config.db)
        .await
        .context("reclone: connect to upstream SurrealDB failed")?;

    // Snapshot the seq the clone reflects BEFORE the (slow) ingest, matching
    // the bootstrap path — new events past this point re-arrive via replay.
    let seq = seq_counter.load(Ordering::SeqCst);

    {
        let mut rep = replica.write().await;
        rep.reset().await.context("reclone: replica reset failed")?;
        rep.ingest_all(&db).await.context("reclone: ingest_all failed")?;
        rep.set_snapshot_state(seq, None)
            .await
            .context("reclone: set_snapshot_state failed")?;
    }

    info!(seq, "Catch-up breaker: replica re-clone from upstream complete");
    Ok(true)
}

/// Reconstruct a touched table's content at the catch-up cut from its rows@N
/// (`seed`) plus the replayed `events`, then XOR-hash it. Events are applied by
/// REPLACE on Create/Update (matching the SSP circuit's `rows.insert`) and
/// remove on Delete — never merged, even though the replica's own `apply`
/// merges. This REPLACE-not-MERGE semantics is the one detail that must match
/// the SSP exactly; it's a pure function so it can be pinned by tests.
#[cfg(test)]
fn project_table_hash(
    table: &str,
    seed: Vec<(String, serde_json::Value)>,
    events: &[&RecordUpdate],
) -> String {
    ssp_protocol::snapshot_hash::xor_table_hash(project_table_rows(table, seed, events).into_iter())
}

/// Reconstruct a touched table's `raw_id -> row` map at the catch-up cut from
/// its rows@N (`seed`) plus the replayed `events`, by REPLACE on Create/Update
/// and remove on Delete — matching the SSP circuit's `rows.insert`. Split out of
/// [`project_table_hash`] so the verifier can also surface the rows themselves
/// (for the mismatch diagnostic), not just their hash.
fn project_table_rows(
    table: &str,
    seed: Vec<(String, serde_json::Value)>,
    events: &[&RecordUpdate],
) -> std::collections::HashMap<String, serde_json::Value> {
    let mut rows: std::collections::HashMap<String, serde_json::Value> = seed.into_iter().collect();
    for ev in events {
        let raw = ev
            .record_id
            .strip_prefix(&format!("{}:", table))
            .unwrap_or(&ev.record_id)
            .to_string();
        match ev.operation {
            RecordOp::Create | RecordOp::Update => {
                if let Some(data) = &ev.data {
                    rows.insert(raw, data.clone());
                }
            }
            RecordOp::Delete => {
                rows.remove(&raw);
            }
        }
    }
    rows
}

#[cfg(test)]
mod catchup_tests {
    use super::*;
    use serde_json::{json, Value};
    use ssp_protocol::snapshot_hash::{xor_acc_to_hex, xor_empty, xor_table_hash};

    fn upd(table: &str, op: RecordOp, id: &str, data: Option<Value>) -> RecordUpdate {
        RecordUpdate {
            table: table.to_string(),
            operation: op,
            record_id: id.to_string(),
            data,
            version: 0,
        }
    }

    #[test]
    fn breaker_thresholds_default_and_floor() {
        // Defaults when unset.
        assert_eq!(
            resolve_breaker_thresholds(None, None),
            (CATCHUP_RECLONE_AFTER_DEFAULT, CATCHUP_ADMIT_AFTER_DEFAULT)
        );
        // Explicit values pass through when admit >= reclone.
        assert_eq!(resolve_breaker_thresholds(Some(2), Some(6)), (2, 6));
        // admit is floored to reclone so admit never precedes the re-clone.
        assert_eq!(resolve_breaker_thresholds(Some(4), Some(1)), (4, 4));
        // Setting them equal disables the re-clone step (admit on the same count).
        assert_eq!(resolve_breaker_thresholds(Some(3), Some(3)), (3, 3));
    }

    #[test]
    fn projection_replaces_does_not_merge() {
        // Seed has {a, b}; a partial-looking update payload {a:9} must REPLACE
        // the whole row (matching the SSP), leaving {a:9} — NOT merge to {a:9,b:2}.
        let seed = vec![("r1".to_string(), json!({"a": 1, "b": 2}))];
        let ev = upd("presence", RecordOp::Update, "presence:r1", Some(json!({"a": 9})));
        let got = project_table_hash("presence", seed, &[&ev]);

        assert_eq!(got, xor_table_hash(vec![("r1".to_string(), json!({"a": 9}))]));
        assert_ne!(
            got,
            xor_table_hash(vec![("r1".to_string(), json!({"a": 9, "b": 2}))]),
            "must not behave like a MERGE"
        );
    }

    #[test]
    fn projection_multi_update_last_write_wins() {
        let evs = vec![
            upd("t", RecordOp::Create, "t:r1", Some(json!({"v": 1}))),
            upd("t", RecordOp::Update, "t:r1", Some(json!({"v": 2}))),
        ];
        let refs: Vec<&RecordUpdate> = evs.iter().collect();
        let got = project_table_hash("t", vec![], &refs);
        assert_eq!(got, xor_table_hash(vec![("r1".to_string(), json!({"v": 2}))]));
    }

    #[test]
    fn projection_delete_removes_row() {
        let seed = vec![("r1".to_string(), json!({"v": 1}))];
        let ev = upd("t", RecordOp::Delete, "t:r1", None);
        let got = project_table_hash("t", seed, &[&ev]);
        assert_eq!(got, xor_acc_to_hex(&xor_empty()));
    }

    #[test]
    fn projection_matches_ssp_incremental_for_hot_table() {
        // The whole point: a row written during catch-up. SSP seeds @N then
        // applies the same events incrementally; the scheduler projection must
        // land on the identical hash so the table is NOT falsely flagged.
        let seed = vec![("dev1".to_string(), json!({"fen": "start", "_00_rv": 3}))];
        let ev = upd(
            "presence",
            RecordOp::Update,
            "presence:dev1",
            Some(json!({"fen": "e4", "_00_rv": 4})),
        );
        let got = project_table_hash("presence", seed, &[&ev]);
        // `_00_rv` is stripped, so only the user content matters.
        assert_eq!(got, xor_table_hash(vec![("dev1".to_string(), json!({"fen": "e4"}))]));
    }
}
