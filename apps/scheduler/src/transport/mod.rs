pub mod nats;

use anyhow::Result;
use async_trait::async_trait;
use futures::Stream;
use serde::{Deserialize, Serialize};
use std::time::Instant;

pub use nats::NatsTransport;

/// Message received from transport layer
#[derive(Debug, Clone)]
pub struct Message {
    pub subject: String,
    pub payload: Vec<u8>,
    pub reply_to: Option<String>,
}

/// Information about a connected SSP
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SspInfo {
    pub id: String,
    #[serde(skip, default = "std::time::Instant::now")]
    pub connected_at: Instant,
    #[serde(skip, default = "std::time::Instant::now")]
    pub last_heartbeat: Instant,
    pub query_count: usize,
    pub active_jobs: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_usage: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_usage: Option<f64>,
}

/// Transport abstraction for communication with SSPs
#[async_trait]
pub trait Transport: Send + Sync + 'static {
    /// Broadcast a message to all connected SSPs
    async fn broadcast(&self, subject: &str, payload: &[u8]) -> Result<()>;

    /// Send a message to one SSP (round-robin / least-loaded)
    async fn send_to(&self, ssp_id: &str, subject: &str, payload: &[u8]) -> Result<()>;

    /// Send to one SSP from a queue group (load-balanced)
    async fn queue_send(&self, subject: &str, payload: &[u8]) -> Result<()>;

    /// Request/reply to a specific SSP
    async fn request(&self, ssp_id: &str, subject: &str, payload: &[u8]) -> Result<Vec<u8>>;

    /// Subscribe to messages from SSPs
    async fn subscribe(&self, subject: &str) -> Result<Box<dyn Stream<Item = Message> + Send + Unpin>>;

    /// Track connected SSPs
    async fn connected_ssps(&self) -> Result<Vec<SspInfo>>;
}
