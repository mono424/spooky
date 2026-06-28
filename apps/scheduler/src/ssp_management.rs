use anyhow::{Context, Result};
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
        Ok(true) => true,
        Ok(false) => {
            // Real divergence: flag for forced re-bootstrap and do NOT mark
            // ready. The SSP (already heartbeating) gets a 409 on its next
            // heartbeat and exits for a clean re-registration.
            ssp_pool.write().await.mark_for_resync(&ssp_id);
            warn!(ssp_id = %ssp_id, "Catch-up verification failed — withholding from broadcast; SSP will re-bootstrap");
            false
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

    info!("SSP '{}' is now fully caught up and ready", ssp_id);
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

    // Persistent mismatch after every attempt → a real divergence.
    for (table, sched, ssp_h) in &mismatches {
        error!(
            ssp_id = %ssp_id,
            table = %table,
            scheduler = %sched,
            ssp = %ssp_h,
            "Catch-up hash mismatch"
        );
        // Diagnostic: dump the canonical JSON of the scheduler's reconstructed
        // rows (capped) so a representation gap (e.g. a `null` optional the SSP
        // omits) is visible in the log without attaching a debugger.
        if let Some(rows) = reference_rows.get(table) {
            for (row_id, val) in rows.iter().take(CATCHUP_DIAG_MAX_ROWS) {
                let canon = String::from_utf8_lossy(
                    &ssp_protocol::snapshot_hash::canonical_json(val),
                )
                .into_owned();
                error!(
                    ssp_id = %ssp_id,
                    table = %table,
                    row_id = %row_id,
                    reference_row = %canon,
                    "Catch-up mismatch diagnostic (scheduler reconstructed row@M)"
                );
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
/// Cap on per-table rows dumped in the mismatch diagnostic, to keep the log bounded.
const CATCHUP_DIAG_MAX_ROWS: usize = 3;

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
