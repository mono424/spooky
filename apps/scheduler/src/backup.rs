use anyhow::{Context, Result};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use flate2::write::GzEncoder;
use flate2::Compression;
use s3::bucket::Bucket;
use s3::creds::Credentials;
use s3::Region;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tracing::{error, info, warn};

use crate::ingest::{pending_events_snapshot, IngestState};
use crate::replica::Replica;

/// Max finished (Completed/Failed) jobs to retain in the registry.
const RECENT_JOB_LIMIT: usize = 50;
/// Bounded queue capacity for pending backup jobs.
const BACKUP_QUEUE_CAPACITY: usize = 64;

#[derive(Clone)]
pub struct BackupState {
    pub replica: Arc<RwLock<Replica>>,
    pub ingest: IngestState,
    pub config: Arc<BackupConfig>,
    pub registry: Arc<BackupRegistry>,
    pub tx: mpsc::Sender<BackupJob>,
}

#[derive(Debug, Clone)]
pub struct BackupConfig {
    pub s3_endpoint: String,
    pub s3_access_key: String,
    pub s3_secret_key: String,
    pub s3_bucket: String,
    pub s3_region: String,
}

impl BackupConfig {
    pub fn from_env() -> Self {
        Self {
            s3_endpoint: std::env::var("S3_ENDPOINT")
                .unwrap_or_else(|_| "http://10.100.1.5:9000".to_string()),
            s3_access_key: std::env::var("S3_ACCESS_KEY")
                .unwrap_or_else(|_| "minioadmin".to_string()),
            s3_secret_key: std::env::var("S3_SECRET_KEY")
                .unwrap_or_else(|_| "minioadmin".to_string()),
            s3_bucket: std::env::var("S3_BUCKET").unwrap_or_else(|_| "backups".to_string()),
            s3_region: std::env::var("S3_REGION").unwrap_or_else(|_| "us-east-1".to_string()),
        }
    }

    fn region(&self) -> Region {
        Region::Custom {
            region: self.s3_region.clone(),
            endpoint: self.s3_endpoint.clone(),
        }
    }

    fn credentials(&self) -> Result<Credentials> {
        Credentials::new(
            Some(&self.s3_access_key),
            Some(&self.s3_secret_key),
            None,
            None,
            None,
        )
        .context("Failed to build S3 credentials")
    }

    fn get_bucket(&self) -> Result<Box<Bucket>> {
        let bucket = Bucket::new(&self.s3_bucket, self.region(), self.credentials()?)?
            .with_path_style();
        Ok(bucket)
    }
}

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
        snapshot_seq: u64,
        storage_path: String,
    ) {
        self.update(id, |s| {
            s.status = BackupStatus::Completed;
            s.finished_at = Some(Utc::now());
            s.size_bytes = Some(size_bytes);
            s.snapshot_seq = Some(snapshot_seq);
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

pub fn create_backup_router(state: BackupState) -> Router {
    Router::new()
        .route("/backup/create", post(create_backup))
        .route("/backup/restore", post(restore_backup))
        .route("/backup/status", get(backup_status))
        .route("/backup/status/:backup_id", get(backup_status_by_id))
        .with_state(state)
}

#[derive(Deserialize)]
struct CreateBackupRequest {
    backup_id: String,
    project_slug: String,
}

#[derive(Serialize)]
struct CreateBackupResponse {
    backup_id: String,
    status: String,
    queue_position: usize,
}

async fn create_backup(
    State(state): State<BackupState>,
    Json(req): Json<CreateBackupRequest>,
) -> Result<(StatusCode, Json<CreateBackupResponse>), (StatusCode, String)> {
    if state.registry.contains(&req.backup_id).await {
        return Err((
            StatusCode::CONFLICT,
            format!("backup_id {} already exists", req.backup_id),
        ));
    }

    state
        .registry
        .enqueue(req.backup_id.clone(), req.project_slug.clone())
        .await;

    let job = BackupJob {
        backup_id: req.backup_id.clone(),
        project_slug: req.project_slug.clone(),
    };

    if let Err(e) = state.tx.send(job).await {
        state
            .registry
            .mark_failed(&req.backup_id, format!("queue send failed: {}", e))
            .await;
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "Backup queue is closed".to_string(),
        ));
    }

    let queue_position = state.registry.queue_len().await;
    info!(backup_id = %req.backup_id, queue_position, "Backup enqueued");

    Ok((
        StatusCode::ACCEPTED,
        Json(CreateBackupResponse {
            backup_id: req.backup_id,
            status: "queued".to_string(),
            queue_position,
        }),
    ))
}

#[derive(Deserialize)]
struct RestoreRequest {
    #[allow(dead_code)]
    backup_id: String,
    #[allow(dead_code)]
    project_slug: String,
    #[allow(dead_code)]
    storage_path: String,
}

async fn restore_backup(
    State(_state): State<BackupState>,
    Json(_req): Json<RestoreRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(serde_json::json!({
            "error": "restore via native import not yet implemented",
            "hint": "download the .surql.gz artifact from S3 and run `surreal import`",
        })),
    )
}

async fn backup_status(State(state): State<BackupState>) -> Json<serde_json::Value> {
    let current = state.registry.current_running().await;
    let queue_len = state.registry.queue_len().await;
    let recent = state.registry.recent().await;
    let s3_reachable = state.config.get_bucket().is_ok();
    let pending = pending_events_snapshot(&state.ingest).await;

    Json(serde_json::json!({
        "current_running": current,
        "queue_len": queue_len,
        "recent": recent,
        "s3_endpoint": state.config.s3_endpoint,
        "s3_bucket": state.config.s3_bucket,
        "s3_reachable": s3_reachable,
        "pending_events": pending.pending_events,
        "snapshot_seq": pending.snapshot_seq,
        "latest_seq": pending.latest_seq,
        "lag": pending.lag,
    }))
}

async fn backup_status_by_id(
    State(state): State<BackupState>,
    Path(backup_id): Path<String>,
) -> Result<Json<BackupJobState>, (StatusCode, String)> {
    match state.registry.get(&backup_id).await {
        Some(s) => Ok(Json(s)),
        None => Err((StatusCode::NOT_FOUND, format!("backup_id {} not found", backup_id))),
    }
}

/// Ensure the configured bucket exists. Idempotent; ignores errors for existing buckets.
async fn ensure_bucket(config: &BackupConfig) {
    let creds = match config.credentials() {
        Ok(c) => c,
        Err(e) => {
            warn!(error = %e, "Failed to build S3 credentials for bucket ensure");
            return;
        }
    };
    let _ = Bucket::create_with_path_style(
        &config.s3_bucket,
        config.region(),
        creds,
        s3::BucketConfiguration::default(),
    )
    .await;
}

/// Single-consumer worker: serially processes backup jobs from the queue.
pub async fn run_backup_worker(
    mut rx: mpsc::Receiver<BackupJob>,
    replica: Arc<RwLock<Replica>>,
    ingest: IngestState,
    config: Arc<BackupConfig>,
    registry: Arc<BackupRegistry>,
) {
    info!("Backup worker started");
    while let Some(job) = rx.recv().await {
        registry.mark_running(&job.backup_id).await;
        info!(backup_id = %job.backup_id, project = %job.project_slug, "Backup worker running job");

        match execute_backup(&job, &replica, &ingest, &config).await {
            Ok((size_bytes, snapshot_seq, storage_path)) => {
                registry
                    .mark_completed(&job.backup_id, size_bytes, snapshot_seq, storage_path.clone())
                    .await;
                info!(
                    backup_id = %job.backup_id,
                    size_bytes,
                    snapshot_seq,
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
    replica: &Arc<RwLock<Replica>>,
    ingest: &IngestState,
    config: &BackupConfig,
) -> Result<(u64, u64, String)> {
    // 1. Drain in-memory events into the replica so the backup captures the latest state.
    let applied = crate::drain_and_apply(&ingest.event_buffer, replica, &ingest.wal)
        .await
        .context("Failed to drain pending events into replica before backup")?;
    if applied > 0 {
        info!(backup_id = %job.backup_id, applied, "Drained pending events into replica");
    }

    // 2. Native SurrealDB export to a tempfile.
    let tmp = tempfile::NamedTempFile::new().context("Failed to create tempfile for export")?;
    let tmp_path = tmp.path().to_path_buf();

    let snapshot_seq = {
        let rep = replica.read().await;
        rep.export_to_file(&tmp_path).await.context("Native export failed")?;
        rep.snapshot_seq()
    };

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
