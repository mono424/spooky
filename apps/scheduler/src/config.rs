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
    pub ingest_host: Option<String>,
    pub ingest_port: u16,
    pub snapshot_update_interval_secs: u64,
    pub max_buffer_per_ssp: usize,
    pub bootstrap_timeout_secs: u64,
    pub ssp_poll_interval_ms: u64,
    pub wal_path: PathBuf,
    pub health_check_interval_secs: u64,
    #[serde(skip)]
    pub scheduler_id: String,
    #[serde(skip)]
    pub backends: Vec<BackendHealthConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BackendHealthConfig {
    pub name: String,
    pub url: String,
    pub healthcheck: String,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub env: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DbConfig {
    pub url: String,
    pub namespace: String,
    pub database: String,
    pub username: String,
    pub password: String,
}

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
                url: "ws://localhost:8000".to_string(),
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
            ingest_host: None,
            ingest_port: 9667,
            snapshot_update_interval_secs: 300,
            max_buffer_per_ssp: 10_000,
            bootstrap_timeout_secs: 120,
            ssp_poll_interval_ms: 3000,
            wal_path: PathBuf::from("./data/event_wal.log"),
            health_check_interval_secs: 15,
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

        // Override DB settings from SPKY_* environment variables
        if let Ok(v) = std::env::var("SPKY_DB_WS") {
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

        // Parse backend health check targets from JSON env var
        if let Ok(backends_json) = std::env::var("SPKY_SCHEDULER_BACKENDS") {
            if let Ok(backends) = serde_json::from_str::<Vec<BackendHealthConfig>>(&backends_json) {
                scheduler_config.backends = backends;
            }
        }

        Ok(scheduler_config)
    }
}
