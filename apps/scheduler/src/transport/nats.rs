use super::{Message, SspInfo, Transport};
use crate::config::NatsConfig;
use anyhow::{Context, Result};
use async_nats::Client;
use async_trait::async_trait;
use futures::{Stream, StreamExt};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// NATS implementation of the Transport trait
pub struct NatsTransport {
    client: Client,
    ssps: Arc<RwLock<HashMap<String, SspInfo>>>,
}

impl NatsTransport {
    /// Create a new NATS transport
    pub async fn new(config: &NatsConfig) -> Result<Self> {
        let client = if let Some(creds_path) = &config.credentials {
            async_nats::ConnectOptions::with_credentials_file(creds_path)
                .await
                .context("Failed to load NATS credentials")?
                .connect(&config.url)
                .await
                .context("Failed to connect to NATS")?
        } else {
            async_nats::connect(&config.url)
                .await
                .context("Failed to connect to NATS")?
        };

        Ok(Self {
            client,
            ssps: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    /// Register or update an SSP in the pool
    pub async fn register_ssp(&self, ssp: SspInfo) {
        let mut ssps = self.ssps.write().await;
        ssps.insert(ssp.id.clone(), ssp);
    }

    /// Remove an SSP from the pool
    pub async fn remove_ssp(&self, ssp_id: &str) {
        let mut ssps = self.ssps.write().await;
        ssps.remove(ssp_id);
    }
}

#[async_trait]
impl Transport for NatsTransport {
    async fn broadcast(&self, subject: &str, payload: &[u8]) -> Result<()> {
        self.client
            .publish(subject.to_string(), payload.to_vec().into())
            .await
            .context("Failed to broadcast message")?;
        Ok(())
    }

    async fn send_to(&self, ssp_id: &str, subject: &str, payload: &[u8]) -> Result<()> {
        let targeted_subject = format!("spooky.ssp.{}.{}", ssp_id, subject);
        self.client
            .publish(targeted_subject, payload.to_vec().into())
            .await
            .context("Failed to send targeted message")?;
        Ok(())
    }

    async fn queue_send(&self, subject: &str, payload: &[u8]) -> Result<()> {
        // For queue groups, just publish to the subject
        // SSPs will subscribe with a queue group name to load balance
        self.client
            .publish(subject.to_string(), payload.to_vec().into())
            .await
            .context("Failed to send queue message")?;
        Ok(())
    }

    async fn request(&self, ssp_id: &str, subject: &str, payload: &[u8]) -> Result<Vec<u8>> {
        let targeted_subject = format!("spooky.ssp.{}.{}", ssp_id, subject);
        let response = self.client
            .request(targeted_subject, payload.to_vec().into())
            .await
            .context("Request failed")?;
        Ok(response.payload.to_vec())
    }

    async fn subscribe(&self, subject: &str) -> Result<Box<dyn Stream<Item = Message> + Send + Unpin>> {
        let subscriber = self.client
            .subscribe(subject.to_string())
            .await
            .context("Failed to subscribe")?;

        // Convert NATS subscriber to our Message stream
        let stream = futures::stream::unfold(subscriber, |mut sub| async move {
            sub.next().await.map(|nats_msg| {
                let msg = Message {
                    subject: nats_msg.subject.to_string(),
                    payload: nats_msg.payload.to_vec(),
                    reply_to: nats_msg.reply.map(|s| s.to_string()),
                };
                (msg, sub)
            })
        });

        Ok(Box::new(Box::pin(stream)))
    }

    async fn connected_ssps(&self) -> Result<Vec<SspInfo>> {
        let ssps = self.ssps.read().await;
        Ok(ssps.values().cloned().collect())
    }
}
