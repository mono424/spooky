use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct SchedulerConfig {
    pub db: DbConfig,
    pub load_balance: LoadBalanceStrategy,
    pub heartbeat_interval_ms: u64,
    pub heartbeat_timeout_ms: u64,
    pub bootstrap_chunk_size: usize,
    pub job_tables: Vec<String>,
    pub replica_db_path: PathBuf,
    /// Hard ceiling on the initial replica clone. A clone that overruns it
    /// fails `start()` (and so exits the process) instead of leaving the
    /// scheduler wedged in `cloning`, answering 503 to every SSP registration
    /// for the life of the container. Override with SPKY_CLONE_TIMEOUT_SECS.
    pub clone_timeout_secs: u64,
    pub ingest_host: Option<String>,
    pub ingest_port: u16,
    pub snapshot_update_interval_secs: u64,
    pub max_buffer_per_ssp: usize,
    pub bootstrap_timeout_secs: u64,
    pub ssp_poll_interval_ms: u64,
    pub wal_path: PathBuf,
    pub health_check_interval_secs: u64,
    pub feature_flag_sweep_interval_secs: u64,
    #[serde(skip)]
    pub scheduler_id: String,
    #[serde(skip)]
    pub backends: Vec<BackendHealthConfig>,
}

// Both moved to the shared `maintenance` crate; re-exported so existing
// `config::DbConfig` / `config::BackendHealthConfig` paths keep working.
pub use maintenance::backend_health::BackendHealthConfig;
pub use maintenance::db::DbConfig;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LoadBalanceStrategy {
    RoundRobin,
    LeastQueries,
    LeastLoad,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            db: DbConfig {
                url: "http://localhost:8000".to_string(),
                namespace: "sp00ky".to_string(),
                database: "sp00ky".to_string(),
                username: "root".to_string(),
                password: "root".to_string(),
            },
            load_balance: LoadBalanceStrategy::LeastQueries,
            heartbeat_interval_ms: 5000,
            heartbeat_timeout_ms: 15000,
            bootstrap_chunk_size: 1000,
            job_tables: vec![],
            replica_db_path: PathBuf::from("./data/replica"),
            // 15 minutes: whitepawn's 288k-record clone takes ~100s on a
            // healthy box, so this is an order of magnitude of headroom for a
            // large tenant on slow disk, and still bounded.
            clone_timeout_secs: 900,
            ingest_host: None,
            ingest_port: 9667,
            snapshot_update_interval_secs: 300,
            max_buffer_per_ssp: 10_000,
            // 120 livelocked a real deployment once its tables outgrew what a
            // paged /proxy load can move in two minutes (2026-08-08): timeout →
            // SSP exit → re-register → re-freeze, forever. Override with
            // SPKY_BOOTSTRAP_TIMEOUT_SECS.
            bootstrap_timeout_secs: 300,
            ssp_poll_interval_ms: 3000,
            wal_path: PathBuf::from("./data/event_wal.log"),
            health_check_interval_secs: 15,
            feature_flag_sweep_interval_secs: 30,
            scheduler_id: String::new(),
            backends: vec![],
        }
    }
}

impl SchedulerConfig {
    /// Load configuration from file and environment variables
    pub fn load() -> Result<Self> {
        let mut builder = config::Config::builder()
            .add_source(config::Config::try_from(&SchedulerConfig::default())?);

        // Try to load from sp00ky.yml (optional)
        builder = builder.add_source(config::File::with_name("sp00ky").required(false));

        let config = builder.build()?;
        let mut scheduler_config: SchedulerConfig = config.try_deserialize()?;

        // Override DB settings from SPKY_* environment variables.
        // SPKY_DB_URL is canonical; SPKY_DB_WS kept as a legacy fallback
        // (any scheme is accepted — ws:// URLs are normalized to HTTP).
        if let Ok(v) = std::env::var("SPKY_DB_URL").or_else(|_| std::env::var("SPKY_DB_WS")) {
            scheduler_config.db.url = v;
        }
        if let Ok(v) = std::env::var("SPKY_DB_NS") {
            scheduler_config.db.namespace = v;
        }
        if let Ok(v) = std::env::var("SPKY_DB_NAME") {
            scheduler_config.db.database = v;
        }
        if let Ok(v) = std::env::var("SPKY_DB_USER") {
            scheduler_config.db.username = v;
        }
        if let Ok(v) = std::env::var("SPKY_DB_PASS") {
            scheduler_config.db.password = v;
        }

        scheduler_config.scheduler_id = std::env::var("SPKY_SCHEDULER_ID")
            .unwrap_or_else(|_| format!("scheduler-{}", uuid::Uuid::new_v4()));

        // Drain WAL → replica every N seconds. Default 300 is fine for
        // production but makes dev painful: records vanish from live
        // queries until the first 5-minute tick. The dev launcher sets
        // this to 2s.
        if let Ok(v) = std::env::var("SPKY_SNAPSHOT_UPDATE_INTERVAL_SECS") {
            if let Ok(n) = v.parse::<u64>() {
                if n > 0 {
                    scheduler_config.snapshot_update_interval_secs = n;
                }
            }
        }

        // Feature-flag materialization sweep cadence. Default 30s.
        if let Ok(v) = std::env::var("SPKY_FEATURE_FLAG_SWEEP_SECS") {
            if let Ok(n) = v.parse::<u64>() {
                if n > 0 {
                    scheduler_config.feature_flag_sweep_interval_secs = n;
                }
            }
        }

        // SSP bootstrap budget. Default 120s fits small datasets; a replica
        // with large tables (registration drain + rehash + paged /proxy load)
        // can legitimately need more — the 2026-08-08 whitepawn outage
        // livelocked on exactly this: every bootstrap timed out, the SSP
        // exited, re-registered, and re-froze the snapshot forever.
        if let Ok(v) = std::env::var("SPKY_CLONE_TIMEOUT_SECS") {
            if let Ok(secs) = v.parse::<u64>() {
                if secs > 0 {
                    scheduler_config.clone_timeout_secs = secs;
                }
            }
        }

        if let Ok(v) = std::env::var("SPKY_BOOTSTRAP_TIMEOUT_SECS") {
            if let Ok(n) = v.parse::<u64>() {
                if n > 0 {
                    scheduler_config.bootstrap_timeout_secs = n;
                }
            }
        }

        // Parse backend health check targets from JSON env var
        // (SPKY_BACKENDS preferred, SPKY_SCHEDULER_BACKENDS legacy fallback).
        scheduler_config.backends = maintenance::backend_health::backends_from_env();

        Ok(scheduler_config)
    }
}
