use super::MaybeSendSync;

#[derive(Debug, Clone)]
pub struct ArtifactMeta {
    pub key: String,
    pub size_bytes: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum ArtifactError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("transport: {0}")]
    Transport(String),
}

/// Blob storage for backup artifacts.
///
/// DEFERRED port: defined now so `packages/maintenance` can be re-plumbed
/// onto it later without another boundary redesign. Current state: the VM
/// maintenance plane keeps its tempfile + rust-s3 implementation; the CF free
/// tier punts backups to the database provider (SurrealDB Cloud). An R2
/// adapter arrives with `apps/ssp-cf` if/when CF-side backups are needed.
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
pub trait ArtifactStore: MaybeSendSync {
    async fn put(&self, key: &str, bytes: Vec<u8>) -> Result<(), ArtifactError>;
    async fn get(&self, key: &str) -> Result<Vec<u8>, ArtifactError>;
    async fn list(&self, prefix: &str) -> Result<Vec<ArtifactMeta>, ArtifactError>;
    async fn delete(&self, key: &str) -> Result<(), ArtifactError>;
}
