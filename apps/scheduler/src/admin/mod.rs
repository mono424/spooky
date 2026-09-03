//! The admin plane: a Solid dashboard plus the JSON API behind it, served on
//! their own listener.
//!
//! # Why a second port
//!
//! Everything on the ingest port is unauthenticated by design — `/ingest`,
//! `/proxy/query` (arbitrary SurrealQL against the replica), `/ssp/*`,
//! `PUT /backends` — because that port is only ever supposed to be reachable
//! from inside the deployment's private network. The dashboard is the first
//! surface here meant for a human browser, so it lives on its own listener
//! that can be published without publishing any of the above.
//!
//! # Why a bearer token rather than a cookie
//!
//! The same bundle has to work embedded (served from this scheduler) and
//! standalone (opened from anywhere, pointed at a scheduler by URL). A
//! cross-origin cookie needs `SameSite=None; Secure`, which no browser will
//! accept against a plain-http scheduler address — exactly the standalone case.
//! A bearer token in `localStorage` works identically in both, and takes CSRF
//! out of the picture entirely.

pub mod auth;
pub mod backends;
pub mod config;
pub mod logs;
pub mod overview;
pub mod session;
pub mod workflows;

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::json;
use tokio::sync::RwLock;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::{ServeDir, ServeFile};
use tracing::{info, warn};

pub use config::AdminConfig;

use maintenance::db::{DbConfig, ReconnectingDb};
use maintenance::log_ring::LogRing;

use self::auth::RateLimiter;
use self::session::{Session, SessionStore};
use self::workflows::WorkflowWatcher;

/// A handle to the scheduler's shared root database connection, published once
/// `Scheduler::start()` has built it.
///
/// It has to be late-bound: the HTTP servers come up before `start()` runs (so
/// health checks answer during the initial replica clone), but the database
/// handle is only created part-way through `start()`. Handlers that need it
/// answer 503 until it lands, which is the same thing every other endpoint does
/// while the scheduler is still `Cloning`.
pub type SharedDbSlot = Arc<RwLock<Option<Arc<ReconnectingDb>>>>;

pub fn new_db_slot() -> SharedDbSlot {
    Arc::new(RwLock::new(None))
}

#[derive(Clone)]
pub struct AdminState {
    pub config: Arc<AdminConfig>,
    pub metrics: crate::metrics::MetricsState,
    pub transport: Arc<crate::transport::HttpTransport>,
    pub sessions: Arc<SessionStore>,
    pub rate_limiter: Arc<RateLimiter>,
    pub logs: Arc<LogRing>,
    pub workflows: Arc<WorkflowWatcher>,
    pub db_config: Arc<DbConfig>,
    db_slot: SharedDbSlot,
    /// Cached copy of the slot, so the hot handlers don't take a lock per
    /// request once the handle exists.
    db_cache: Arc<std::sync::RwLock<Option<Arc<ReconnectingDb>>>>,
    pub bootstrap_timeout_secs: u64,
    pub health_check_interval_secs: u64,
}

impl AdminState {
    /// The shared root handle, or `None` while the scheduler is still starting.
    pub fn db(&self) -> Option<Arc<ReconnectingDb>> {
        if let Some(db) = self
            .db_cache
            .read()
            .expect("admin db cache poisoned")
            .clone()
        {
            return Some(db);
        }
        // Not cached yet: consult the slot. `try_read` rather than a blocking
        // read because this is called from async handlers — a miss simply means
        // "not ready yet", which is already a state every caller handles.
        let found = self.db_slot.try_read().ok().and_then(|g| g.clone())?;
        *self.db_cache.write().expect("admin db cache poisoned") = Some(Arc::clone(&found));
        Some(found)
    }
}

/// The error shape every admin endpoint returns.
///
/// One shape across the whole plane, so the dashboard can always read
/// `{"error": "..."}` and show the server's own words. A handler that returned
/// a bare string here would surface in the UI as a generic "Request failed",
/// throwing away the explanation it took the trouble to write.
pub type ApiError = (StatusCode, Json<serde_json::Value>);

pub fn api_error(status: StatusCode, message: impl std::fmt::Display) -> ApiError {
    (status, Json(json!({ "error": message.to_string() })))
}

/// The signed-in session, extracted by [`require_session`] and attached to the
/// request for handlers that want to know who is asking.
#[derive(Clone)]
pub struct CurrentSession(pub Session);

/// Bearer-token gate for `/admin/api/*`, minus the endpoints registered
/// outside it (`/config` and `/session`).
async fn require_session(
    State(state): State<AdminState>,
    mut req: Request,
    next: Next,
) -> Response {
    let token = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim);

    match token.and_then(|t| state.sessions.get(t)) {
        Some(session) => {
            req.extensions_mut().insert(CurrentSession(session));
            next.run(req).await
        }
        None => (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "Not signed in" })),
        )
            .into_response(),
    }
}

/// `GET /admin/api/config` — the only unauthenticated endpoint.
///
/// It exists so the frontend can tell embedded from standalone: a successful
/// same-origin response means it is being served by a scheduler and should hide
/// its endpoint field. It reveals nothing an unauthenticated caller of `/health`
/// on the ingest port could not already see.
async fn config_handler(State(state): State<AdminState>) -> Json<serde_json::Value> {
    Json(json!({
        "scheduler_id": state.metrics.scheduler_id,
        "version": env!("CARGO_PKG_VERSION"),
        "breakglass_available": state.config.password.is_some(),
    }))
}

/// `GET /admin/api/me` — who the current token belongs to.
async fn me_handler(
    axum::Extension(CurrentSession(session)): axum::Extension<CurrentSession>,
) -> Json<serde_json::Value> {
    Json(json!({
        "subject": session.subject,
        "label": session.label,
        "mode": session.mode,
    }))
}

/// `POST /admin/api/logout` — drop the current token server-side.
async fn logout_handler(
    State(state): State<AdminState>,
    req: Request,
) -> StatusCode {
    if let Some(token) = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
    {
        state.sessions.revoke(token.trim());
    }
    StatusCode::NO_CONTENT
}

/// Placeholder served when no dashboard bundle is present.
///
/// A scheduler built with `cargo build` and run from a checkout has no
/// `dist/` — that must be a readable message, not a 404 storm or a failed
/// startup.
async fn missing_dashboard() -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        concat!(
            "<!doctype html><meta charset=utf-8>",
            "<title>Sp00ky admin</title>",
            "<body style=\"font:14px system-ui;margin:3rem auto;max-width:32rem;color:#ddd;background:#141210\">",
            "<h1 style=\"font-size:1.1rem\">Dashboard not bundled</h1>",
            "<p>This scheduler has no dashboard build at its <code>SPKY_ADMIN_DIR</code>.</p>",
            "<p>Build it with <code>pnpm --filter @spooky-sync/dashboard build</code> and point ",
            "<code>SPKY_ADMIN_DIR</code> at <code>apps/dashboard/dist</code>, or use an official ",
            "scheduler image, which ships one.</p>",
            "<p>The API on this port is unaffected: <code>/admin/api/config</code> still answers.</p>",
            "</body>",
        ),
    )
}

/// Build the admin router: JSON API plus the static dashboard.
pub fn create_admin_router(state: AdminState) -> Router {
    // Origin `*` with no credentials: the token travels in a header the browser
    // only sends when the page has it, so a hostile page gains nothing here.
    // Allowing credentials with `*` is what would be unsafe, and is not done.
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE])
        .allow_methods(Any);

    let protected = Router::new()
        .route("/me", get(me_handler))
        .route("/logout", post(logout_handler))
        .route("/overview", get(overview::overview))
        .route("/backends", get(backends::list))
        .route("/backends/:name", get(backends::detail))
        .route("/logs", get(logs::stream))
        .route("/workflows/runs", get(workflows::list_runs))
        .route("/workflows/runs/:id", get(workflows::run_detail))
        .route("/workflows/stream", get(workflows::stream_runs))
        .route("/schedules", get(workflows::list_schedules))
        .route("/schedules/:name", get(workflows::schedule_detail))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            require_session,
        ));

    let api = Router::new()
        .route("/config", get(config_handler))
        .route("/session", post(auth::login))
        .merge(protected)
        .layer(cors)
        .with_state(state.clone());

    // Serve the built dashboard, falling back to index.html so client-side
    // routes survive a reload or a deep link.
    let index = state.config.dir.join("index.html");
    let assets = if index.is_file() {
        info!(dir = %state.config.dir.display(), "Serving admin dashboard");
        get_service_router(ServeDir::new(&state.config.dir).fallback(ServeFile::new(index)))
    } else {
        warn!(
            dir = %state.config.dir.display(),
            "No dashboard bundle found; /admin serves a placeholder (the API still works)"
        );
        Router::new().fallback(missing_dashboard)
    };

    Router::new()
        .nest("/admin/api", api)
        .nest_service("/admin", assets)
        // Bare `/` is a convenience: someone who types the host and port
        // without a path means the dashboard.
        .route(
            "/",
            get(|| async { axum::response::Redirect::permanent("/admin/") }),
        )
}

fn get_service_router(svc: ServeDir<ServeFile>) -> Router {
    Router::new().fallback_service(svc)
}

/// Assemble the admin state and its router.
#[allow(clippy::too_many_arguments)]
pub fn build(
    config: AdminConfig,
    metrics: crate::metrics::MetricsState,
    transport: Arc<crate::transport::HttpTransport>,
    logs: Arc<LogRing>,
    db_config: DbConfig,
    db_slot: SharedDbSlot,
    bootstrap_timeout_secs: u64,
    health_check_interval_secs: u64,
) -> (AdminState, Router) {
    let state = AdminState {
        sessions: SessionStore::new(config.session_ttl),
        config: Arc::new(config),
        metrics,
        transport,
        rate_limiter: RateLimiter::new(),
        logs,
        workflows: WorkflowWatcher::new(),
        db_config: Arc::new(db_config),
        db_slot,
        db_cache: Arc::new(std::sync::RwLock::new(None)),
        bootstrap_timeout_secs,
        health_check_interval_secs,
    };
    let router = create_admin_router(state.clone());
    (state, router)
}
