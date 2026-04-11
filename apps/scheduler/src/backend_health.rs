use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
use tokio::sync::RwLock;
use tracing::{info, warn};

use crate::config::BackendHealthConfig;

/// Backend health status
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendStatus {
    Healthy,
    Unhealthy,
    Unreachable,
    Unknown,
}

impl BackendStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            BackendStatus::Healthy => "healthy",
            BackendStatus::Unhealthy => "unhealthy",
            BackendStatus::Unreachable => "unreachable",
            BackendStatus::Unknown => "unknown",
        }
    }
}

/// Cached health state for a single backend
#[derive(Debug, Clone)]
pub struct BackendHealthEntry {
    pub name: String,
    pub url: String,
    pub healthcheck: String,
    pub port: Option<u16>,
    pub env: Option<Vec<String>>,
    pub status: BackendStatus,
    pub last_checked: Option<SystemTime>,
    pub last_healthy: Option<SystemTime>,
    pub response_time_ms: Option<u64>,
}

impl BackendHealthEntry {
    pub fn from_config(cfg: &BackendHealthConfig) -> Self {
        Self {
            name: cfg.name.clone(),
            url: cfg.url.clone(),
            healthcheck: cfg.healthcheck.clone(),
            port: cfg.port,
            env: cfg.env.clone(),
            status: BackendStatus::Unknown,
            last_checked: None,
            last_healthy: None,
            response_time_ms: None,
        }
    }

    /// Extract IP from URL (e.g. "http://10.100.1.40:3000" -> "10.100.1.40")
    pub fn ip(&self) -> Option<String> {
        self.url
            .trim_start_matches("http://")
            .trim_start_matches("https://")
            .split(':')
            .next()
            .map(|s| s.to_string())
    }
}

pub type BackendHealthCache = Arc<RwLock<Vec<BackendHealthEntry>>>;

/// Create a cache pre-populated from config (all Unknown status)
pub fn create_health_cache(backends: &[BackendHealthConfig]) -> BackendHealthCache {
    let entries = backends.iter().map(BackendHealthEntry::from_config).collect();
    Arc::new(RwLock::new(entries))
}

/// Spawn a background task that periodically health-checks all backends
pub fn start_backend_health_monitor(
    backends: Vec<BackendHealthConfig>,
    cache: BackendHealthCache,
    interval_secs: u64,
) {
    if backends.is_empty() {
        info!("No backends configured for health checking");
        return;
    }

    let http_client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap_or_default();

    info!(
        count = backends.len(),
        interval_secs, "Starting backend health monitor"
    );

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));
        // Run the first check immediately
        interval.tick().await;

        loop {
            for (i, backend) in backends.iter().enumerate() {
                let health_url = format!(
                    "{}{}",
                    backend.url.trim_end_matches('/'),
                    backend.healthcheck
                );

                let start = Instant::now();
                let (status, response_time_ms) = match http_client.get(&health_url).send().await {
                    Ok(resp) if resp.status().is_success() => {
                        (BackendStatus::Healthy, start.elapsed().as_millis() as u64)
                    }
                    Ok(resp) => {
                        warn!(
                            backend = %backend.name,
                            status_code = resp.status().as_u16(),
                            "Backend health check returned non-success"
                        );
                        (BackendStatus::Unhealthy, start.elapsed().as_millis() as u64)
                    }
                    Err(e) => {
                        warn!(
                            backend = %backend.name,
                            error = %e,
                            "Backend health check failed"
                        );
                        (BackendStatus::Unreachable, start.elapsed().as_millis() as u64)
                    }
                };

                let now = SystemTime::now();
                let mut entries = cache.write().await;
                if let Some(entry) = entries.get_mut(i) {
                    entry.status = status;
                    entry.last_checked = Some(now);
                    entry.response_time_ms = Some(response_time_ms);
                    if status == BackendStatus::Healthy {
                        entry.last_healthy = Some(now);
                    }
                }
            }

            interval.tick().await;
        }
    });
}
