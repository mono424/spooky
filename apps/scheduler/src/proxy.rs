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
use tracing::{debug, error};

use crate::replica::Replica;

/// Request to execute a SurrealQL query against the snapshot DB
#[derive(Debug, Deserialize)]
pub struct ProxyQueryRequest {
    pub query: String,
}

/// Shared state for proxy handlers
#[derive(Clone)]
pub struct ProxyState {
    pub replica: Arc<RwLock<Replica>>,
}

/// Create proxy router for SSP bootstrap
pub fn create_proxy_router(state: ProxyState) -> Router {
    Router::new()
        .route("/proxy/query", post(handle_proxy_query))
        .route("/proxy/signin", post(handle_proxy_signin))
        .route("/proxy/use", post(handle_proxy_use))
        .with_state(state)
}

/// Handle a SurrealQL query forwarded to the snapshot DB
async fn handle_proxy_query(
    State(state): State<ProxyState>,
    Json(request): Json<ProxyQueryRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    debug!("Proxy query: {}", request.query);

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
async fn handle_proxy_signin() -> StatusCode {
    StatusCode::OK
}

/// No-op namespace/db selection — already configured
async fn handle_proxy_use() -> StatusCode {
    StatusCode::OK
}
