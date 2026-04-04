use anyhow::{Context, Result};
use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use flate2::write::GzEncoder;
use flate2::Compression;
use s3::bucket::Bucket;
use s3::creds::Credentials;
use s3::Region;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

use crate::replica::Replica;
use crate::SchedulerStatus;

/// Shared state for backup handlers
#[derive(Clone)]
pub struct BackupState {
    pub replica: Arc<RwLock<Replica>>,
    pub status: Arc<RwLock<SchedulerStatus>>,
    pub config: Arc<BackupConfig>,
}

#[derive(Debug, Clone)]
pub struct BackupConfig {
    pub minio_endpoint: String,
    pub minio_access_key: String,
    pub minio_secret_key: String,
    pub bucket_name: String,
}

impl BackupConfig {
    pub fn from_env() -> Self {
        Self {
            minio_endpoint: std::env::var("MINIO_ENDPOINT")
                .unwrap_or_else(|_| "http://10.100.1.5:9000".to_string()),
            minio_access_key: std::env::var("MINIO_ACCESS_KEY")
                .unwrap_or_else(|_| "minioadmin".to_string()),
            minio_secret_key: std::env::var("MINIO_SECRET_KEY")
                .unwrap_or_else(|_| "minioadmin".to_string()),
            bucket_name: std::env::var("BACKUP_BUCKET")
                .unwrap_or_else(|_| "backups".to_string()),
        }
    }

    fn get_bucket(&self) -> Result<Box<Bucket>> {
        let region = Region::Custom {
            region: "us-east-1".to_string(),
            endpoint: self.minio_endpoint.clone(),
        };
        let credentials = Credentials::new(
            Some(&self.minio_access_key),
            Some(&self.minio_secret_key),
            None,
            None,
            None,
        )?;
        let bucket = Bucket::new(&self.bucket_name, region, credentials)?
            .with_path_style();
        Ok(bucket)
    }
}

pub fn create_backup_router(state: BackupState) -> Router {
    Router::new()
        .route("/backup/create", post(create_backup))
        .route("/backup/restore", post(restore_backup))
        .route("/backup/status", get(backup_status))
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
    size_bytes: u64,
    snapshot_seq: u64,
    storage_path: String,
}

/// Create a backup by exporting the current snapshot to MinIO
async fn create_backup(
    State(state): State<BackupState>,
    Json(req): Json<CreateBackupRequest>,
) -> Result<Json<CreateBackupResponse>, (StatusCode, String)> {
    info!("Creating backup {} for project {}", req.backup_id, req.project_slug);

    // Freeze snapshot during export
    let prev_status = *state.status.read().await;
    *state.status.write().await = SchedulerStatus::SnapshotFrozen;

    let result = do_create_backup(&state, &req).await;

    // Restore previous status
    *state.status.write().await = prev_status;

    match result {
        Ok(resp) => Ok(Json(resp)),
        Err(e) => {
            error!("Backup failed: {}", e);
            Err((StatusCode::INTERNAL_SERVER_ERROR, format!("Backup failed: {}", e)))
        }
    }
}

async fn do_create_backup(
    state: &BackupState,
    req: &CreateBackupRequest,
) -> Result<CreateBackupResponse> {
    let replica = state.replica.read().await;
    let snapshot_seq = replica.snapshot_seq();

    // Export all data as chunks
    let chunks = replica.iter_chunks(1000).await
        .context("Failed to export snapshot chunks")?;

    info!(
        chunks = chunks.len(),
        snapshot_seq,
        "Exported snapshot data"
    );

    // Serialize to JSON and compress with gzip
    let json_data = serde_json::to_vec(&chunks)
        .context("Failed to serialize chunks")?;

    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&json_data)
        .context("Failed to compress")?;
    let compressed = encoder.finish()
        .context("Failed to finish compression")?;

    let size_bytes = compressed.len() as u64;
    let storage_path = format!("{}/{}.json.gz", req.project_slug, req.backup_id);

    info!(
        size_bytes,
        storage_path = %storage_path,
        "Uploading backup to MinIO"
    );

    // Upload to MinIO
    let bucket = state.config.get_bucket()
        .context("Failed to connect to MinIO")?;

    // Ensure bucket exists by trying to create it (idempotent)
    let region = Region::Custom {
        region: "us-east-1".to_string(),
        endpoint: state.config.minio_endpoint.clone(),
    };
    let creds = Credentials::new(
        Some(&state.config.minio_access_key),
        Some(&state.config.minio_secret_key),
        None, None, None,
    ).context("Failed to create credentials")?;
    let _ = Bucket::create_with_path_style(
        &state.config.bucket_name, region, creds, s3::BucketConfiguration::default(),
    ).await;

    let resp_code = bucket.put_object(&storage_path, &compressed).await
        .context("Failed to upload to MinIO")?;

    if resp_code.status_code() != 200 {
        anyhow::bail!("MinIO upload returned status {}", resp_code.status_code());
    }

    info!(
        backup_id = %req.backup_id,
        size_bytes,
        snapshot_seq,
        "Backup uploaded successfully"
    );

    Ok(CreateBackupResponse {
        backup_id: req.backup_id.clone(),
        status: "completed".to_string(),
        size_bytes,
        snapshot_seq,
        storage_path,
    })
}

#[derive(Deserialize)]
struct RestoreRequest {
    backup_id: String,
    project_slug: String,
    storage_path: String,
}

#[derive(Serialize)]
struct RestoreResponse {
    status: String,
    records_restored: usize,
}

/// Restore database from a backup stored in MinIO
async fn restore_backup(
    State(state): State<BackupState>,
    Json(req): Json<RestoreRequest>,
) -> Result<Json<RestoreResponse>, (StatusCode, String)> {
    info!("Restoring backup {} for project {}", req.backup_id, req.project_slug);

    match do_restore_backup(&state, &req).await {
        Ok(resp) => Ok(Json(resp)),
        Err(e) => {
            error!("Restore failed: {}", e);
            Err((StatusCode::INTERNAL_SERVER_ERROR, format!("Restore failed: {}", e)))
        }
    }
}

async fn do_restore_backup(
    state: &BackupState,
    req: &RestoreRequest,
) -> Result<RestoreResponse> {
    let bucket = state.config.get_bucket()
        .context("Failed to connect to MinIO")?;

    // Download backup
    let resp = bucket.get_object(&req.storage_path).await
        .context("Failed to download backup from MinIO")?;

    if resp.status_code() != 200 {
        anyhow::bail!("MinIO download returned status {}", resp.status_code());
    }

    // Decompress
    let compressed_data = resp.bytes().to_vec();
    let mut decoder = flate2::read::GzDecoder::new(compressed_data.as_slice());
    let mut json_data = Vec::new();
    std::io::Read::read_to_end(&mut decoder, &mut json_data)
        .context("Failed to decompress backup")?;

    // Deserialize chunks
    let chunks: Vec<crate::replica::ReplicaChunk> = serde_json::from_slice(&json_data)
        .context("Failed to parse backup data")?;

    let total_records: usize = chunks.iter().map(|c| c.records.len()).sum();
    info!(chunks = chunks.len(), records = total_records, "Downloaded and parsed backup");

    // Import into replica (which will sync to remote SurrealDB)
    let mut replica = state.replica.write().await;

    // Clear existing data first
    let tables: Vec<String> = chunks.iter().map(|c| c.table.clone()).collect::<std::collections::HashSet<_>>().into_iter().collect();
    for table in &tables {
        if let Err(e) = replica.query(&format!("DELETE {};", table)).await {
            warn!("Failed to clear table {}: {}", table, e);
        }
    }

    // Import all records
    let mut restored = 0;
    for chunk in &chunks {
        for (id, record) in &chunk.records {
            let thing_id = if id.contains(':') {
                id.clone()
            } else {
                format!("{}:{}", chunk.table, id)
            };
            if let Err(e) = replica.query(&format!(
                "CREATE {} CONTENT {};",
                thing_id,
                serde_json::to_string(record).unwrap_or_default()
            )).await {
                warn!("Failed to restore record {}: {}", thing_id, e);
            } else {
                restored += 1;
            }
        }
    }

    info!(restored, "Restore complete");

    Ok(RestoreResponse {
        status: "completed".to_string(),
        records_restored: restored,
    })
}

/// Get backup system status
async fn backup_status(
    State(state): State<BackupState>,
) -> Json<serde_json::Value> {
    let bucket_ok = state.config.get_bucket()
        .map(|_| true)
        .unwrap_or(false);

    Json(serde_json::json!({
        "minio_endpoint": state.config.minio_endpoint,
        "bucket": state.config.bucket_name,
        "minio_reachable": bucket_ok,
    }))
}
