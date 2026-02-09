pub mod config;
pub mod replica;
pub mod router;
pub mod job_scheduler;
pub mod transport;
pub mod messages;
pub mod ingest;
pub mod query;
pub mod metrics;

use anyhow::Result;
use futures::StreamExt;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, error, warn, debug};

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
    start_time: std::time::Instant,
}

impl Scheduler {
    /// Create a new Scheduler instance
    pub fn new(config: SchedulerConfig, transport: Arc<dyn Transport>) -> Result<Self> {
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

        // Step 3: Start bootstrap request handler
        info!("Starting bootstrap request handler...");
        self.start_bootstrap_handler().await?;

        // Step 4: Subscribe to LIVE SELECT for all tables
        info!("Setting up LIVE SELECT subscriptions...");
        self.start_live_select().await?;

        // Step 5: Start heartbeat listener for SSP discovery
        info!("Starting SSP heartbeat listener...");
        self.start_heartbeat_listener().await?;

        // Step 6: Start job scheduling loop (Phase 4 - not implemented yet)
        info!("Job scheduler will be implemented in Phase 4");

        info!("Scheduler is ready and running");

        // Keep running until shutdown signal
        tokio::signal::ctrl_c().await?;

        Ok(())
    }

    /// Placeholder for LIVE SELECT - using event-driven ingest instead
    /// Database events will call ingest endpoint to update replica
    async fn start_live_select(&self) -> Result<()> {
        info!("LIVE SELECT not needed - using event-driven ingest from database events");
        // TODO: Implement ingest HTTP/RPC endpoint for database events
        // When events occur: db_event -> ingest(table, op, id, data) -> replica.apply() -> broadcast to SSPs
        Ok(())
    }

    /// Listen for SSP heartbeats and track pool
    async fn start_heartbeat_listener(&self) -> Result<()> {
        let transport = Arc::clone(&self.transport);
        let ssp_pool = Arc::clone(&self.ssp_pool);

        info!("Starting SSP heartbeat listener...");
        
        let mut stream = self.transport
            .subscribe("spooky.ssp.heartbeat")
            .await?;
        
        while let Some(msg) = stream.next().await {
            // Parse heartbeat message
            match serde_json::from_slice::<serde_json::Value>(&msg.payload) {
                Ok(heartbeat) => {
                    // Extract SSP info
                    if let Some(ssp_id) = heartbeat.get("ssp_id").and_then(|v| v.as_str()) {
                        let active_queries = heartbeat.get("active_queries")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0) as usize;
                        let cpu_usage = heartbeat.get("cpu_usage").and_then(|v| v.as_f64());
                        let memory_usage = heartbeat.get("memory_usage").and_then(|v| v.as_f64());
                        
                        // Update SSP pool
                        let mut pool = self.ssp_pool.write().await;
                        pool.update_ssp(ssp_id, active_queries, cpu_usage, memory_usage);
                        
                        debug!("Received heartbeat from SSP '{}'", ssp_id);
                    } else {
                        warn!("Received heartbeat without ssp_id");
                    }
                }
                Err(e) => {
                    warn!("Failed to parse heartbeat message: {}", e);
                }
            }
        }
        
        Ok(())
    }

    /// Handle bootstrap requests from SSPs
    async fn start_bootstrap_handler(&self) -> Result<()> {
        let transport = Arc::clone(&self.transport);
        let replica = Arc::clone(&self.replica);
        let ssp_pool = Arc::clone(&self.ssp_pool);
        let chunk_size = self.config.bootstrap_chunk_size;

        tokio::spawn(async move {
            match transport.subscribe("spooky.bootstrap.request").await {
                Ok(mut stream) => {
                    info!("Listening for bootstrap requests...");
                    
                    while let Some(msg) = stream.next().await {
                        // Parse bootstrap request
                        match serde_json::from_slice::<crate::messages::BootstrapRequest>(&msg.payload) {
                            Ok(request) => {
                                info!("Received bootstrap request from SSP '{}'", request.ssp_id);
                                
                                // Mark SSP as bootstrapping
                                {
                                    let mut pool = ssp_pool.write().await;
                                    pool.mark_bootstrapping(&request.ssp_id);
                                    info!("Marked SSP '{}' as bootstrapping", request.ssp_id);
                                }
                                
                                // Get chunks from replica
                                let chunks = {
                                    let replica = replica.read().await;
                                    match replica.iter_chunks(chunk_size) {
                                        Ok(chunks) => chunks,
                                        Err(e) => {
                                            error!("Failed to get chunks from replica: {}", e);
                                            continue;
                                        }
                                    }
                                };
                                
                                info!("Sending {} chunks to SSP '{}'", chunks.len(), request.ssp_id);
                                
                                // Convert replica chunks to bootstrap chunks
                                let bootstrap_chunks: Vec<crate::messages::BootstrapChunk> = chunks
                                    .iter()
                                    .map(|chunk| crate::messages::BootstrapChunk {
                                        chunk_index: chunk.chunk_index,
                                        total_chunks: chunks.len(),
                                        table: chunk.table.clone(),
                                        records: chunk.records.clone(),
                                    })
                                    .collect();
                                
                                let response = crate::messages::BootstrapResponse {
                                    ssp_id: request.ssp_id.clone(),
                                    chunks: bootstrap_chunks,
                                };
                                
                                // Send response
                                if let Some(reply_to) = msg.reply_to {
                                    match serde_json::to_vec(&response) {
                                        Ok(payload) => {
                                            if let Err(e) = transport.broadcast(&reply_to, &payload).await {
                                                error!("Failed to send bootstrap response: {}", e);
                                            } else {
                                                info!("Bootstrap response sent to SSP '{}'", request.ssp_id);
                                            }
                                        }
                                        Err(e) => {
                                            error!("Failed to serialize bootstrap response: {}", e);
                                        }
                                    }
                                } else {
                                    warn!("Bootstrap request missing reply_to field");
                                }
                            }
                            Err(e) => {
                                warn!("Failed to parse bootstrap request: {}", e);
                            }
                        }
                    }
                }
                Err(e) => {
                    error!("Failed to subscribe to bootstrap requests: {}", e);
                }
            }
        });

        Ok(())
    }
}
