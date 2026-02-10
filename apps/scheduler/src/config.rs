use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SchedulerConfig {
    pub db: DbConfig,
    pub load_balance: LoadBalanceStrategy,
    pub heartbeat_interval_ms: u64,
    pub heartbeat_timeout_ms: u64,
    pub bootstrap_chunk_size: usize,
    pub job_tables: Vec<String>,
    pub replica_db_path: PathBuf,
    pub replica_keep_versions: u64,
    pub ingest_host: Option<String>,
    pub ingest_port: u16,
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
                namespace: "spooky".to_string(),
                database: "spooky".to_string(),
                username: "root".to_string(),
                password: "root".to_string(),
            },
            load_balance: LoadBalanceStrategy::LeastQueries,
            heartbeat_interval_ms: 5000,
            heartbeat_timeout_ms: 15000,
            bootstrap_chunk_size: 1000,
            job_tables: vec![],
            replica_db_path: PathBuf::from("./data/replica.db"),
            replica_keep_versions: 10,
            ingest_host: None,
            ingest_port: 9667,
        }
    }
}

impl SchedulerConfig {
    /// Load configuration from file and environment variables
    pub fn load() -> Result<Self> {
        let mut builder = config::Config::builder()
            .add_source(config::Config::try_from(&SchedulerConfig::default())?);

        // Try to load from spooky.yml (optional)
        builder = builder.add_source(config::File::with_name("spooky").required(false));

        // Override with environment variables (SPOOKY_SCHEDULER_*)
        builder =
            builder.add_source(config::Environment::with_prefix("SPOOKY_SCHEDULER").separator("_"));

        let config = builder.build()?;
        let scheduler_config: SchedulerConfig = config.try_deserialize()?;

        Ok(scheduler_config)
    }
}
