//! Long-lived tokens for MCP clients.
//!
//! A token is an ordinary session with a lifetime of days, a scope, and the
//! `Mcp` mode. With `SPKY_AUTH_SECRET` set it is a signed token that survives
//! scheduler restarts; without one it lives in the in-memory store and dies
//! with the process, which `/admin/api/config.sessions_persistent` says and
//! the dashboard repeats before minting.
//!
//! Nothing is stored about a minted token, so there is no list endpoint; the
//! operator keeps the token, and revocation is by presenting it back.

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::{Extension, Json};
use serde::Deserialize;
use serde_json::{json, Value};
use tracing::info;

use super::session::{Scope, SessionMode};
use super::{api_error, AdminState, ApiError, CurrentSession};

const DEFAULT_TTL_DAYS: u64 = 90;
const MAX_TTL_DAYS: u64 = 365;

#[derive(Debug, Deserialize)]
pub struct MintRequest {
    pub label: String,
    #[serde(default = "default_scope")]
    pub scope: String,
    #[serde(default)]
    pub ttl_days: Option<u64>,
}

fn default_scope() -> String {
    "read".to_string()
}

/// `POST /admin/api/tokens`
pub async fn mint(
    State(state): State<AdminState>,
    Extension(CurrentSession(session)): Extension<CurrentSession>,
    Json(req): Json<MintRequest>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    // A token that can mint tokens is a token that never expires. Only a
    // person's own session may do it.
    if session.mode == SessionMode::Mcp {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "An MCP token cannot mint further tokens; sign in to the dashboard to create one",
        ));
    }
    if session.scope != Scope::Full {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "This session is read-only",
        ));
    }
    let label = req.label.trim();
    if label.is_empty() || label.len() > 64 {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "A label of 1 to 64 characters is required",
        ));
    }
    let scope = Scope::parse(&req.scope).ok_or_else(|| {
        api_error(
            StatusCode::BAD_REQUEST,
            "scope must be \"read\" or \"full\"",
        )
    })?;
    let ttl_days = req.ttl_days.unwrap_or(DEFAULT_TTL_DAYS);
    if ttl_days == 0 || ttl_days > MAX_TTL_DAYS {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            format!("ttl_days must be between 1 and {MAX_TTL_DAYS}"),
        ));
    }
    let ttl = std::time::Duration::from_secs(ttl_days * 86_400);
    let token = state.sessions.create_with(
        format!("mcp:{label}"),
        label.to_string(),
        SessionMode::Mcp,
        scope,
        ttl,
    );
    let expires_at = chrono::Utc::now() + chrono::Duration::seconds(ttl.as_secs() as i64);
    info!(label, ?scope, ttl_days, by = %session.subject, "MCP token minted");
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "token": token,
            "label": label,
            "scope": scope,
            "expires_at": expires_at.to_rfc3339(),
            "endpoint": "/admin/api/mcp",
            "persistent": state.sessions.persistent(),
        })),
    ))
}

#[derive(Debug, Deserialize)]
pub struct RevokeRequest {
    pub token: String,
}

/// `DELETE /admin/api/tokens`
pub async fn revoke(
    State(state): State<AdminState>,
    Extension(CurrentSession(session)): Extension<CurrentSession>,
    req: Request,
) -> Result<StatusCode, ApiError> {
    let body = axum::body::to_bytes(req.into_body(), 64 * 1024)
        .await
        .map_err(|_| api_error(StatusCode::BAD_REQUEST, "Unreadable body"))?;
    let RevokeRequest { token } = serde_json::from_slice(&body)
        .map_err(|_| api_error(StatusCode::BAD_REQUEST, "Expected {\"token\": \"...\"}"))?;
    let token = token.trim();
    if token.is_empty() {
        return Err(api_error(StatusCode::BAD_REQUEST, "token is required"));
    }
    // Revoking an unknown or already-dead token is not an error; the
    // outcome the operator wants (this token does not work) is true.
    let known = state.sessions.get(token).map(|s| s.label);
    state.sessions.revoke(token);
    info!(revoked = ?known, by = %session.subject, "Token revoked");
    Ok(StatusCode::NO_CONTENT)
}
