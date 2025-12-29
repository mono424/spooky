use rust_lib_surrealdb::api::client::{SurrealDb, StorageMode};
use tempfile::tempdir;
use std::time::Duration;

#[tokio::test]
async fn test_dev_sidecar_connect_and_query() {
    let dir = tempdir().expect("Failed to create temp dir");
    let file_path = dir.path().join("sidecar.db");
    let path_str = file_path.to_str().expect("Invalid path").to_string();
    let port = 15555; // Use a high port to avoid collisions

    println!("Starting DevSidecar on port {} at path {}", port, path_str);

    // 1. Connect (this should spawn the server)
    let db = SurrealDb::connect(StorageMode::DevSidecar { 
        path: path_str.clone(), 
        port 
    }).await.expect("Failed to connect to DevSidecar");

    // 2. Use DB (Root auth is auto-handled)
    db.use_db("test_ns".to_string(), "test_db".to_string()).await.expect("Failed to use db");

    // 3. Create data
    // Note: client.create uses RETURN NONE, so it returns "{}". 
    // We verify data via SELECT below.
    let created = db.create("person".to_string(), Some("{\"name\": \"Sidecar\"}".to_string())).await.expect("Failed to create");
    println!("Created (Ignored): {}", created);
    // assert!(created.contains("Sidecar")); // REMOVED: create returns "{}"

    // 4. Query data
    let result = db.query("SELECT * FROM person".to_string(), None).await.expect("Failed to query");
    println!("Query Result: {}", result);
    assert!(result.contains("Sidecar"));

    // 5. Close (should kill process)
    db.close().await.expect("Failed to close");

    // Short sleep to ensure process cleanup logic runs (async drop/kill is fast but OS might lag)
    tokio::time::sleep(Duration::from_millis(500)).await;

    // NOTE: We can't easily assert the process is dead cross-platform without extra deps (sysinfo),
    // but the successful connection and query proves the sidecar started and worked.
}
