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

use crate::replica::{Replica, RecordOp};
use crate::router::SspPool;
use crate::transport::HttpTransport;

/// Ingest request from database events (matches SSP's IngestRequest format)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestRequest {
    pub table: String,
    pub op: String,
    pub id: String,
    pub record: Value,
}

/// Shared state for ingest handlers
#[derive(Clone)]
pub struct IngestState {
    pub replica: Arc<RwLock<Replica>>,
    pub transport: Arc<HttpTransport>,
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

    // Update replica
    {
        let mut replica = state.replica.write().await;
        if let Err(e) = replica.apply(
            &request.table,
            operation,
            &request.id,
            Some(request.record.clone()),
        ) {
            error!("Failed to apply to replica: {}", e);
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to apply to replica: {}", e),
            ));
        }
    }

    // Get all ready SSPs
    let ready_ssps = {
        let pool = state.ssp_pool.read().await;
        pool.all()
            .into_iter()
            .filter(|ssp| pool.is_ready(&ssp.id))
            .cloned()
            .collect::<Vec<_>>()
    };

    // Broadcast to all ready SSPs via HTTP POST /ingest
    if !ready_ssps.is_empty() {
        info!("Broadcasting to {} ready SSPs", ready_ssps.len());
        let results = state
            .transport
            .broadcast_to_ssps(&ready_ssps, "/ingest", &request)
            .await;

        // Log failures
        for (ssp_id, result) in results {
            if let Err(e) = result {
                error!("Failed to send to SSP '{}': {}", ssp_id, e);
            }
        }
    }

    // For bootstrapping SSPs, buffer the update
    // (This logic moved to router module's buffering mechanism)

    info!("Ingest processed successfully");
    Ok(StatusCode::OK)
}
