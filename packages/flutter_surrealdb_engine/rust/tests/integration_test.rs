use rust_lib_surrealdb::api::client::{SurrealDb, StorageMode};
use serde_json::Value;

#[tokio::test]
async fn test_unified_architecture() -> anyhow::Result<()> {
    // 1. Initialize Memory DB
    let db = SurrealDb::connect(StorageMode::Memory).await?;
    db.use_db("test_ns".to_string(), "test_db".to_string()).await?;

    // 2. Create Record
    let created = db.create("user".to_string(), Some(r#"{"name": "Rustacean"}"#.to_string())).await?;
    // create returns "null" or "{}"? crud.rs returns "{}"
    // But internally it uses RETURN NONE.
    // Wait, crud.rs: Ok("{}".to_string())
    assert_eq!(created, "{}");

    // 3. Select via unified method
    // select() calls queries::query which returns [[{...}]] structure stringified
    let selected = db.select("user".to_string()).await?;
    let json: Vec<Vec<Value>> = serde_json::from_str(&selected)?;
    
    // Check structure
    let users = &json[0];
    assert!(!users.is_empty());
    assert_eq!(users[0]["name"], "Rustacean");

    // 4. Update
    db.update("user".to_string(), Some(r#"{"name": "Ferris"}"#.to_string())).await?;
    
    // 5. Query verification
    let queried = db.query("SELECT * FROM user".to_string(), None).await?;
    let query_res: Vec<Vec<Value>> = serde_json::from_str(&queried)?;
    assert_eq!(query_res[0][0]["name"], "Ferris");

    // 6. Delete
    db.delete("user".to_string()).await?;
    let empty = db.select("user".to_string()).await?;
    let empty_json: Vec<Vec<Value>> = serde_json::from_str(&empty)?;
    assert!(empty_json[0].is_empty());

    Ok(())
}
