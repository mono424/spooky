use super::MaybeSendSync;
use crate::api::Method;

/// Fire-side of a cancellation pair. Cloning is cheap; firing is idempotent.
/// Portable replacement for `tokio_util::sync::CancellationToken` (which is
/// not wasm-safe) — built on `tokio::sync::watch`, which is.
#[derive(Clone)]
pub struct CancelHandle(tokio::sync::watch::Sender<bool>);

/// Wait-side of a cancellation pair.
#[derive(Clone)]
pub struct CancelWatch(tokio::sync::watch::Receiver<bool>);

impl CancelHandle {
    pub fn new() -> (CancelHandle, CancelWatch) {
        let (tx, rx) = tokio::sync::watch::channel(false);
        (CancelHandle(tx), CancelWatch(rx))
    }

    /// Fire the cancellation. Watches resolve promptly; repeat calls no-op.
    pub fn cancel(&self) {
        let _ = self.0.send(true);
    }

    pub fn is_cancelled(&self) -> bool {
        *self.0.borrow()
    }
}

impl CancelWatch {
    /// Resolves once the paired [`CancelHandle`] fires (immediately if it
    /// already has, or if the handle was dropped).
    pub async fn cancelled(&mut self) {
        if *self.0.borrow() {
            return;
        }
        // Err = sender dropped without firing; treat as cancelled so waiters
        // never hang on an abandoned pair.
        while self.0.changed().await.is_ok() {
            if *self.0.borrow() {
                return;
            }
        }
    }

    pub fn is_cancelled(&self) -> bool {
        *self.0.borrow()
    }
}

pub struct OutboundRequest {
    pub method: Method,
    pub url: String,
    pub bearer: Option<String>,
    /// Extra request headers (name, value). Used by adapters that need more
    /// than bearer auth — e.g. the SurrealDB HTTP-RPC `Db` adapter sets
    /// `Authorization`/`surreal-ns`/`surreal-db`. Empty for job dispatch.
    pub headers: Vec<(String, String)>,
    pub json_body: Option<serde_json::Value>,
    pub timeout: std::time::Duration,
}

pub struct OutboundResponse {
    pub status: u16,
    pub body: String,
}

#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    #[error("timeout after {0:?}")]
    Timeout(std::time::Duration),
    #[error("cancelled")]
    Cancelled,
    #[error("transport: {0}")]
    Transport(String),
}

#[cfg(test)]
mod tests {
    use super::CancelHandle;

    #[tokio::test]
    async fn cancel_resolves_watch_promptly_and_idempotently() {
        let (handle, mut watch) = CancelHandle::new();
        assert!(!watch.is_cancelled());

        handle.cancel();
        handle.cancel(); // idempotent
        watch.cancelled().await; // resolves immediately
        assert!(watch.is_cancelled());

        // A watch obtained after the fire also resolves immediately.
        let mut late = watch.clone();
        late.cancelled().await;
    }

    #[tokio::test]
    async fn dropped_handle_unblocks_waiters() {
        let (handle, mut watch) = CancelHandle::new();
        drop(handle);
        // Must not hang: abandoned pair counts as cancelled.
        watch.cancelled().await;
    }
}

/// Outbound HTTP port: job dispatch to user backends, backend health pings.
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
pub trait HttpClient: MaybeSendSync {
    /// Send the request. When `cancel` is provided and fires first, resolve
    /// promptly with `Err(HttpError::Cancelled)` — cancel WINS the race (the
    /// `biased` select semantics of the job runner's kill path).
    async fn send(
        &self,
        req: OutboundRequest,
        cancel: Option<CancelWatch>,
    ) -> Result<OutboundResponse, HttpError>;
}
