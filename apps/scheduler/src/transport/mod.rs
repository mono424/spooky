use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Instant;
use tracing::{debug, warn};

/// Information about a connected SSP
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SspInfo {
    pub id: String,
    pub url: String,
    pub version: String,
    #[serde(skip, default = "std::time::Instant::now")]
    pub connected_at: Instant,
    #[serde(skip, default = "std::time::Instant::now")]
    pub last_heartbeat: Instant,
    pub query_count: usize,
    pub views: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_usage: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_usage: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<std::collections::HashMap<String, String>>,
    /// Last bootstrap progress this SSP reported, if any. Advisory only —
    /// nothing in the bootstrap handshake reads it. Cleared when the SSP goes
    /// Ready so a later `/info` never shows a stale bar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bootstrap: Option<ssp_protocol::BootstrapProgress>,
}

/// HTTP-based transport for communicating with SSP sidecars
#[derive(Clone)]
pub struct HttpTransport {
    client: Client,
    /// Separate client for responses that are *meant* to stay open (SSE log
    /// tails). `client`'s 30s request timeout is correct for every RPC-shaped
    /// call and fatal for a stream, and reqwest applies it to the whole
    /// response, not just the headers.
    stream_client: Client,
    ssp_auth_secret: Option<String>,
}

impl HttpTransport {
    /// Create a new HTTP transport
    pub fn new() -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");

        let stream_client = Client::builder()
            // No overall timeout; a connect timeout still bounds the one part
            // of a stream that should never take long.
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("Failed to create streaming HTTP client");

        let ssp_auth_secret = std::env::var("SPKY_AUTH_SECRET").ok();

        Self { client, stream_client, ssp_auth_secret }
    }

    /// GET a long-lived response (an SSE stream) from an absolute URL, with the
    /// SSP bearer attached. The response is returned unread so the caller can
    /// relay its body as it arrives.
    pub async fn get_stream(&self, url: &str) -> Result<reqwest::Response> {
        let mut request = self.stream_client.get(url);
        if let Some(ref secret) = self.ssp_auth_secret {
            request = request.bearer_auth(secret);
        }
        request
            .send()
            .await
            .with_context(|| format!("Failed to open stream at {}", url))
    }

    /// POST a JSON payload to a specific SSP endpoint
    pub async fn post_to_ssp<T: Serialize>(
        &self,
        ssp_url: &str,
        path: &str,
        payload: &T,
    ) -> Result<reqwest::Response> {
        let url = format!("{}{}", ssp_url.trim_end_matches('/'), path);
        debug!("POST {} -> {}", path, url);

        let mut request = self.client.post(&url).json(payload);
        if let Some(ref secret) = self.ssp_auth_secret {
            request = request.bearer_auth(secret);
        }

        let response = request
            .send()
            .await
            .with_context(|| format!("Failed to POST to SSP at {}", url))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("SSP returned {} for {}: {}", status, url, body);
        }

        Ok(response)
    }

    /// Broadcast a JSON payload to all ready SSPs
    pub async fn broadcast_to_ssps<T: Serialize + std::fmt::Debug>(
        &self,
        ssps: &[SspInfo],
        path: &str,
        payload: &T,
    ) -> Vec<(String, Result<()>)> {
        let mut results = Vec::new();

        for ssp in ssps {
            let result = self.post_to_ssp(&ssp.url, path, payload).await.map(|_| ());
            if let Err(ref e) = result {
                warn!("Failed to broadcast to SSP '{}': {}", ssp.id, e);
            }
            results.push((ssp.id.clone(), result));
        }

        results
    }

    /// GET from a specific SSP endpoint. Sends the bearer like the POST path
    /// does — harmless on public routes, required for the authed ones
    /// (`/debug/heartbeat`, `/debug/catchup-rows/*` previously only worked
    /// when no auth secret was configured).
    pub async fn get_from_ssp(&self, ssp_url: &str, path: &str) -> Result<reqwest::Response> {
        let url = format!("{}{}", ssp_url.trim_end_matches('/'), path);
        debug!("GET {}", url);

        let mut request = self.client.get(&url);
        if let Some(ref secret) = self.ssp_auth_secret {
            request = request.bearer_auth(secret);
        }

        let response = request
            .send()
            .await
            .with_context(|| format!("Failed to GET from SSP at {}", url))?;

        Ok(response)
    }

    /// Check if an SSP is healthy via GET /health
    pub async fn check_ssp_health(&self, ssp_url: &str) -> bool {
        match self.get_from_ssp(ssp_url, "/health").await {
            Ok(resp) => resp.status().is_success(),
            Err(_) => false,
        }
    }

    /// Check SSP health and return its status string (e.g. "bootstrapping", "ready", "failed")
    pub async fn check_ssp_health_status(&self, ssp_url: &str) -> Option<String> {
        match self.get_from_ssp(ssp_url, "/health").await {
            Ok(resp) => {
                if let Ok(body) = resp.json::<serde_json::Value>().await {
                    body.get("status")
                        .and_then(|s| s.as_str())
                        .map(|s| s.to_string())
                } else {
                    None
                }
            }
            Err(_) => None,
        }
    }
}
