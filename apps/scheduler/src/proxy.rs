use axum::{
    extract::State,
    http::StatusCode,
    routing::post,
    Json, Router,
};
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, trace};

use crate::replica::Replica;
use crate::SchedulerStatus;

/// Request to execute a SurrealQL query against the snapshot DB
#[derive(Debug, Deserialize)]
pub struct ProxyQueryRequest {
    pub query: String,
}

/// Shared state for proxy handlers
#[derive(Clone)]
pub struct ProxyState {
    pub replica: Arc<RwLock<Replica>>,
    pub status: Arc<RwLock<SchedulerStatus>>,
}

/// Create proxy router for SSP bootstrap
pub fn create_proxy_router(state: ProxyState) -> Router {
    Router::new()
        .route("/proxy/query", post(handle_proxy_query))
        .route("/proxy/signin", post(handle_proxy_signin))
        .route("/proxy/use", post(handle_proxy_use))
        .with_state(state)
}

async fn reject_if_restoring(
    status: &Arc<RwLock<SchedulerStatus>>,
) -> Result<(), (StatusCode, String)> {
    if *status.read().await == SchedulerStatus::Restoring {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "Scheduler is restoring from backup".to_string(),
        ));
    }
    Ok(())
}

/// Handle a SurrealQL query forwarded to the snapshot DB
async fn handle_proxy_query(
    State(state): State<ProxyState>,
    Json(request): Json<ProxyQueryRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    reject_if_restoring(&state.status).await?;

    trace!(query = %request.query, "proxy query (forwarded to local replica)");

    let replica = state.replica.read().await;
    match replica.query(&request.query).await {
        Ok(result) => Ok(Json(result)),
        Err(e) => {
            error!(error = %e, query = %request.query, "Proxy query failed");
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Query failed: {}", e),
            ))
        }
    }
}

/// No-op signin — snapshot DB doesn't need auth
async fn handle_proxy_signin(
    State(state): State<ProxyState>,
) -> Result<StatusCode, (StatusCode, String)> {
    reject_if_restoring(&state.status).await?;
    Ok(StatusCode::OK)
}

/// No-op namespace/db selection — already configured
async fn handle_proxy_use(
    State(state): State<ProxyState>,
) -> Result<StatusCode, (StatusCode, String)> {
    reject_if_restoring(&state.status).await?;
    Ok(StatusCode::OK)
}
