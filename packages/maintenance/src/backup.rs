use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use flate2::write::GzEncoder;
use flate2::Compression;
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex, RwLock};
use tracing::{error, info};

use crate::db::{connect_http, DbConfig};
use crate::host::MaintenanceHost;
use crate::s3::{ensure_bucket, BackupConfig};

/// Max finished (Completed/Failed) jobs to retain in the registry.
const RECENT_JOB_LIMIT: usize = 50;
/// Bounded queue capacity for pending backup jobs.
const BACKUP_QUEUE_CAPACITY: usize = 64;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BackupStatus {
    Queued,
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct BackupJobState {
    pub backup_id: String,
    pub project_slug: String,
    pub status: BackupStatus,
    pub enqueued_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub size_bytes: Option<u64>,
    pub snapshot_seq: Option<u64>,
    pub storage_path: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct BackupJob {
    pub backup_id: String,
    pub project_slug: String,
}

/// Registry of all backup jobs (queued, running, recent finished).
pub struct BackupRegistry {
    jobs: RwLock<HashMap<String, BackupJobState>>,
    order: RwLock<VecDeque<String>>,
}

impl BackupRegistry {
    pub fn new() -> Self {
        Self {
            jobs: RwLock::new(HashMap::new()),
            order: RwLock::new(VecDeque::new()),
        }
    }

    pub async fn contains(&self, id: &str) -> bool {
        self.jobs.read().await.contains_key(id)
    }

    pub async fn enqueue(&self, backup_id: String, project_slug: String) -> BackupJobState {
        let state = BackupJobState {
            backup_id: backup_id.clone(),
            project_slug,
            status: BackupStatus::Queued,
            enqueued_at: Utc::now(),
            started_at: None,
            finished_at: None,
            size_bytes: None,
            snapshot_seq: None,
            storage_path: None,
            error: None,
        };
        self.jobs.write().await.insert(backup_id.clone(), state.clone());
        self.order.write().await.push_back(backup_id);
        state
    }

    async fn update<F: FnOnce(&mut BackupJobState)>(&self, id: &str, f: F) {
        if let Some(state) = self.jobs.write().await.get_mut(id) {
            f(state);
        }
    }

    pub async fn mark_running(&self, id: &str) {
        self.update(id, |s| {
            s.status = BackupStatus::Running;
            s.started_at = Some(Utc::now());
        })
        .await;
    }

    pub async fn mark_completed(
        &self,
        id: &str,
        size_bytes: u64,
        snapshot_seq: Option<u64>,
        storage_path: String,
    ) {
        self.update(id, |s| {
            s.status = BackupStatus::Completed;
            s.finished_at = Some(Utc::now());
            s.size_bytes = Some(size_bytes);
            s.snapshot_seq = snapshot_seq;
            s.storage_path = Some(storage_path);
        })
        .await;
        self.trim().await;
    }

    pub async fn mark_failed(&self, id: &str, err: String) {
        self.update(id, |s| {
            s.status = BackupStatus::Failed;
            s.finished_at = Some(Utc::now());
            s.error = Some(err);
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
                    .map(|s| matches!(s.status, BackupStatus::Completed | BackupStatus::Failed))
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

    pub async fn get(&self, id: &str) -> Option<BackupJobState> {
        self.jobs.read().await.get(id).cloned()
    }

    pub async fn current_running(&self) -> Option<BackupJobState> {
        self.jobs
            .read()
            .await
            .values()
            .find(|s| matches!(s.status, BackupStatus::Running))
            .cloned()
    }

    pub async fn queue_len(&self) -> usize {
        self.jobs
            .read()
            .await
            .values()
            .filter(|s| matches!(s.status, BackupStatus::Queued))
            .count()
    }

    pub async fn recent(&self) -> Vec<BackupJobState> {
        let jobs = self.jobs.read().await;
        let order = self.order.read().await;
        order
            .iter()
            .filter_map(|id| jobs.get(id).cloned())
            .collect()
    }
}

impl Default for BackupRegistry {
    fn default() -> Self {
        Self::new()
    }
}

pub fn create_backup_channel() -> (mpsc::Sender<BackupJob>, mpsc::Receiver<BackupJob>) {
    mpsc::channel(BACKUP_QUEUE_CAPACITY)
}

/// Single-consumer worker: serially processes backup jobs from the queue.
pub async fn run_backup_worker(
    mut rx: mpsc::Receiver<BackupJob>,
    host: Arc<dyn MaintenanceHost>,
    config: Arc<BackupConfig>,
    db_config: Arc<DbConfig>,
    registry: Arc<BackupRegistry>,
    lock: Arc<Mutex<()>>,
) {
    info!("Backup worker started");
    while let Some(job) = rx.recv().await {
        registry.mark_running(&job.backup_id).await;
        info!(backup_id = %job.backup_id, project = %job.project_slug, "Backup worker running job");

        // Serialize with the restore worker — never back up mid-restore.
        let _guard = lock.lock().await;

        match execute_backup(&job, host.as_ref(), &config, &db_config).await {
            Ok((size_bytes, snapshot_seq, storage_path)) => {
                registry
                    .mark_completed(&job.backup_id, size_bytes, snapshot_seq, storage_path.clone())
                    .await;
                info!(
                    backup_id = %job.backup_id,
                    size_bytes,
                    snapshot_seq = ?snapshot_seq,
                    storage_path = %storage_path,
                    "Backup completed"
                );
            }
            Err(e) => {
                let err_str = format!("{:#}", e);
                registry.mark_failed(&job.backup_id, err_str.clone()).await;
                error!(backup_id = %job.backup_id, error = %err_str, "Backup failed");
            }
        }
    }
    info!("Backup worker exiting (channel closed)");
}

async fn execute_backup(
    job: &BackupJob,
    host: &dyn MaintenanceHost,
    config: &BackupConfig,
    db_config: &DbConfig,
) -> Result<(u64, Option<u64>, String)> {
    // 1. Host-specific preparation. The scheduler drains in-memory events into
    //    its replica (keeping snapshot_seq current for SSP bootstrap) and
    //    reports the seq; a standalone SSP has nothing to prepare.
    let snapshot_seq = host
        .pre_backup()
        .await
        .context("Host pre-backup step failed")?;

    // 2. Export the MAIN SurrealDB directly over HTTP.
    //
    //    Exporting main is authoritative — whatever the user sees in SurrealDB
    //    is what lands in the backup.
    //
    //    Note: SurrealDB's native export does NOT include bucket file contents;
    //    only the `DEFINE BUCKET` statements. Backing up bucket-backed files
    //    requires a separate copy step from the bucket's backing store.
    let tmp = tempfile::NamedTempFile::new().context("Failed to create tempfile for export")?;
    let tmp_path = tmp.path().to_path_buf();

    let remote = connect_http(db_config)
        .await
        .context("Failed to connect to main SurrealDB for backup export")?;
    remote
        .export(&tmp_path)
        .await
        .context("Failed to export main SurrealDB")?;

    // 3. Read & gzip.
    let raw = std::fs::read(&tmp_path).context("Failed to read exported file")?;
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&raw).context("Failed to gzip export")?;
    let compressed = encoder.finish().context("Failed to finalize gzip")?;
    let size_bytes = compressed.len() as u64;

    // 4. Upload to S3.
    ensure_bucket(config).await;
    let bucket = config.get_bucket().context("Failed to build S3 bucket client")?;
    let storage_path = format!("{}/{}.surql.gz", job.project_slug, job.backup_id);
    let resp = bucket
        .put_object(&storage_path, &compressed)
        .await
        .context("Failed to upload backup to S3")?;
    if resp.status_code() != 200 {
        anyhow::bail!("S3 upload returned status {}", resp.status_code());
    }

    Ok((size_bytes, snapshot_seq, storage_path))
}
