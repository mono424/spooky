pub mod config;
pub mod replica;
pub mod router;
pub mod job_scheduler;
pub mod transport;
pub mod messages;
pub mod ingest;
pub mod query;
pub mod metrics;
pub mod ssp_management;

use anyhow::Result;

use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

use crate::config::SchedulerConfig;
use crate::replica::Replica;
use crate::router::SspPool;
use crate::transport::HttpTransport;

/// Main Scheduler service that orchestrates SurrealDB and SSP sidecars
pub struct Scheduler {
    config: SchedulerConfig,
    transport: Arc<HttpTransport>,
    pub replica: Arc<RwLock<Replica>>,
    ssp_pool: Arc<RwLock<SspPool>>,
    start_time: std::time::Instant,
}

impl Scheduler {
    /// Create a new Scheduler instance
    pub fn new(config: SchedulerConfig, transport: Arc<HttpTransport>) -> Result<Self> {
        let strategy = config.load_balance.clone();
        
        // Initialize persistent replica
        let replica = Replica::new(
            config.replica_db_path.clone(),
            config.replica_keep_versions,
        )?;
        
        Ok(Self {
            config,
            transport,
            replica: Arc::new(RwLock::new(replica)),
            ssp_pool: Arc::new(RwLock::new(SspPool::new(strategy))),
            start_time: std::time::Instant::now(),
        })
    }

    /// Get ingest state for HTTP handlers
    pub fn ingest_state(&self) -> crate::ingest::IngestState {
        crate::ingest::IngestState {
            replica: Arc::clone(&self.replica),
            transport: Arc::clone(&self.transport),
            ssp_pool: Arc::clone(&self.ssp_pool),
        }
    }

    /// Get query state for HTTP handlers
    pub fn query_state(&self) -> crate::query::QueryState {
        crate::query::QueryState {
            ssp_pool: Arc::clone(&self.ssp_pool),
            transport: Arc::clone(&self.transport),
            query_tracker: Arc::new(crate::query::QueryTracker::new()),
        }
    }

    /// Get job state for HTTP handlers
    pub fn job_state(&self) -> crate::job_scheduler::JobState {
        crate::job_scheduler::JobState {
            ssp_pool: Arc::clone(&self.ssp_pool),
            transport: Arc::clone(&self.transport),
            job_tracker: Arc::new(crate::job_scheduler::JobTracker::new()),
        }
    }

    /// Get metrics state for HTTP handlers
    pub fn metrics_state(
        &self,
        query_tracker: Arc<crate::query::QueryTracker>,
        job_tracker: Arc<crate::job_scheduler::JobTracker>,
    ) -> crate::metrics::MetricsState {
        crate::metrics::MetricsState {
            ssp_pool: Arc::clone(&self.ssp_pool),
            query_tracker,
            job_tracker,
            start_time: self.start_time,
        }
    }

    /// Get config
    pub fn config(&self) -> &SchedulerConfig {
        &self.config
    }

    /// Graceful shutdown
    pub async fn shutdown(&self) -> Result<()> {
        info!("Scheduler shutting down gracefully...");
        
        // Persist replica state
        {
            let replica = self.replica.read().await;
            let count = replica.record_count().unwrap_or(0);
            info!("Replica has {} records", count);
        }
        
        info!("Scheduler shutdown complete");
        Ok(())
    }

    /// Start the scheduler service
    pub async fn start(&self) -> Result<()> {
        info!("Starting Scheduler service...");

        // Step 1: Connect to SurrealDB
        info!("Connecting to SurrealDB at {}", self.config.db.url);
        let db = surrealdb::Surreal::new::<surrealdb::engine::remote::ws::Ws>(self.config.db.url.as_str()).await?;
        
        db.signin(surrealdb::opt::auth::Root {
            username: &self.config.db.username,
            password: &self.config.db.password,
        }).await?;

        db.use_ns(&self.config.db.namespace)
            .use_db(&self.config.db.database)
            .await?;

        info!("Connected to SurrealDB");

        // Step 2: Ingest all existing records into in-memory replica
        info!("Ingesting existing records into replica...");
        {
            let mut replica = self.replica.write().await;
            replica.ingest_all(&db).await?;
        }
        info!("Replica ingestion complete");

        // Note: SSP registration and heartbeats are now received via HTTP POST endpoints
        // Bootstrap is triggered via HTTP when an SSP registers

        info!("Scheduler is ready and running");

        // Keep running until shutdown signal
        tokio::signal::ctrl_c().await?;

        Ok(())
    }


}
