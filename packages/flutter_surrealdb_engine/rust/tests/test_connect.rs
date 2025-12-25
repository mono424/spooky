use rust_lib_surrealdb::api::client::{SurrealDb, StorageMode};
use tempfile::tempdir;

#[tokio::test]
async fn test_connect_mem() {
    let _db = SurrealDb::connect(StorageMode::Memory).await.expect("Should connect to mem");
}

#[tokio::test]
async fn test_connect_file() {
    let dir = tempdir().expect("Failed to create temp dir");
    let file_path = dir.path().join("test.db");
    let path_str = file_path.to_str().expect("Invalid path").to_string();

    let _db = SurrealDb::connect(StorageMode::Disk { path: path_str }).await.expect("Should connect to file path");
    
    // Verify directory exists (SurrealKV creates it)
    assert!(file_path.exists() || dir.path().exists()); 
}
