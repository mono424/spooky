use rust_lib_surrealdb::api::client::{SurrealDb, StorageMode};

#[tokio::test]
async fn test_surrealdb_wrapper_flow() -> anyhow::Result<()> {
    // 1. Connect (Memory)
    let db = SurrealDb::connect(StorageMode::Memory).await?;
    
    // 2. Use NS/DB
    db.use_db("test".to_string(), "test".to_string()).await?;
    
    // 3. Create
    let created = db.create("person".to_string(), Some(r#"{"name": "Tester"}"#.to_string())).await?;
    assert!(!created.is_empty());
    
    // 4. Query
    let result = db.query("SELECT * FROM person".to_string(), Option::<String>::None).await?;
    println!("DEBUG RESULT: {}", result);
    assert!(result.contains("Tester"));
    
    // 5. Check helper methods
    let clients = db.select("person".to_string()).await?;
    assert!(clients.contains("Tester"));

    Ok(())
}
