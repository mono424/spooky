use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
use tokio::sync::RwLock;
use tracing::{info, warn};

use crate::config::BackendHealthConfig;

/// Shared backend configs that can be updated at runtime (e.g. via PUT /backends).
pub type SharedBackendConfigs = Arc<RwLock<Vec<BackendHealthConfig>>>;

pub fn create_shared_configs(backends: &[BackendHealthConfig]) -> SharedBackendConfigs {
    Arc::new(RwLock::new(backends.to_vec()))
}

/// Replace the backend configs and reconcile the health cache.
/// Existing backends keep their health status; new ones start as Unknown.
pub async fn update_backends(
    configs: &SharedBackendConfigs,
    cache: &BackendHealthCache,
    new_backends: Vec<BackendHealthConfig>,
) {
    // Update shared configs first (same lock order as the health monitor)
    *configs.write().await = new_backends.clone();

    // Reconcile cache: keep health status for unchanged backends
    let mut entries = cache.write().await;
    let old_entries: Vec<BackendHealthEntry> = entries.drain(..).collect();
    for cfg in &new_backends {
        if let Some(existing) = old_entries.iter().find(|e| e.name == cfg.name && e.url == cfg.url) {
            let mut entry = existing.clone();
            entry.env = cfg.env.clone();
            entry.healthcheck = cfg.healthcheck.clone();
            entry.port = cfg.port;
            entries.push(entry);
        } else {
            entries.push(BackendHealthEntry::from_config(cfg));
        }
    }
}

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

/// Spawn a background task that periodically health-checks all backends.
/// Reads from `SharedBackendConfigs` on each tick so live updates via
/// `update_backends()` are picked up without restarting.
pub fn start_backend_health_monitor(
    configs: SharedBackendConfigs,
    cache: BackendHealthCache,
    interval_secs: u64,
) {
    let http_client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap_or_default();

    info!(interval_secs, "Starting backend health monitor");

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));
        interval.tick().await;

        loop {
            let backends = configs.read().await.clone();

            for backend in &backends {
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
                if let Some(entry) = entries.iter_mut().find(|e| e.name == backend.name) {
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
