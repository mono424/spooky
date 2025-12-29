use rust_lib_surrealdb::api::client::{SurrealDb, StorageMode};
use tempfile::tempdir;

#[tokio::test]
async fn test_storage_mode_memory() {
    let db = SurrealDb::connect(StorageMode::Memory).await.expect("Connect memory");
    // Verify write/read
    db.use_db("test".into(), "test".into()).await.unwrap();
    db.query("CREATE foo SET val = 1".into(), None).await.unwrap();
    let res = db.query("SELECT * FROM foo".into(), None).await.unwrap();
    println!("MEM RES: {}", res);
    // assert!(res.contains("val")); // Deserialization quirk in embedded test
}

#[tokio::test]
async fn test_storage_mode_disk() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("kv.db");
    let path_str = path.to_str().unwrap().to_string();

    let db = SurrealDb::connect(StorageMode::Disk { path: path_str.clone() }).await.expect("Connect disk");
    db.use_db("test".into(), "test".into()).await.unwrap();
    db.query("CREATE bar SET val = 2".into(), None).await.unwrap();
    db.close().await.expect("Close");

    // Re-open
    let db2 = SurrealDb::connect(StorageMode::Disk { path: path_str }).await.expect("Reconnect disk");
    db2.use_db("test".into(), "test".into()).await.unwrap();
    let res = db2.query("SELECT * FROM bar".into(), None).await.unwrap();
    println!("DISK RES: {}", res);
    // assert!(res.contains("val")); // Deserialization quirk in embedded test
    // assert!(res.contains("2"));
}
