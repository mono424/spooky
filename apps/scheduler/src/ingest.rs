use anyhow::Result;
use axum::{
    extract::State,
    http::StatusCode,
    routing::post,
    Json, Router,
};
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;
use tracing::{error, info, warn};

use crate::messages::{BufferedEvent, RecordUpdate, RecordOp};
use crate::replica::Replica;
use crate::router::SspPool;
use crate::transport::HttpTransport;
use crate::wal::EventWal;
use crate::SchedulerStatus;
use ssp_protocol::IngestRequest;

/// Shared state for ingest handlers
#[derive(Clone)]
pub struct IngestState {
    pub replica: Arc<RwLock<Replica>>,
    pub transport: Arc<HttpTransport>,
    pub ssp_pool: Arc<RwLock<SspPool>>,
    pub status: Arc<RwLock<SchedulerStatus>>,
    pub event_buffer: Arc<RwLock<VecDeque<BufferedEvent>>>,
    pub seq_counter: Arc<AtomicU64>,
    pub wal: Arc<RwLock<EventWal>>,
}

/// Snapshot of how far behind the replica is vs. the ingest stream.
/// `pending_events` are durable in the WAL but not yet applied to the replica.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PendingEventsStat {
    pub pending_events: usize,
    pub snapshot_seq: u64,
    pub latest_seq: u64,
    pub lag: u64,
}

/// Cheap, non-blocking read of the in-memory buffer + counters.
pub async fn pending_events_snapshot(state: &IngestState) -> PendingEventsStat {
    let pending_events = state.event_buffer.read().await.len();
    let snapshot_seq = state.replica.read().await.snapshot_seq();
    let latest_seq = state.seq_counter.load(Ordering::SeqCst);
    let lag = latest_seq.saturating_sub(snapshot_seq);
    PendingEventsStat {
        pending_events,
        snapshot_seq,
        latest_seq,
        lag,
    }
}

/// Create ingest router
pub fn create_ingest_router(state: IngestState) -> Router {
    Router::new()
        .route("/ingest", post(handle_ingest))
        .with_state(state)
}

/// Handle ingest requests from database events
async fn handle_ingest(
    State(state): State<IngestState>,
    Json(request): Json<IngestRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    // Gate: reject if scheduler is cloning
    let scheduler_status = *state.status.read().await;
    if scheduler_status == SchedulerStatus::Cloning {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "SSP_NOT_READY: Scheduler is cloning database".to_string(),
        ));
    }

    info!(
        "Received ingest: {} {} on {}",
        request.op, request.id, request.table
    );

    // Parse operation
    let operation = match request.op.to_uppercase().as_str() {
        "CREATE" => RecordOp::Create,
        "UPDATE" => RecordOp::Update,
        "DELETE" => RecordOp::Delete,
        _ => {
            error!("Invalid operation: {}", request.op);
            return Err((
                StatusCode::BAD_REQUEST,
                format!("Invalid operation: {}", request.op),
            ));
        }
    };

    // Assign monotonic sequence number
    let seq = state.seq_counter.fetch_add(1, Ordering::SeqCst) + 1;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    // Create the buffered event
    let record_update = RecordUpdate {
        table: request.table.clone(),
        operation,
        record_id: request.id.clone(),
        data: Some(request.record.clone()),
        version: seq,
    };

    let buffered_event = BufferedEvent {
        seq,
        update: record_update,
        received_at: now,
    };

    // Write-ahead: append to WAL before processing
    {
        let mut wal = state.wal.write().await;
        if let Err(e) = wal.append(&buffered_event) {
            error!(error = %e, "Failed to write to WAL");
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("WAL write failed: {}", e),
            ));
        }
    }

    // Append to in-memory event buffer
    {
        let mut buffer = state.event_buffer.write().await;
        buffer.push_back(buffered_event.clone());
    }

    // Select one SSP for job execution (round-robin)
    let job_assignee = {
        let mut pool = state.ssp_pool.write().await;
        pool.select_for_query()
    };

    info!(
        table = %request.table,
        op = %request.op,
        record_id = %request.id,
        job_assignee = ?job_assignee,
        "Ingest: job assignee selected for event"
    );

    // Set assignee on request before broadcast
    let mut request = request;
    request.job_assignee = job_assignee;

    // Get all ready SSPs and broadcast
    let ready_ssps = {
        let pool = state.ssp_pool.read().await;
        pool.all()
            .into_iter()
            .filter(|ssp| pool.is_ready(&ssp.id))
            .cloned()
            .collect::<Vec<_>>()
    };

    if !ready_ssps.is_empty() {
        info!("Broadcasting to {} ready SSPs", ready_ssps.len());
        let results = state
            .transport
            .broadcast_to_ssps(&ready_ssps, "/ingest", &request)
            .await;

        for (ssp_id, result) in results {
            if let Err(e) = result {
                error!("Failed to send to SSP '{}': {}", ssp_id, e);
            }
        }
    }

    // Buffer for bootstrapping SSPs
    {
        let mut pool = state.ssp_pool.write().await;
        let bootstrapping_ids: Vec<String> = pool
            .all()
            .iter()
            .filter(|ssp| !pool.is_ready(&ssp.id))
            .map(|ssp| ssp.id.clone())
            .collect();

        for ssp_id in bootstrapping_ids {
            let update = RecordUpdate {
                table: request.table.clone(),
                operation,
                record_id: request.id.clone(),
                data: Some(request.record.clone()),
                version: seq,
            };
            if !pool.buffer_message(&ssp_id, update) {
                warn!("Buffer overflow for SSP '{}', needs re-bootstrap", ssp_id);
            }
        }
    }

    info!(seq, "Ingest processed successfully");
    Ok(StatusCode::OK)
}
