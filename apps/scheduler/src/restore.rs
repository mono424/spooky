use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use flate2::read::GzDecoder;
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::io::Read;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex, RwLock};
use tracing::{error, info, warn};

use crate::backup::BackupConfig;
use crate::config::DbConfig;
use crate::ingest::IngestState;
use crate::replica::Replica;
use crate::router::SspPool;
use crate::SchedulerStatus;

const RECENT_JOB_LIMIT: usize = 50;
const RESTORE_QUEUE_CAPACITY: usize = 8;
const BOOTSTRAP_DRAIN_TIMEOUT_SECS: u64 = 10;

#[derive(Clone)]
pub struct RestoreState {
    pub replica: Arc<RwLock<Replica>>,
    pub ingest: IngestState,
    pub ssp_pool: Arc<RwLock<SspPool>>,
    pub s3_config: Arc<BackupConfig>,
    pub db_config: Arc<DbConfig>,
    pub registry: Arc<RestoreRegistry>,
    pub tx: mpsc::Sender<RestoreJob>,
    pub backup_restore_lock: Arc<Mutex<()>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RestoreStatus {
    Queued,
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct RestoreJobState {
    pub restore_id: String,
    pub backup_id: String,
    pub project_slug: String,
    pub storage_path: String,
    pub status: RestoreStatus,
    pub enqueued_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub snapshot_seq: Option<u64>,
    pub pending_cleared: Option<usize>,
    pub main_db_restored: bool,
    pub replica_restored: bool,
    pub ssps_evicted: Option<usize>,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RestoreJob {
    pub restore_id: String,
    pub backup_id: String,
    pub project_slug: String,
    pub storage_path: String,
}

pub struct RestoreRegistry {
    jobs: RwLock<HashMap<String, RestoreJobState>>,
    order: RwLock<VecDeque<String>>,
}

impl RestoreRegistry {
    pub fn new() -> Self {
        Self {
            jobs: RwLock::new(HashMap::new()),
            order: RwLock::new(VecDeque::new()),
        }
    }

    pub async fn contains(&self, id: &str) -> bool {
        self.jobs.read().await.contains_key(id)
    }

    pub async fn enqueue(
        &self,
        restore_id: String,
        backup_id: String,
        project_slug: String,
        storage_path: String,
    ) -> RestoreJobState {
        let state = RestoreJobState {
            restore_id: restore_id.clone(),
            backup_id,
            project_slug,
            storage_path,
            status: RestoreStatus::Queued,
            enqueued_at: Utc::now(),
            started_at: None,
            finished_at: None,
            snapshot_seq: None,
            pending_cleared: None,
            main_db_restored: false,
            replica_restored: false,
            ssps_evicted: None,
            error: None,
        };
        self.jobs.write().await.insert(restore_id.clone(), state.clone());
        self.order.write().await.push_back(restore_id);
        state
    }

    async fn update<F: FnOnce(&mut RestoreJobState)>(&self, id: &str, f: F) {
        if let Some(state) = self.jobs.write().await.get_mut(id) {
            f(state);
        }
    }

    pub async fn mark_running(&self, id: &str) {
        self.update(id, |s| {
            s.status = RestoreStatus::Running;
            s.started_at = Some(Utc::now());
        })
        .await;
    }

    pub async fn mark_completed(&self, id: &str, outcome: RestoreOutcome) {
        self.update(id, |s| {
            s.status = RestoreStatus::Completed;
            s.finished_at = Some(Utc::now());
            s.snapshot_seq = Some(outcome.snapshot_seq);
            s.pending_cleared = Some(outcome.pending_cleared);
            s.main_db_restored = outcome.main_db_restored;
            s.replica_restored = outcome.replica_restored;
            s.ssps_evicted = Some(outcome.ssps_evicted);
        })
        .await;
        self.trim().await;
    }

    pub async fn mark_failed(&self, id: &str, err: String, progress: RestoreProgress) {
        self.update(id, |s| {
            s.status = RestoreStatus::Failed;
            s.finished_at = Some(Utc::now());
            s.error = Some(err);
            s.main_db_restored = progress.main_db_restored;
            s.replica_restored = progress.replica_restored;
        })
        .await;
        self.trim().await;
    }

    async fn trim(&self) {
        let mut jobs = self.jobs.write().await;
        let mut order = self.order.write().await;
        let finished: Vec<String> = order
            .iter()
            .filter(|id| {
                jobs.get(id.as_str())
                    .map(|s| matches!(s.status, RestoreStatus::Completed | RestoreStatus::Failed))
                    .unwrap_or(false)
            })
            .cloned()
            .collect();
        if finished.len() <= RECENT_JOB_LIMIT {
            return;
        }
        let to_drop = finished.len() - RECENT_JOB_LIMIT;
        for id in finished.into_iter().take(to_drop) {
            jobs.remove(&id);
            if let Some(pos) = order.iter().position(|x| x == &id) {
                order.remove(pos);
            }
        }
    }

    pub async fn get(&self, id: &str) -> Option<RestoreJobState> {
        self.jobs.read().await.get(id).cloned()
    }

    pub async fn current_running(&self) -> Option<RestoreJobState> {
        self.jobs
            .read()
            .await
            .values()
            .find(|s| matches!(s.status, RestoreStatus::Running))
            .cloned()
    }

    pub async fn queue_len(&self) -> usize {
        self.jobs
            .read()
            .await
            .values()
            .filter(|s| matches!(s.status, RestoreStatus::Queued))
            .count()
    }

    pub async fn recent(&self) -> Vec<RestoreJobState> {
        let jobs = self.jobs.read().await;
        let order = self.order.read().await;
        order
            .iter()
            .filter_map(|id| jobs.get(id).cloned())
            .collect()
    }
}

impl Default for RestoreRegistry {
    fn default() -> Self {
        Self::new()
    }
}

pub fn create_restore_channel() -> (mpsc::Sender<RestoreJob>, mpsc::Receiver<RestoreJob>) {
    mpsc::channel(RESTORE_QUEUE_CAPACITY)
}

#[derive(Debug, Default, Clone, Copy)]
pub struct RestoreProgress {
    pub main_db_restored: bool,
    pub replica_restored: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct RestoreOutcome {
    pub snapshot_seq: u64,
    pub pending_cleared: usize,
    pub main_db_restored: bool,
    pub replica_restored: bool,
    pub ssps_evicted: usize,
}

#[allow(clippy::too_many_arguments)]
pub async fn run_restore_worker(
    mut rx: mpsc::Receiver<RestoreJob>,
    replica: Arc<RwLock<Replica>>,
    ingest: IngestState,
    ssp_pool: Arc<RwLock<SspPool>>,
    s3_config: Arc<BackupConfig>,
    db_config: Arc<DbConfig>,
    registry: Arc<RestoreRegistry>,
    lock: Arc<Mutex<()>>,
) {
    info!("Restore worker started");
    while let Some(job) = rx.recv().await {
        registry.mark_running(&job.restore_id).await;
        info!(
            restore_id = %job.restore_id,
            backup_id = %job.backup_id,
            project = %job.project_slug,
            "Restore worker running job"
        );

        let mut progress = RestoreProgress::default();
        match execute_restore(
            &job,
            &replica,
            &ingest,
            &ssp_pool,
            &s3_config,
            &db_config,
            &lock,
            &mut progress,
        )
        .await
        {
            Ok(outcome) => {
                registry.mark_completed(&job.restore_id, outcome).await;
                info!(
                    restore_id = %job.restore_id,
                    snapshot_seq = outcome.snapshot_seq,
                    pending_cleared = outcome.pending_cleared,
                    ssps_evicted = outcome.ssps_evicted,
                    "Restore completed"
                );
            }
            Err(e) => {
                let err_str = format!("{:#}", e);
                registry
                    .mark_failed(&job.restore_id, err_str.clone(), progress)
                    .await;
                error!(
                    restore_id = %job.restore_id,
                    error = %err_str,
                    main_db_restored = progress.main_db_restored,
                    replica_restored = progress.replica_restored,
                    "Restore failed"
                );
            }
        }
    }
    info!("Restore worker exiting (channel closed)");
}

#[allow(clippy::too_many_arguments)]
async fn execute_restore(
    job: &RestoreJob,
    replica: &Arc<RwLock<Replica>>,
    ingest: &IngestState,
    ssp_pool: &Arc<RwLock<SspPool>>,
    s3_config: &BackupConfig,
    db_config: &DbConfig,
    lock: &Arc<Mutex<()>>,
    progress: &mut RestoreProgress,
) -> Result<RestoreOutcome> {
    // 1. Download the gzipped dump from S3.
    let bucket = s3_config
        .get_bucket()
        .context("Failed to build S3 bucket client")?;
    let storage_path: &str = job.storage_path.as_str();
    let resp = bucket
        .get_object(storage_path)
        .await
        .with_context(|| format!("Failed to download {} from S3", job.storage_path))?;
    if resp.status_code() != 200 {
        anyhow::bail!(
            "S3 download returned status {} for {}",
            resp.status_code(),
            job.storage_path
        );
    }

    // 2. Gunzip to a tempfile.
    let tmp = tempfile::NamedTempFile::new().context("Failed to create tempfile for dump")?;
    {
        let compressed: Vec<u8> = resp.bytes().to_vec();
        let mut decoder = GzDecoder::new(std::io::Cursor::new(compressed));
        let mut raw = Vec::new();
        decoder
            .read_to_end(&mut raw)
            .context("Failed to decompress S3 artifact")?;
        std::fs::write(tmp.path(), &raw).context("Failed to write dump tempfile")?;
    }
    let dump_path = tmp.path().to_path_buf();

    // 3. Serialize with the backup worker — only one DB-mutating op at a time.
    let _guard = lock.lock().await;

    // 4. Transition Ready → Restoring. Refuse if scheduler isn't Ready.
    {
        let mut status = ingest.status.write().await;
        if *status != SchedulerStatus::Ready {
            anyhow::bail!(
                "Cannot restore: scheduler status is {:?}, expected Ready",
                *status
            );
        }
        *status = SchedulerStatus::Restoring;
        info!(restore_id = %job.restore_id, "Scheduler status → Restoring");
    }

    // From here on, any early return must decide whether to reset status to
    // Ready (safe failure, no DB mutated) or leave it Restoring (unsafe,
    // operator intervention required). We track that via `progress`.
    let result = execute_restore_inner(
        job,
        &dump_path,
        replica,
        ingest,
        ssp_pool,
        db_config,
        progress,
    )
    .await;

    match &result {
        Ok(_) => {
            *ingest.status.write().await = SchedulerStatus::Ready;
            info!(restore_id = %job.restore_id, "Scheduler status → Ready");
        }
        Err(e) => {
            if progress.replica_restored {
                // Replica import succeeded but a later step (pending-clear or
                // SSP eviction) failed. Replica + main are consistent; it's
                // safe to return to Ready.
                *ingest.status.write().await = SchedulerStatus::Ready;
                warn!(
                    restore_id = %job.restore_id,
                    error = %e,
                    "Restore post-import step failed; status → Ready anyway"
                );
            } else if progress.main_db_restored {
                // Main DB wiped/imported but replica is still the pre-restore
                // state or has been reset without import — serving reads would
                // return wrong data. Leave Restoring to block traffic.
                warn!(
                    restore_id = %job.restore_id,
                    error = %e,
                    "Restore partial failure after main DB changed; status stays Restoring"
                );
            } else {
                // Nothing mutated; safe to recover.
                *ingest.status.write().await = SchedulerStatus::Ready;
                warn!(
                    restore_id = %job.restore_id,
                    error = %e,
                    "Restore failed before any DB mutation; status → Ready"
                );
            }
        }
    }

    result
}

#[allow(clippy::too_many_arguments)]
async fn execute_restore_inner(
    job: &RestoreJob,
    dump_path: &std::path::Path,
    replica: &Arc<RwLock<Replica>>,
    ingest: &IngestState,
    ssp_pool: &Arc<RwLock<SspPool>>,
    db_config: &DbConfig,
    progress: &mut RestoreProgress,
) -> Result<RestoreOutcome> {
    // 5. Wait (bounded) for in-flight SSP bootstraps to drain.
    let deadline = std::time::Instant::now()
        + std::time::Duration::from_secs(BOOTSTRAP_DRAIN_TIMEOUT_SECS);
    loop {
        let active = ssp_pool.read().await.has_active_bootstrap();
        if !active {
            break;
        }
        if std::time::Instant::now() >= deadline {
            warn!(
                restore_id = %job.restore_id,
                "Proceeding with restore despite active SSP bootstraps (timed out)"
            );
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }

    // 6. Restore the main remote SurrealDB.
    let remote = connect_remote(db_config)
        .await
        .context("Failed to connect to main SurrealDB for restore")?;

    // Wipe: REMOVE DATABASE drops every table/record inside, DEFINE recreates.
    let wipe_sql = format!(
        "REMOVE DATABASE IF EXISTS {db}; DEFINE DATABASE {db}; USE DB {db};",
        db = db_config.database
    );
    remote
        .query(&wipe_sql)
        .await
        .context("Failed to wipe remote database before import")?;
    // Re-select the database on the connection after the REMOVE/DEFINE cycle.
    remote
        .use_db(&db_config.database)
        .await
        .context("Failed to re-select remote database after wipe")?;

    remote
        .import(dump_path)
        .await
        .context("Failed to import dump into main SurrealDB")?;
    progress.main_db_restored = true;
    info!(restore_id = %job.restore_id, "Main SurrealDB restored");

    // 7. Restore the snapshot replica: drop the on-disk DB, reopen empty, import.
    let restored_seq = {
        let mut rep = replica.write().await;
        rep.reset().await.context("Failed to reset replica")?;
        rep.import_from_file(dump_path)
            .await
            .context("Failed to import dump into replica")?;
        rep.reload_snapshot_seq()
            .await
            .context("Failed to reload snapshot_seq from restored replica")?
    };
    progress.replica_restored = true;
    info!(
        restore_id = %job.restore_id,
        restored_seq,
        "Replica restored from dump"
    );

    // 8. Clear pending items.
    let buffer_cleared = {
        let mut buffer = ingest.event_buffer.write().await;
        let n = buffer.len();
        buffer.clear();
        n
    };
    {
        let mut wal = ingest.wal.write().await;
        wal.truncate(u64::MAX)
            .context("Failed to truncate WAL during restore")?;
    }
    ingest.seq_counter.store(restored_seq, Ordering::SeqCst);
    {
        // Persist the restored seq explicitly so the metadata row matches the
        // authoritative counter even if the dump's seq differs subtly.
        let mut rep = replica.write().await;
        rep.set_snapshot_seq(restored_seq)
            .await
            .context("Failed to persist restored snapshot_seq")?;
    }

    // 9. Evict SSPs. They will re-register on next heartbeat.
    let evicted = {
        let mut pool = ssp_pool.write().await;
        pool.clear_all()
    };
    info!(
        restore_id = %job.restore_id,
        evicted,
        "SSPs evicted; will re-register against restored state"
    );

    Ok(RestoreOutcome {
        snapshot_seq: restored_seq,
        pending_cleared: buffer_cleared,
        main_db_restored: true,
        replica_restored: true,
        ssps_evicted: evicted,
    })
}

/// Open a fresh WebSocket connection to the main SurrealDB using the same
/// credentials the scheduler uses for the initial clone.
async fn connect_remote(
    db_config: &DbConfig,
) -> Result<surrealdb::Surreal<surrealdb::engine::remote::ws::Client>> {
    let ws_addr = db_config
        .url
        .strip_prefix("ws://")
        .or_else(|| db_config.url.strip_prefix("wss://"))
        .unwrap_or(&db_config.url);

    let db = surrealdb::Surreal::new::<surrealdb::engine::remote::ws::Ws>(ws_addr)
        .await
        .with_context(|| format!("Failed to open WS to {}", db_config.url))?;

    db.signin(surrealdb::opt::auth::Root {
        username: db_config.username.clone(),
        password: db_config.password.clone(),
    })
    .await
    .context("Remote SurrealDB signin failed")?;

    db.use_ns(&db_config.namespace)
        .use_db(&db_config.database)
        .await
        .context("Failed to select remote namespace/database")?;

    Ok(db)
}
