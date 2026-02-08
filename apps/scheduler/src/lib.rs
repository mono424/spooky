pub mod config;
pub mod replica;
pub mod router;
pub mod job_scheduler;
pub mod transport;

use anyhow::Result;
use futures::StreamExt;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, error, warn};

use crate::config::SchedulerConfig;
use crate::replica::Replica;
use crate::router::SspPool;
use crate::transport::Transport;

/// Main Scheduler service that orchestrates SurrealDB and SSP sidecars
pub struct Scheduler {
    config: SchedulerConfig,
    transport: Arc<dyn Transport>,
    replica: Arc<RwLock<Replica>>,
    ssp_pool: Arc<RwLock<SspPool>>,
}

impl Scheduler {
    /// Create a new Scheduler instance
    pub fn new(config: SchedulerConfig, transport: Arc<dyn Transport>) -> Self {
        let strategy = config.load_balance.clone();
        Self {
            config,
            transport,
            replica: Arc::new(RwLock::new(Replica::new())),
            ssp_pool: Arc::new(RwLock::new(SspPool::new(strategy))),
        }
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

        // Step 3: Subscribe to LIVE SELECT for all tables
        info!("Setting up LIVE SELECT subscriptions...");
        self.start_live_select().await?;

        // Step 4: Start heartbeat listener for SSP discovery
        info!("Starting SSP heartbeat listener...");
        self.start_heartbeat_listener().await?;

        // Step 5: Start job scheduling loop (Phase 4 - not implemented yet)
        info!("Job scheduler will be implemented in Phase 4");

        info!("Scheduler is ready and running");

        // Keep running until shutdown signal
        tokio::signal::ctrl_c().await?;

        Ok(())
    }

    /// Subscribe to LIVE SELECT for all tables and broadcast updates
    async fn start_live_select(&self) -> Result<()> {
        // Get list of tables from schema
        // For now, we'll use a hardcoded list, but this should be discovered from schema
        let tables = vec!["thread", "job", "user"]; // TODO: discover from schema

        for table in tables {
            let replica = Arc::clone(&self.replica);
            let transport = Arc::clone(&self.transport);
            let table_name = table.to_string();
            let config = self.config.clone();

            tokio::spawn(async move {
                if let Err(e) = Self::subscribe_to_table(&config, &table_name, replica, transport).await {
                    error!("Error in LIVE SELECT for table '{}': {}", table_name, e);
                }
            });
        }

        Ok(())
    }

    /// Subscribe to updates for a specific table
    async fn subscribe_to_table(
        config: &SchedulerConfig,
        table: &str,
        _replica: Arc<RwLock<Replica>>,
        transport: Arc<dyn Transport>,
    ) -> Result<()> {
        // Create a new DB connection for this task
        let db = surrealdb::Surreal::new::<surrealdb::engine::remote::ws::Ws>(config.db.url.as_str()).await?;
        
        db.signin(surrealdb::opt::auth::Root {
            username: &config.db.username,
            password: &config.db.password,
        }).await?;

        db.use_ns(&config.db.namespace)
            .use_db(&config.db.database)
            .await?;

        // For now, we'll just log that LIVE SELECT would happen here
        // Full implementation requires understanding SurrealDB's LIVE SELECT API
        warn!("LIVE SELECT for table '{}' - full implementation pending", table);
        
        // TODO: Implement actual LIVE SELECT subscription
        // This requires:
        // 1. Execute LIVE SELECT query on SurrealDB
        // 2. Stream events
        // 3. Update replica with each event
        // 4. Broadcast to SSPs via transport
        
        // Placeholder to keep the task alive
        tokio::time::sleep(std::time::Duration::from_secs(3600)).await;

        Ok(())
    }

    /// Listen for SSP heartbeats and track pool
    async fn start_heartbeat_listener(&self) -> Result<()> {
        let transport = Arc::clone(&self.transport);
        let ssp_pool = Arc::clone(&self.ssp_pool);

        tokio::spawn(async move {
            match transport.subscribe("spooky.ssp.heartbeat").await {
                Ok(mut stream) => {
                    info!("Listening for SSP heartbeats...");
                    while let Some(_msg) = stream.next().await {
                        // Parse heartbeat message and update SSP pool
                        // TODO: implement heartbeat parsing and pool update in Phase 2
                        let _pool = ssp_pool.write().await;
                        // pool.upsert(ssp_info);
                    }
                }
                Err(e) => error!("Failed to subscribe to heartbeat: {}", e),
            }
        });

        Ok(())
    }

    /// Gracefully shutdown the scheduler
    pub async fn shutdown(&self) -> Result<()> {
        info!("Shutting down Scheduler service...");
        // TODO: implement graceful shutdown
        Ok(())
    }
}
