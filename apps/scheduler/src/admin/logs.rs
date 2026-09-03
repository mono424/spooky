//! `GET /admin/api/logs` — recent log history, then a live tail, over SSE.
//!
//! `source=scheduler` reads this process's own ring. `source=ssp:<id>` proxies
//! the same endpoint on that SSP, attaching `SPKY_AUTH_SECRET` exactly as every
//! other scheduler-to-SSP call does; the SSP serves it from the identical ring
//! type inside its authenticated route group.
//!
//! Backends have no source here, and that is a deliberate limit rather than an
//! omission: they are arbitrary user services reached over HTTP health checks,
//! with no log pipe the scheduler could read.

use std::convert::Infallible;
use std::time::Duration;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use futures::stream::{self, Stream, StreamExt};
use serde::Deserialize;
use tokio::sync::broadcast::error::RecvError;
use tracing::warn;

use maintenance::log_ring::LogLine;

use super::{api_error, AdminState, ApiError};

/// How many historical lines a new stream is seeded with. Enough to explain a
/// crash that just happened; not so many that opening the tab ships a megabyte.
const DEFAULT_BACKFILL: usize = 500;

#[derive(Debug, Deserialize)]
pub struct LogQuery {
    /// `scheduler` (default) or `ssp:<id>`.
    #[serde(default)]
    pub source: Option<String>,
    /// Keep the stream open after the backfill. Defaults to true; `tail=false`
    /// gives a one-shot history dump that closes on its own.
    #[serde(default)]
    pub tail: Option<bool>,
    #[serde(default)]
    pub backfill: Option<usize>,
}

fn line_event(line: &LogLine) -> Event {
    // `json_data` can only fail if the value is not serialisable, which
    // `LogLine` always is; falling back to a comment keeps the stream alive
    // rather than tearing it down over one line.
    Event::default()
        .event("line")
        .json_data(line)
        .unwrap_or_else(|_| Event::default().comment("unserialisable line"))
}

fn dropped_event(n: u64) -> Event {
    Event::default().event("dropped").data(n.to_string())
}

pub async fn stream(
    State(state): State<AdminState>,
    Query(q): Query<LogQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let source = q.source.unwrap_or_else(|| "scheduler".to_string());
    let tail = q.tail.unwrap_or(true);
    let backfill = q.backfill.unwrap_or(DEFAULT_BACKFILL).min(10_000);

    if let Some(ssp_id) = source.strip_prefix("ssp:") {
        return proxy_ssp_logs(&state, ssp_id, tail, backfill)
            .await
            .map(IntoResponse::into_response);
    }
    if source != "scheduler" {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            format!("Unknown log source '{}'. Use 'scheduler' or 'ssp:<id>'.", source),
        ));
    }

    let history = state.logs.snapshot(backfill);
    let rx = state.logs.subscribe();

    let backlog = stream::iter(history.into_iter().map(|l| Ok::<_, Infallible>(line_event(&l))));

    if !tail {
        return Ok(sse(backlog.boxed()).into_response());
    }

    // `unfold` over the broadcast receiver rather than a wrapper crate: this is
    // the whole of what we need from one, and it keeps the Lagged handling
    // visible right here.
    let live = stream::unfold(rx, |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(line) => return Some((Ok::<_, Infallible>(line_event(&line)), rx)),
                // The client fell behind. Tell it how much it missed instead of
                // silently closing the gap — a log viewer that hides its own
                // holes is worse than one that admits them.
                Err(RecvError::Lagged(n)) => return Some((Ok(dropped_event(n)), rx)),
                Err(RecvError::Closed) => return None,
            }
        }
    });

    Ok(sse(backlog.chain(live).boxed()).into_response())
}

fn sse<S>(s: S) -> Sse<S>
where
    S: Stream<Item = Result<Event, Infallible>> + Send + 'static,
{
    // Comments every 15s so an idle stream survives intermediary idle
    // timeouts, and so the client can tell "connected and quiet" from "dead".
    Sse::new(s).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

/// Relay an SSP's log stream.
///
/// A byte-for-byte passthrough of the SSP's own SSE body: it already speaks
/// this exact format, so re-parsing and re-encoding it here would only add a
/// place for the two to drift.
async fn proxy_ssp_logs(
    state: &AdminState,
    ssp_id: &str,
    tail: bool,
    backfill: usize,
) -> Result<axum::response::Response, ApiError> {
    let url = {
        let pool = state.metrics.ssp_pool.read().await;
        match pool.get(ssp_id) {
            Some(ssp) if !ssp.url.is_empty() => ssp.url.clone(),
            Some(_) => {
                return Err(api_error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    format!("SSP '{}' has no advertised URL yet", ssp_id),
                ))
            }
            None => {
                return Err(api_error(
                    StatusCode::NOT_FOUND,
                    format!("No SSP named '{}'", ssp_id),
                ))
            }
        }
    };

    let target = format!(
        "{}/logs?tail={}&backfill={}",
        url.trim_end_matches('/'),
        tail,
        backfill
    );

    let resp = state
        .transport
        .get_stream(&target)
        .await
        .map_err(|e| {
            warn!(ssp = ssp_id, error = %e, "Failed to open SSP log stream");
            api_error(
                StatusCode::BAD_GATEWAY,
                format!("Could not reach SSP '{}': {}", ssp_id, e),
            )
        })?;

    if !resp.status().is_success() {
        let status = StatusCode::from_u16(resp.status().as_u16())
            .unwrap_or(StatusCode::BAD_GATEWAY);
        return Err(api_error(
            status,
            format!("SSP '{}' refused the log stream ({})", ssp_id, resp.status()),
        ));
    }

    let body = axum::body::Body::from_stream(resp.bytes_stream());
    Ok(axum::response::Response::builder()
        .header(axum::http::header::CONTENT_TYPE, "text/event-stream")
        .header(axum::http::header::CACHE_CONTROL, "no-cache")
        .body(body)
        .expect("static header values are valid"))
}
