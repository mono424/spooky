use anyhow::Result;
use axum::{
    extract::State,
    http::StatusCode,
    routing::post,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info};

use crate::router::SspPool;
use crate::transport::Transport;

/// Query registration request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryRegistration {
    pub query_id: String,
    pub client_id: String,
    pub tables: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<u8>,
}

/// Query assignment response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryAssignment {
    pub query_id: String,
    pub ssp_id: String,
    pub assigned_at: u64,
}

/// Query tracker state
#[derive(Clone)]
pub struct QueryTracker {
    /// Map query_id -> ssp_id
    assignments: Arc<RwLock<HashMap<String, String>>>,
}

impl QueryTracker {
    pub fn new() -> Self {
        Self {
            assignments: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Assign a query to an SSP
    pub async fn assign(&self, query_id: String, ssp_id: String) {
        let mut assignments = self.assignments.write().await;
        assignments.insert(query_id, ssp_id);
    }

    /// Get SSP assigned to a query
    pub async fn get_assignment(&self, query_id: &str) -> Option<String> {
        let assignments = self.assignments.read().await;
        assignments.get(query_id).cloned()
    }

    /// Unassign a query (when client disconnects)
    pub async fn unassign(&self, query_id: &str) {
        let mut assignments = self.assignments.write().await;
        assignments.remove(query_id);
    }

    /// Unassign all queries from an SSP (when SSP disconnects)
    pub async fn unassign_ssp(&self, ssp_id: &str) -> Vec<String> {
        let mut assignments = self.assignments.write().await;
        let removed: Vec<String> = assignments
            .iter()
            .filter(|(_, sid)| *sid == ssp_id)
            .map(|(qid, _)| qid.clone())
            .collect();
        
        for qid in &removed {
            assignments.remove(qid);
        }
        
        removed
    }

    /// Get all assignments
    pub async fn all(&self) -> HashMap<String, String> {
        let assignments = self.assignments.read().await;
        assignments.clone()
    }
}

/// Shared state for query handlers
#[derive(Clone)]
pub struct QueryState {
    pub ssp_pool: Arc<RwLock<SspPool>>,
    pub transport: Arc<dyn Transport>,
    pub query_tracker: Arc<QueryTracker>,
}

/// Create query router
pub fn create_query_router(state: QueryState) -> Router {
    Router::new()
        .route("/query/register", post(register_query))
        .route("/query/unregister", post(unregister_query))
        .with_state(state)
}

/// Handle query registration
async fn register_query(
    State(state): State<QueryState>,
    Json(request): Json<QueryRegistration>,
) -> Result<Json<QueryAssignment>, (StatusCode, String)> {
    info!("Registering query: {}", request.query_id);

    // Select SSP based on load balancing strategy
    let ssp_id = {
        let mut pool = state.ssp_pool.write().await;
        match pool.select_for_query() {
            Some(id) => {
                // Increment query count for the selected SSP
                pool.increment_query_count(&id);
                id
            }
            None => {
                error!("No ready SSP available for query {}", request.query_id);
                return Err((
                    StatusCode::SERVICE_UNAVAILABLE,
                    "No SSP available".to_string(),
                ));
            }
        }
    };

    // Assign query to SSP in tracker
    state.query_tracker.assign(request.query_id.clone(), ssp_id.clone()).await;

    // Send registration to SSP via queue group
    let payload = match serde_json::to_vec(&request) {
        Ok(p) => p,
        Err(e) => {
            error!("Failed to serialize query registration: {}", e);
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Serialization error: {}", e),
            ));
        }
    };

    if let Err(e) = state
        .transport
        .send_to(&ssp_id, "query.register", &payload)
        .await
    {
        error!("Failed to send query registration to SSP: {}", e);
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to send to SSP: {}", e),
        ));
    }

    let assignment = QueryAssignment {
        query_id: request.query_id,
        ssp_id,
        assigned_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    };

    info!("Assigned query {} to SSP {}", assignment.query_id, assignment.ssp_id);
    Ok(Json(assignment))
}

/// Unregister query request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryUnregistration {
    pub query_id: String,
}

/// Handle query unregistration
async fn unregister_query(
    State(state): State<QueryState>,
    Json(request): Json<QueryUnregistration>,
) -> Result<StatusCode, (StatusCode, String)> {
    info!("Unregistering query: {}", request.query_id);

    // Get SSP assignment
    let ssp_id = match state.query_tracker.get_assignment(&request.query_id).await {
        Some(id) => id,
        None => {
            return Err((
                StatusCode::NOT_FOUND,
                format!("Query {} not found", request.query_id),
            ));
        }
    };

    // Send unregistration to SSP
    let payload = match serde_json::to_vec(&request) {
        Ok(p) => p,
        Err(e) => {
            error!("Failed to serialize query unregistration: {}", e);
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Serialization error: {}", e),
            ));
        }
    };

    if let Err(e) = state
        .transport
        .send_to(&ssp_id, "query.unregister", &payload)
        .await
    {
        error!("Failed to send query unregistration to SSP: {}", e);
    }

    // Decrement query count
    {
        let mut pool = state.ssp_pool.write().await;
        pool.decrement_query_count(&ssp_id);
    }

    // Unassign from tracker
    state.query_tracker.unassign(&request.query_id).await;

    info!("Unregistered query {}", request.query_id);
    Ok(StatusCode::OK)
}
