pub mod backend_health;
pub mod backup;
pub mod config;
pub mod replica;
pub mod restore;
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

use anyhow::{Context, Result};

use std::collections::{BTreeSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::RwLock;
use tracing::{error, info, trace, warn};

use crate::config::SchedulerConfig;
use crate::messages::BufferedEvent;
use crate::replica::Replica;
use crate::router::SspPool;
use crate::transport::HttpTransport;
use crate::wal::EventWal;

/// Drain the in-memory event buffer and apply all events to the replica.
/// Also advances `snapshot_seq` and truncates the WAL up to that seq.
/// Returns the number of events applied (may be 0). Does NOT touch `SchedulerStatus`.
pub async fn drain_and_apply(
    event_buffer: &Arc<RwLock<VecDeque<BufferedEvent>>>,
    replica: &Arc<RwLock<Replica>>,
    wal: &Arc<RwLock<EventWal>>,
) -> Result<usize> {
    let events: Vec<BufferedEvent> = {
        let mut buffer = event_buffer.write().await;
        buffer.drain(..).collect()
    };

    if events.is_empty() {
        return Ok(0);
    }

    let event_count = events.len();
    let max_seq = events.last().map(|e| e.seq).unwrap_or(0);

    // Track which tables this batch touched so the snapshot-state writer
    // only rehashes the affected tables, not the whole replica.
    let touched: BTreeSet<String> = events
        .iter()
        .filter(|e| !e.update.table.starts_with("_00_"))
        .map(|e| e.update.table.clone())
        .collect();

    {
        let mut rep = replica.write().await;
        for event in &events {
            let op = match event.update.operation {
                crate::messages::RecordOp::Create => crate::replica::RecordOp::Create,
                crate::messages::RecordOp::Update => crate::replica::RecordOp::Update,
                crate::messages::RecordOp::Delete => crate::replica::RecordOp::Delete,
            };
            if let Err(e) = rep
                .apply(
                    &event.update.table,
                    op,
                    &event.update.record_id,
                    event.update.data.clone(),
                )
                .await
            {
                error!(seq = event.seq, error = ?e, "Failed to apply event to snapshot");
            }
        }
        if let Err(e) = rep.set_snapshot_state(max_seq, Some(&touched)).await {
            error!(error = %e, "Failed to persist snapshot state");
        }
    }

    {
        let mut wal_guard = wal.write().await;
        if let Err(e) = wal_guard.truncate(max_seq) {
            error!(error = %e, "Failed to truncate WAL");
        }
    }

    Ok(event_count)
}

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
    /// Restore in progress — ingest, register, proxy are all rejected
    Restoring,
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
        backend_health: crate::backend_health::BackendHealthCache,
        shared_backend_configs: crate::backend_health::SharedBackendConfigs,
    ) -> crate::metrics::MetricsState {
        crate::metrics::MetricsState {
            ssp_pool: Arc::clone(&self.ssp_pool),
            query_tracker,
            job_tracker,
            start_time: self.start_time,
            scheduler_id: self.config.scheduler_id.clone(),
            status: Arc::clone(&self.status),
            backend_health,
            shared_backend_configs,
            ingest: self.ingest_state(),
            replica: Arc::clone(&self.replica),
        }
    }

    /// Get proxy state for HTTP handlers
    pub fn proxy_state(&self) -> crate::proxy::ProxyState {
        crate::proxy::ProxyState {
            replica: Arc::clone(&self.replica),
            status: Arc::clone(&self.status),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn backup_state(
        &self,
        registry: Arc<crate::backup::BackupRegistry>,
        tx: tokio::sync::mpsc::Sender<crate::backup::BackupJob>,
        config: Arc<crate::backup::BackupConfig>,
        restore_registry: Arc<crate::restore::RestoreRegistry>,
        restore_tx: tokio::sync::mpsc::Sender<crate::restore::RestoreJob>,
        backup_restore_lock: Arc<tokio::sync::Mutex<()>>,
    ) -> crate::backup::BackupState {
        crate::backup::BackupState {
            replica: Arc::clone(&self.replica),
            ingest: self.ingest_state(),
            config,
            registry,
            tx,
            restore_registry,
            restore_tx,
            backup_restore_lock,
        }
    }

    pub fn restore_state(
        &self,
        registry: Arc<crate::restore::RestoreRegistry>,
        tx: tokio::sync::mpsc::Sender<crate::restore::RestoreJob>,
        s3_config: Arc<crate::backup::BackupConfig>,
        backup_restore_lock: Arc<tokio::sync::Mutex<()>>,
    ) -> crate::restore::RestoreState {
        crate::restore::RestoreState {
            replica: Arc::clone(&self.replica),
            ingest: self.ingest_state(),
            ssp_pool: Arc::clone(&self.ssp_pool),
            s3_config,
            db_config: Arc::new(self.config.db.clone()),
            registry,
            tx,
            backup_restore_lock,
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
        info!(
            url = %self.config.db.url,
            namespace = %self.config.db.namespace,
            database = %self.config.db.database,
            user = %self.config.db.username,
            "Bootstrap target: ns={} db={} url={}",
            self.config.db.namespace,
            self.config.db.database,
            self.config.db.url,
        );
        let ws_addr = self.config.db.url
            .strip_prefix("ws://")
            .or_else(|| self.config.db.url.strip_prefix("wss://"))
            .unwrap_or(&self.config.db.url);

        let db = surrealdb::Surreal::new::<surrealdb::engine::remote::ws::Ws>(
            ws_addr
        ).await?;

        db.signin(surrealdb::opt::auth::Root {
            username: self.config.db.username.clone(),
            password: self.config.db.password.clone(),
        }).await?;

        db.use_ns(&self.config.db.namespace)
            .use_db(&self.config.db.database)
            .await?;

        info!("Connected to SurrealDB");

        // Step 2: Clear stale registered views from the remote DB. Views are
        // tied to live SSPs/clients, so leftover `_00_query` rows from a prior
        // scheduler run point at SSPs that no longer exist. Wipe them before
        // cloning so the replica starts with a clean view registry; clients
        // will re-register against the fresh scheduler.
        info!("Clearing registered view data from remote SurrealDB...");
        trace!(ns = %self.config.db.namespace, db = %self.config.db.database, "remote query: DELETE _00_query");
        db.query("DELETE _00_query")
            .await
            .context("Failed to clear _00_query on remote")?;

        // Step 3: Clone remote DB into local snapshot replica — only when
        // there's nothing already persisted. A non-zero `snapshot_seq` means
        // `Replica::new` restored a real snapshot (with hashes + known tables)
        // from `_00_metadata:snapshot`, and re-cloning would wipe that state
        // and reset `snapshot_seq` to whatever the in-memory counter says.
        // `spky dev --clean` deletes `.sp00ky/scheduler_data`, which is what
        // forces `snapshot_seq == 0` and triggers a fresh clone here.
        let needs_bootstrap = {
            let replica = self.replica.read().await;
            replica.snapshot_seq() == 0
        };

        if needs_bootstrap {
            info!("No persisted snapshot found — cloning remote database...");
            let mut replica = self.replica.write().await;

            // The replica may still hold orphan records from a prior startup
            // that ingested some tables and then crashed before persisting
            // `_00_metadata:snapshot`. Without a wipe, `ingest_all` re-issues
            // `CREATE` on those rows and fails with "record already exists",
            // so reset before re-cloning. Safe because `needs_bootstrap` is
            // gated on `snapshot_seq == 0` — no committed snapshot to lose.
            replica.reset().await.context("Failed to reset replica before bootstrap")?;

            trace!(
                ns = %self.config.db.namespace,
                db = %self.config.db.database,
                "starting replica.ingest_all from remote"
            );
            replica.ingest_all(&db).await?;

            // Pass `None` for touched_tables so set_snapshot_state hashes
            // every table we just ingested — that hash is the integrity
            // baseline an SSP gets handed at /ssp/register.
            let current_seq = self.seq_counter.load(Ordering::SeqCst);
            replica.set_snapshot_state(current_seq, None).await?;

            let hashes = replica.snapshot_hashes();
            info!(
                tables = hashes.len(),
                "Snapshot integrity hashes computed: {:?}",
                hashes
                    .iter()
                    .map(|(t, h)| (t.as_str(), &h[..h.len().min(11)]))
                    .collect::<Vec<_>>(),
            );
            info!("Snapshot clone complete");
        } else {
            let replica = self.replica.read().await;
            info!(
                snapshot_seq = replica.snapshot_seq(),
                tables = replica.snapshot_hashes().len(),
                known_tables = replica.known_tables().len(),
                "Reusing persisted snapshot — skipping bootstrap clone"
            );
        }

        // Startup self-check: hash the replica fresh and compare against
        // what's persisted. Mismatch ⇒ the on-disk replica disagrees with
        // its own metadata (corruption, bad backup, manual edits) and we
        // can't trust it. Triggers a re-clone before we serve any SSPs.
        if let Err(e) = self.startup_integrity_check().await {
            warn!(error = %e, "Startup integrity check encountered errors");
        }

        // Transition to Ready
        *self.status.write().await = SchedulerStatus::Ready;
        info!("Scheduler is ready and running");

        // Step 3: Spawn periodic snapshot update task
        self.spawn_snapshot_updater();

        // Keep running until shutdown signal
        tokio::signal::ctrl_c().await?;

        Ok(())
    }

    /// Recompute every table's hash from the current replica state and
    /// compare against the persisted `snapshot_hashes`. Logs each mismatch
    /// and returns Ok regardless — the caller decides whether to escalate.
    async fn startup_integrity_check(&self) -> Result<()> {
        let replica = self.replica.read().await;
        let persisted = replica.snapshot_hashes().clone();
        let fresh = replica.compute_table_hashes().await?;

        let diffs = ssp_protocol::snapshot_hash::diff_table_hashes(&persisted, &fresh);
        if diffs.is_empty() {
            info!(tables = persisted.len(), "Startup integrity check passed");
            return Ok(());
        }

        for d in &diffs {
            error!(
                table = %d.table,
                persisted = %d.a,
                actual = %d.b,
                "Startup integrity mismatch"
            );
        }
        error!(
            count = diffs.len(),
            "Replica disagrees with persisted snapshot hashes — POST /admin/resync to re-clone"
        );
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

                match drain_and_apply(&event_buffer, &replica, &wal).await {
                    Ok(0) => {}
                    Ok(event_count) => {
                        info!(event_count, "Snapshot update complete");
                    }
                    Err(e) => {
                        error!(error = %e, "Snapshot update failed");
                    }
                }

                // Set status back to Ready
                *status.write().await = SchedulerStatus::Ready;
            }
        });
    }
}
