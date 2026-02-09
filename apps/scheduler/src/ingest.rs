use anyhow::Result;
use axum::{
    extract::State,
    http::StatusCode,
    routing::post,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info};

use crate::messages::RecordUpdate;
use crate::replica::{Replica, RecordOp};
use crate::router::SspPool;
use crate::transport::Transport;

/// Ingest request from database events
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestRequest {
    pub table: String,
    pub operation: RecordOp,
    pub record_id: String,
    pub data: Option<Value>,
}

/// Shared state for ingest handlers
#[derive(Clone)]
pub struct IngestState {
    pub replica: Arc<RwLock<Replica>>,
    pub transport: Arc<dyn Transport>,
    pub ssp_pool: Arc<RwLock<SspPool>>,
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
    info!(
        "Received ingest: {} {} on {}",
        request.operation, request.record_id, request.table
    );

    // Update replica
    {
        let mut replica = state.replica.write().await;
        if let Err(e) = replica.apply(
            &request.table,
            request.operation,
            &request.record_id,
            request.data.clone(),
        ) {
            error!("Failed to apply to replica: {}", e);
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to apply to replica: {}", e),
            ));
        }
    }

    // Get version from replica
    let version = {
        let replica = state.replica.read().await;
        match replica.get_current_version(&request.table, &request.record_id) {
            Ok(v) => v,
            Err(e) => {
                error!("Failed to get version: {}", e);
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to get version: {}", e),
                ));
            }
        }
    };

    // Create update message
    let update = RecordUpdate {
        table: request.table.clone(),
        operation: request.operation,
        record_id: request.record_id.clone(),
        data: request.data,
        version,
    };

    // Route to SSPs based on state
    let mut pool = state.ssp_pool.write().await;
    
    // Collect SSPs and their states before routing (avoid borrow checker issues)
    let ssp_states: Vec<(String, bool)> = pool
        .all()
        .iter()
        .map(|ssp| (ssp.id.clone(), pool.is_ready(&ssp.id)))
        .collect();
    
    // Broadcast to ready SSPs
    let ready_ssps: Vec<_> = ssp_states
        .iter()
        .filter(|(_, is_ready)| *is_ready)
        .collect();
    
    if !ready_ssps.is_empty() {
        let payload = match serde_json::to_vec(&update) {
            Ok(p) => p,
            Err(e) => {
                error!("Failed to serialize update: {}", e);
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to serialize update: {}", e),
                ));
            }
        };

        if let Err(e) = state
            .transport
            .broadcast(&format!("spooky.ingest.{}", request.table), &payload)
            .await
        {
            error!("Failed to broadcast to SSPs: {}", e);
        }
    }
    
    // Buffer for bootstrapping SSPs
    for (ssp_id, is_ready) in ssp_states {
        if !is_ready {
            if !pool.buffer_message(&ssp_id, update.clone()) {
                error!(
                    "Buffer overflow for SSP '{}' - needs re-bootstrap",
                    ssp_id
                );
                // TODO: Implement re-bootstrap trigger mechanism
            }
        }
    }

    info!("Ingest processed successfully");
    Ok(StatusCode::OK)
}
