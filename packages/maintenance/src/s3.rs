use anyhow::{Context, Result};
use s3::bucket::Bucket;
use s3::creds::Credentials;
use s3::Region;
use tracing::warn;

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

    /// True when any S3_* variable is present in the environment — used by
    /// hosts that only enable the backup plane when storage is configured.
    pub fn env_configured() -> bool {
        ["S3_ENDPOINT", "S3_ACCESS_KEY", "S3_SECRET_KEY", "S3_BUCKET"]
            .iter()
            .any(|k| std::env::var(k).is_ok())
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

    pub fn get_bucket(&self) -> Result<Box<Bucket>> {
        let bucket = Bucket::new(&self.s3_bucket, self.region(), self.credentials()?)?
            .with_path_style();
        Ok(bucket)
    }
}

/// Ensure the configured bucket exists. Idempotent; ignores errors for existing buckets.
pub async fn ensure_bucket(config: &BackupConfig) {
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
