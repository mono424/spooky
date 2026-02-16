use anyhow::Result;
use axum::{
    extract::State,
    http::StatusCode,
    routing::post,
    Json, Router,
};
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

use crate::config::SchedulerConfig;
use crate::messages::{BootstrapChunk, SspHeartbeat};
use crate::replica::Replica;
use crate::router::SspPool;
use crate::transport::{HttpTransport, SspInfo};

/// SSP registration request
#[derive(Debug, Deserialize)]
pub struct SspRegistration {
    pub ssp_id: String,
    pub url: String,
}

/// Shared state for SSP management handlers
#[derive(Clone)]
pub struct SspManagementState {
    pub ssp_pool: Arc<RwLock<SspPool>>,
    pub replica: Arc<RwLock<Replica>>,
    pub transport: Arc<HttpTransport>,
    pub config: Arc<SchedulerConfig>,
}

/// Create SSP management router
pub fn create_ssp_router(state: SspManagementState) -> Router {
    Router::new()
        .route("/ssp/register", post(handle_register))
        .route("/ssp/heartbeat", post(handle_heartbeat))
        .with_state(state)
}

/// Handle SSP registration
async fn handle_register(
    State(state): State<SspManagementState>,
    Json(request): Json<SspRegistration>,
) -> Result<StatusCode, (StatusCode, String)> {
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

    // Add to pool and mark as bootstrapping
    {
        let mut pool = state.ssp_pool.write().await;
        pool.upsert(ssp_info);
        pool.mark_bootstrapping(&request.ssp_id);
    }

    // Spawn async task to handle bootstrap (non-blocking)
    let ssp_id = request.ssp_id.clone();
    let ssp_url = request.url.clone();
    let replica = state.replica.clone();
    let ssp_pool = state.ssp_pool.clone();
    let transport = state.transport.clone();
    let config = state.config.clone();

    tokio::spawn(async move {
        if let Err(e) = bootstrap_ssp(ssp_id.clone(), ssp_url, replica, ssp_pool.clone(), transport, config).await {
            error!("Bootstrap failed for SSP '{}': {}", ssp_id, e);
            // Remove SSP on bootstrap failure
            let mut pool = ssp_pool.write().await;
            pool.remove(&ssp_id);
        }
    });

    info!("SSP registration accepted, bootstrap starting");
    Ok(StatusCode::ACCEPTED)
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

/// Bootstrap an SSP with replica data
async fn bootstrap_ssp(
    ssp_id: String,
    ssp_url: String,
    replica: Arc<RwLock<Replica>>,
    ssp_pool: Arc<RwLock<SspPool>>,
    transport: Arc<HttpTransport>,
    config: Arc<SchedulerConfig>,
) -> Result<()> {
    info!("Starting bootstrap for SSP: {}", ssp_id);

    // Read replica data and chunk it
    let chunks = {
        let replica = replica.read().await;
        replica.iter_chunks(config.bootstrap_chunk_size)?
    };

    let total_chunks = chunks.len();
    info!("Sending {} bootstrap chunks to SSP '{}'", total_chunks, ssp_id);

    // Send each chunk to SSP
    for (chunk_index, chunk) in chunks.into_iter().enumerate() {
        let bootstrap_chunk = BootstrapChunk {
            chunk_index,
            total_chunks,
            table: chunk.table,
            records: chunk.records,
        };

        transport
            .post_to_ssp(&ssp_url, "/bootstrap", &bootstrap_chunk)
            .await?;

        info!(
            "Sent bootstrap chunk {}/{} to SSP '{}'",
            chunk_index + 1,
            total_chunks,
            ssp_id
        );
    }

    info!("Bootstrap complete for {}, marking as ready", ssp_id);

    // Mark SSP as ready and get buffered messages
    let buffered_messages = {
        let mut pool = ssp_pool.write().await;
        pool.mark_ready(&ssp_id)
    };

    // Replay buffered messages
    if !buffered_messages.is_empty() {
        info!(
            "Replaying {} buffered messages to SSP '{}'",
            buffered_messages.len(),
            ssp_id
        );

        for message in buffered_messages {
            // Convert RecordUpdate to IngestRequest format
            let ingest_payload = serde_json::json!({
                "table": message.table,
                "op": message.operation.to_string(),
                "id": message.record_id,
                "record": message.data.unwrap_or(serde_json::json!({}))
            });

            if let Err(e) = transport.post_to_ssp(&ssp_url, "/ingest", &ingest_payload).await {
                warn!(
                    "Failed to replay buffered message to SSP '{}': {}",
                    ssp_id, e
                );
            }
        }

        info!("Buffered messages replayed to SSP '{}'", ssp_id);
    }

    info!("SSP '{}' is now ready", ssp_id);
    Ok(())
}
