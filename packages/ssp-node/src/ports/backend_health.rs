use super::MaybeSendSync;
use serde::{Deserialize, Serialize};

/// Aggregated backend health, as consumed by `GET /health`.
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct BackendCounts {
    pub healthy: usize,
    pub unhealthy: usize,
    pub unreachable: usize,
    pub total: usize,
}

/// A backend to health-check (the wire shape of `PUT /backends` — matches
/// `maintenance::BackendHealthConfig` field-for-field).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendSpec {
    pub name: String,
    pub url: String,
    pub healthcheck: String,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub env: Option<Vec<String>>,
}

/// Backend health monitoring port — standalone mode only (`None` on the node
/// in cluster mode, where the scheduler owns backend health).
///
/// VM adapter wraps the `maintenance` crate's cache + live-config pair; a CF
/// shell brings its own fetch-based checker later.
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
pub trait BackendHealth: MaybeSendSync {
    async fn counts(&self) -> BackendCounts;
    /// Replace the monitored backend list at runtime (`PUT /backends`).
    async fn update(&self, backends: Vec<BackendSpec>);
}
