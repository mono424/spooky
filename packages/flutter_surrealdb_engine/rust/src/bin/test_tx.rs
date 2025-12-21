use rust_lib_flutter_surrealdb_engine::{connect_db, SurrealResult};

#[tokio::main]
async fn main() -> Result<(), String> {
    println!("--- Starting Rust Transaction Test (Library Integration) ---");

    // 1. Prepare DB
    // Remove if exists to start fresh (RocksDB)
    let _ = std::fs::remove_dir_all("test_tx_db");
    
    // Connect using library function
    let db = connect_db("test_tx_db".to_string()).await?;

    // Setup: Use DB to create a namespace/db (implied valid for local usually, but good to call)
    db.use_ns("test".to_string()).await?;
    db.use_db("test".to_string()).await?;

    // Clean state
    db.query_db("DELETE person;".to_string(), None).await?;

    // 2. Test Commit
    println!("2. Testing Commit...");
    {
        // Begin Transaction via Lib
        let tx = db.begin_transaction().await?;
        
        // Query inside TX via Lib
        tx.query("CREATE person:1 SET name = 'Alice';".to_string(), None).await?;
        
        // Verify inside (optional, but good)
        let res = tx.query("SELECT * FROM person:1;".to_string(), None).await?;
        // Result is Vec<SurrealResult>. Each result has .result which is Option<String> (JSON).
        let val_s = res.first().and_then(|r| r.result.as_ref()).map(|s| s.as_str()).unwrap_or("[]");
        println!("   Inside TX: {}", val_s);

        // Verify outside (should be empty/hidden)
        let res_outside = db.query_db("SELECT * FROM person:1;".to_string(), None).await?;
        let val_s = res_outside.first().and_then(|r| r.result.as_ref()).map(|s| s.as_str()).unwrap_or("[]");
        
        // "[]" or "null" or some generic empty json
        if val_s == "[]" || val_s == "null" {
             println!("   Data HIDDEN from outside. Isolation works!");
        } else {
             println!("   WARNING: Data visible outside BEFORE commit! Value: {}", val_s);
        }

        // Commit via Lib
        tx.commit().await?;
    }

    // Verify persistence
    let res = db.query_db("SELECT * FROM person:1;".to_string(), None).await?;
    let val_s = res.first().and_then(|r| r.result.as_ref()).map(|s| s.as_str()).unwrap_or("[]");
    
    // Check if it's the expected JSON
    if val_s == "[]" || val_s == "null" {
         println!("   Commit FAILED (empty). Value: {}", val_s);
    } else {
         println!("   Commit verified: {}", val_s);
    }

    // 3. Test Cancel
    println!("3. Testing Cancel...");
    {
        let tx = db.begin_transaction().await?;
        tx.query("CREATE person:2 SET name = 'Bob';".to_string(), None).await?;
        tx.cancel().await?;
    }

    // Verify absence
    let res = db.query_db("SELECT * FROM person:2;".to_string(), None).await?;
    let val_s = res.first().and_then(|r| r.result.as_ref()).map(|s| s.as_str()).unwrap_or("[]");
     if val_s == "[]" || val_s == "null" {
         println!("   Cancel verified (empty).");
     } else {
         println!("   Cancel FAILED (data exists: {}).", val_s);
     }

    println!("--- Test Completed ---");
    Ok(())
}
