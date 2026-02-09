use anyhow::{Context, Result};
use async_nats::Client;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use tracing::{debug, error, info, warn};

/// Record operation type (copy from scheduler/messages.rs for now)
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordOp {
    Create,
    Update,
    Delete,
}

/// Record update from Scheduler
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RecordUpdate {
    pub table: String,
    pub operation: RecordOp, 
    pub record_id: String,
    pub data: Option<Value>,
    pub version: u64,
}

/// SSP heartbeat
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SspHeartbeat {
    pub ssp_id: String,
    pub timestamp: u64,
    pub active_queries: usize,
    pub cpu_usage: Option<f64>,
    pub memory_usage: Option<f64>,
}

/// Bootstrap request
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BootstrapRequest {
    pub ssp_id: String,
    pub tables: Vec<String>,
}

/// Bootstrap chunk  
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BootstrapChunk {
    pub chunk_index: usize,
    pub total_chunks: usize,
    pub table: String,
    pub records: Vec<(String, Value)>,
}

/// Bootstrap response
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BootstrapResponse {
    pub ssp_id: String,
    pub chunks: Vec<BootstrapChunk>,
}

/// NATS transport for SSP sidecar
pub struct SspNatsClient {
    client: Client,
    ssp_id: String,
}

impl SspNatsClient {
    /// Connect to NATS
    pub async fn connect(url: &str, ssp_id: String) -> Result<Self> {
        info!("Connecting to NATS at {}...", url);
        let client = async_nats::connect(url)
            .await
            .context("Failed to connect to NATS")?;
        info!("Connected to NATS successfully");
        
        Ok(Self { client, ssp_id })
    }

    /// Request bootstrap from Scheduler
    pub async fn request_bootstrap(&self) -> Result<BootstrapResponse> {
        info!("Requesting bootstrap from Scheduler...");
        
        let request = BootstrapRequest {
            ssp_id: self.ssp_id.clone(),
            tables: vec!["thread".to_string(), "job".to_string(), "user".to_string()],
        };
        
        let request_payload = serde_json::to_vec(&request)?;
        
        let response = self
            .client
            .request("spooky.bootstrap.request", request_payload.into())
            .await
            .context("Failed to send bootstrap request")?;
        
        let bootstrap: BootstrapResponse = serde_json::from_slice(&response.payload)
            .context("Failed to parse bootstrap response")?;
        
        info!(
            "Received bootstrap with {} chunks",
            bootstrap.chunks.len()
        );
        
        Ok(bootstrap)
    }

    /// Subscribe to record updates
    pub async fn subscribe_to_updates(&self) -> Result<impl futures::Stream<Item = RecordUpdate>> {
        info!("Subscribing to record updates...");
        
        let mut subscriber = self
            .client
            .subscribe("spooky.ingest.*")
            .await
            .context("Failed to subscribe to updates")?;
        
        Ok(async_stream::stream! {
            while let Some(msg) = subscriber.next().await {
                match serde_json::from_slice::<RecordUpdate>(&msg.payload) {
                    Ok(update) => {
                        debug!("Received update: {} {} on {}", 
                            update.operation, update.record_id, update.table);
                        yield update;
                    }
                    Err(e) => {
                        warn!("Failed to parse update message: {}", e);
                    }
                }
            }
        })
    }

    /// Publish heartbeat
    pub async fn publish_heartbeat(&self, heartbeat: SspHeartbeat) -> Result<()> {
        let payload = serde_json::to_vec(&heartbeat)?;
        
        self.client
            .publish("spooky.ssp.heartbeat", payload.into())
            .await
            .context("Failed to publish heartbeat")?;
        
        debug!("Published heartbeat");
        Ok(())
    }

    /// Notify Scheduler that bootstrap is complete and SSP is ready
    pub async fn mark_ready(&self) -> Result<()> {
        info!("Marking SSP '{}' as ready", self.ssp_id);
        
        // Send a special heartbeat to signal readiness
        // The Scheduler will call mark_ready() on its SspPool
        let heartbeat = SspHeartbeat {
            ssp_id: self.ssp_id.clone(),
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_secs(),
            active_queries: 0,
            cpu_usage: None,
            memory_usage: None,
        };
        
        self.publish_heartbeat(heartbeat).await?;
        Ok(())
    }
}
