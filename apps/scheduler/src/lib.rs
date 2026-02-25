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
pub mod wal;
pub mod proxy;

use anyhow::Result;

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::RwLock;
use tracing::{error, info, warn};

use crate::config::SchedulerConfig;
use crate::messages::BufferedEvent;
use crate::replica::Replica;
use crate::router::SspPool;
use crate::transport::HttpTransport;
use crate::wal::EventWal;

/// Scheduler lifecycle status
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SchedulerStatus {
    /// Initial DB clone in progress
    Cloning,
    /// Normal operation, snapshot unfrozen
    Ready,
    /// Snapshot frozen — SSP bootstrapping or catching up
    SnapshotFrozen,
    /// Batch-applying buffered events to snapshot
    SnapshotUpdating,
}

/// Main Scheduler service that orchestrates SurrealDB and SSP sidecars
pub struct Scheduler {
    config: SchedulerConfig,
    transport: Arc<HttpTransport>,
    pub replica: Arc<RwLock<Replica>>,
    pub ssp_pool: Arc<RwLock<SspPool>>,
    pub status: Arc<RwLock<SchedulerStatus>>,
    pub event_buffer: Arc<RwLock<VecDeque<BufferedEvent>>>,
    pub seq_counter: Arc<AtomicU64>,
    pub wal: Arc<RwLock<EventWal>>,
    start_time: std::time::Instant,
}

impl Scheduler {
    /// Create a new Scheduler instance
    pub async fn new(config: SchedulerConfig, transport: Arc<HttpTransport>) -> Result<Self> {
        let strategy = config.load_balance.clone();

        // Initialize persistent replica with embedded SurrealDB/RocksDB
        let replica = Replica::new(
            config.replica_db_path.clone(),
        ).await?;

        // Initialize WAL
        let wal = EventWal::new(config.wal_path.clone())?;

        // Recover state from WAL if available
        let snapshot_seq = replica.snapshot_seq();
        let recovered_events = wal.recover()?;
        let recovered_count = recovered_events.len();

        // Determine seq_counter from WAL or snapshot
        let max_wal_seq = recovered_events.last().map(|e| e.seq).unwrap_or(0);
        let initial_seq = max_wal_seq.max(snapshot_seq);

        // Rebuild event buffer from WAL (only events after snapshot)
        let event_buffer: VecDeque<BufferedEvent> = recovered_events
            .into_iter()
            .filter(|e| e.seq > snapshot_seq)
            .collect();

        if recovered_count > 0 {
            info!(
                recovered_count,
                buffer_size = event_buffer.len(),
                snapshot_seq,
                initial_seq,
                "Recovered events from WAL"
            );
        }

        let max_buffer_per_ssp = config.max_buffer_per_ssp;
        Ok(Self {
            config,
            transport,
            replica: Arc::new(RwLock::new(replica)),
            ssp_pool: Arc::new(RwLock::new(SspPool::new(strategy, max_buffer_per_ssp))),
            status: Arc::new(RwLock::new(SchedulerStatus::Cloning)),
            event_buffer: Arc::new(RwLock::new(event_buffer)),
            seq_counter: Arc::new(AtomicU64::new(initial_seq)),
            wal: Arc::new(RwLock::new(wal)),
            start_time: std::time::Instant::now(),
        })
    }

    /// Get ingest state for HTTP handlers
    pub fn ingest_state(&self) -> crate::ingest::IngestState {
        crate::ingest::IngestState {
            replica: Arc::clone(&self.replica),
            transport: Arc::clone(&self.transport),
            ssp_pool: Arc::clone(&self.ssp_pool),
            status: Arc::clone(&self.status),
            event_buffer: Arc::clone(&self.event_buffer),
            seq_counter: Arc::clone(&self.seq_counter),
            wal: Arc::clone(&self.wal),
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

    /// Get proxy state for HTTP handlers
    pub fn proxy_state(&self) -> crate::proxy::ProxyState {
        crate::proxy::ProxyState {
            replica: Arc::clone(&self.replica),
        }
    }

    /// Get config
    pub fn config(&self) -> &SchedulerConfig {
        &self.config
    }

    /// Graceful shutdown
    pub async fn shutdown(&self) -> Result<()> {
        info!("Scheduler shutting down gracefully...");

        // Log replica state
        {
            let replica = self.replica.read().await;
            let count = replica.record_count().await.unwrap_or(0);
            info!("Replica has {} records, snapshot_seq={}", count, replica.snapshot_seq());
        }

        info!("Scheduler shutdown complete");
        Ok(())
    }

    /// Start the scheduler service
    pub async fn start(&self) -> Result<()> {
        info!("Starting Scheduler service...");

        // Step 1: Connect to remote SurrealDB
        info!("Connecting to SurrealDB at {}", self.config.db.url);
        let db = surrealdb::Surreal::new::<surrealdb::engine::remote::ws::Ws>(
            self.config.db.url.as_str()
        ).await?;

        db.signin(surrealdb::opt::auth::Root {
            username: &self.config.db.username,
            password: &self.config.db.password,
        }).await?;

        db.use_ns(&self.config.db.namespace)
            .use_db(&self.config.db.database)
            .await?;

        info!("Connected to SurrealDB");

        // Step 2: Clone remote DB into local snapshot replica
        info!("Cloning remote database into local snapshot...");
        {
            let mut replica = self.replica.write().await;
            replica.ingest_all(&db).await?;

            // Set snapshot_seq to current seq_counter (snapshot is up to date at this point)
            let current_seq = self.seq_counter.load(Ordering::SeqCst);
            replica.set_snapshot_seq(current_seq).await?;
        }
        info!("Snapshot clone complete");

        // Transition to Ready
        *self.status.write().await = SchedulerStatus::Ready;
        info!("Scheduler is ready and running");

        // Step 3: Spawn periodic snapshot update task
        self.spawn_snapshot_updater();

        // Keep running until shutdown signal
        tokio::signal::ctrl_c().await?;

        Ok(())
    }

    /// Spawn a background task that periodically applies buffered events to the snapshot
    fn spawn_snapshot_updater(&self) {
        let interval_secs = self.config.snapshot_update_interval_secs;
        let status = Arc::clone(&self.status);
        let event_buffer = Arc::clone(&self.event_buffer);
        let replica = Arc::clone(&self.replica);
        let ssp_pool = Arc::clone(&self.ssp_pool);
        let wal = Arc::clone(&self.wal);

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(
                std::time::Duration::from_secs(interval_secs)
            );
            // Skip the first immediate tick
            interval.tick().await;

            loop {
                interval.tick().await;

                // Check if any SSPs are bootstrapping/replaying — skip if so
                let has_active_bootstrap = {
                    let pool = ssp_pool.read().await;
                    pool.has_active_bootstrap()
                };

                if has_active_bootstrap {
                    info!("Skipping snapshot update: SSPs are bootstrapping");
                    continue;
                }

                // Check scheduler status
                let current_status = *status.read().await;
                if current_status != SchedulerStatus::Ready {
                    info!("Skipping snapshot update: scheduler status is {:?}", current_status);
                    continue;
                }

                // Set status to SnapshotUpdating
                *status.write().await = SchedulerStatus::SnapshotUpdating;

                // Drain event buffer and apply to snapshot
                let events: Vec<BufferedEvent> = {
                    let mut buffer = event_buffer.write().await;
                    buffer.drain(..).collect()
                };

                if events.is_empty() {
                    *status.write().await = SchedulerStatus::Ready;
                    continue;
                }

                let event_count = events.len();
                let max_seq = events.last().map(|e| e.seq).unwrap_or(0);

                info!(event_count, max_seq, "Applying buffered events to snapshot");

                // Apply events to replica
                {
                    let mut rep = replica.write().await;
                    for event in &events {
                        let op = match event.update.operation {
                            crate::messages::RecordOp::Create => crate::replica::RecordOp::Create,
                            crate::messages::RecordOp::Update => crate::replica::RecordOp::Update,
                            crate::messages::RecordOp::Delete => crate::replica::RecordOp::Delete,
                        };
                        if let Err(e) = rep.apply(
                            &event.update.table,
                            op,
                            &event.update.record_id,
                            event.update.data.clone(),
                        ).await {
                            error!(
                                seq = event.seq,
                                error = %e,
                                "Failed to apply event to snapshot"
                            );
                        }
                    }

                    // Update snapshot_seq
                    if let Err(e) = rep.set_snapshot_seq(max_seq).await {
                        error!(error = %e, "Failed to persist snapshot_seq");
                    }
                }

                // Truncate WAL
                {
                    let mut wal_guard = wal.write().await;
                    if let Err(e) = wal_guard.truncate(max_seq) {
                        error!(error = %e, "Failed to truncate WAL");
                    }
                }

                info!(event_count, max_seq, "Snapshot update complete");

                // Set status back to Ready
                *status.write().await = SchedulerStatus::Ready;
            }
        });
    }
}
