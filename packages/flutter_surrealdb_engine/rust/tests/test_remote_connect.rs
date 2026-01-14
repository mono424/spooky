use rust_lib_surrealdb::api::client::{SurrealDb, StorageMode};

#[tokio::test]
async fn test_connect_remote() {
    // Attempt to connect to local SurrealDB instance
    // Note: This requires a running SurrealDB server at localhost:8000
    // docker run --rm -p 8000:8000 surrealdb/surrealdb:latest start --user root --pass root
    
    let url = "ws://127.0.0.1:8000".to_string();
    
    println!("Connecting to remote SurrealDB at {}", url);
    
    let db = SurrealDb::connect(StorageMode::Remote { url }).await.expect("Should connect to remote DB");
    
    // Authenticate
    // Assuming default root/root credentials for local dev
    let token = db.signin("{\"user\":\"root\",\"pass\":\"root\"}".to_string()).await.expect("Signin failed");
    assert!(!token.is_empty(), "Signin should return a token");
    
    // Select Namespace and Database
    db.use_db("test".to_string(), "test".to_string()).await.expect("use_db failed");
    
    // Execute a simple query
    // Query returns a JSON string, we expect it to be a valid JSON array
    let result = db.query("INFO FOR DB;".to_string(), None).await.expect("Query failed");
    println!("Query result: {}", result);
    
    assert!(!result.contains("ERR"), "Query should not return error");
}
