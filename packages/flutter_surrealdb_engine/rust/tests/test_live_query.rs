use rust_lib_surrealdb::api::client::SurrealDb; // Assumes usage of lib
use surrealdb::engine::any::connect;
use surrealdb::{Notification, Action};
use serde_json::{json, Value};
use tokio::time::{sleep, Duration};
use futures_util::StreamExt;

// Integration test for Live Query
// Note: This test currently fails on SurrealDB v3.0.0-alpha.17 embedded backend (mem://)
// with "Internal error: Expected any, got record".
// It is preserved here for future validation when upstream is fixed.
#[tokio::test]
async fn test_live_query_flow() -> anyhow::Result<()> {
    let db = connect("mem://").await?;
    db.use_ns("test").use_db("test").await?;

    // We can't easily access SurrealDb wrapper internals here without mocking StreamSink.
    // So we test the raw SurrealDB logic similar to the unit test.
    
    let db_clone = db.clone();
    let (tx, mut rx) = tokio::sync::mpsc::channel(10);

    tokio::spawn(async move {
        // This line fails in alpha versions for mem://
        let mut stream = db_clone.select("person").live().await.unwrap();
        
        while let Some(Ok(notification)) = stream.next().await {
            // Force type inference
            let notification: Notification<Value> = notification;
            
            // Match Action enum directly as per user optimization
            let action_str = match notification.action {
                Action::Create => "CREATE",
                Action::Update => "UPDATE",
                Action::Delete => "DELETE",
                _ => "UNKNOWN", 
            };
            
            // Send simplified event
            if tx.send((action_str, notification.data)).await.is_err() {
                break;
            }
        }
    });

    sleep(Duration::from_millis(50)).await;

    // Trigger Create
    let _: Option<Value> = db.create("person").content(json!({"name": "PING"})).await?;
    
    // Expect PING
    let (action, data) = rx.recv().await.expect("Failed to receive PING");
    assert_eq!(action, "CREATE");
    assert_eq!(data["name"], "PING");

    Ok(())
}
