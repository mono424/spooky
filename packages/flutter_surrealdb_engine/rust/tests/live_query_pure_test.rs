use rust_lib_surrealdb::api::client::{SurrealDb, StorageMode};
use futures_util::StreamExt;
use std::time::Duration;
use tokio::time::timeout;

#[tokio::test]
async fn test_pure_rust_live_query() -> anyhow::Result<()> {
    // 1. Initialize
    let db = SurrealDb::connect(StorageMode::Memory).await?;
    db.use_db("test_ns".to_string(), "test_db".to_string()).await?;
    
    let table = "user_test".to_string();
    
    // 2. Start Live Query (using internal logic mimic)
    // We need to access inner client. SurrealDb has `get_client`.
    // But it is crate-private.
    // So we use public API if possible or just use `surrealdb` directly here?
    // We want to test `Surreal<Any>` wrapper.
    
    // We can't access `get_client` from integration test (external crate).
    // We will use `surrealdb` directly to reproduce the ENGINE behavior.
    
    use surrealdb::engine::any::connect;
    use surrealdb::Surreal;
    
    let db_direct = connect("mem://").await?;
    db_direct.use_ns("test_ns").use_db("test_db").await?;
    
    // Define Table
    db_direct.query("DEFINE TABLE user_test SCHEMALESS").await?;
    
    // Clone for spawner
    let db_clone = db_direct.clone();
    let table_clone = table.clone();
    
    // Channel for signal
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    
    tokio::spawn(async move {
        println!("RUST TEST: subscribing...");
        let mut stream = db_clone.select(&table_clone).live().await.expect("subscribed");
        println!("RUST TEST: subscribed. waiting for events...");
        
        while let Some(msg) = stream.next().await {
            println!("RUST TEST: event received!");
            // Hint the type
            // Try generic Record first, relying on inference for inner type, or assume dynamic?
            // Record is usually Record<T>.
            // Let's try to see if Record is available.
            let notification: surrealdb::Result<surrealdb::Notification<serde_json::Value>> = msg;
            tx.send(notification).await.unwrap();
        }
    });
    
    // Allow subscription to propagate (important for mem channels?)
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    // 3. Create Record
    println!("RUST TEST: creating record...");
    let _: Option<serde_json::Value> = db_direct.create(table.clone()).content(serde_json::json!({"name": "PureRust"})).await?;
    println!("RUST TEST: record created.");
    
    // 4. Wait for event
    println!("RUST TEST: waiting for event...");
    match timeout(Duration::from_secs(2), rx.recv()).await {
        Ok(Some(msg)) => {
            println!("RUST TEST: Success! {:?}", msg);
        },
        Ok(None) => panic!("Stream closed"),
        Err(_) => panic!("Timeout waiting for event"),
    }
    
    Ok(())
}
