use rust_lib_flutter_surrealdb_engine::{connect_db, SurrealResult};

#[tokio::main]
async fn main() -> Result<(), String> {
    println!("--- Starting Rust Transaction Test (Library Integration) ---");

    // 1. Prepare DB
    // Remove if exists to start fresh (RocksDB)
    //let _ = std::fs::remove_dir_all("test_tx_db");
    
    // Connect using library function
    let db = connect_db("/Users/timohty/projekts/spooky/packages/flutter_core/example/db".to_string()).await?;

    // Setup: Use DB to create a namespace/db (implied valid for local usually, but good to call)
    db.use_ns("spooky_dev".to_string()).await?;
    db.use_db("spooky_db".to_string()).await?;
    let result = db.query_db("SELECT hash, created_at FROM _spooky_schema ORDER BY created_at DESC LIMIT 1;".to_string(), None).await?;
    println!("Result: {:?}", result);

    Ok(())
}
