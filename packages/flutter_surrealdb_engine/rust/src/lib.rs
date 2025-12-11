mod frb_generated; /* AUTO INJECTED BY flutter_rust_bridge. This line may not be accurate, and you can change it according to your needs. */
use surrealdb::Surreal;
use surrealdb::engine::local::RocksDb;
use std::sync::OnceLock;

static DB: OnceLock<Surreal<surrealdb::engine::local::Db>> = OnceLock::new();

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Serialize, Deserialize)]
pub struct SurrealResult {
    pub result: Option<String>,
    pub status: String,
    pub time: String,
}

pub async fn connect_db(path: String) -> Result<(), String> {
    if DB.get().is_some() {
        return Ok(());
    }

    let db = match Surreal::new::<RocksDb>(path).await {
        Ok(db) => db,
        Err(e) => return Err(format!("Error connecting to database: {}", e)),
    };

    match db.use_ns("mein_namespace").use_db("meine_db").await {
        Ok(_) => {
            let _ = DB.set(db);
            Ok(())
        }
        Err(e) => Err(format!("Error selecting namespace/db: {}", e)),
    }
}

pub async fn query_db(query: String) -> Result<Vec<SurrealResult>, String> {
    let db = match DB.get() {
        Some(db) => db,
        None => return Err("Database not initialized".to_string()),
    };

    match db.query(query).await {
        Ok(mut response) => {
            let mut results = Vec::new();
            // SurrealDB responses can contain multiple results if multiple queries were executed.
            // We iterate through them.
            // Note: The exact API to iterate generic results might depend on the version.
            // Assuming we can take results one by one or iterate.
            // For now, let's try to collect all results.
            
            // Since we don't know how many statements, we can try to take until error or empty?
            // Actually, response.take(index) is the way.
            // But we need to know how many.
            // response.num_statements() is available in some versions.
            
            // Let's assume we just want to return the whole response as a list of results.
            // We can try to serialize the whole response to Value?
            // Or iterate.
            
            // Let's try a loop.
            let num_statements = response.num_statements();
            for i in 0..num_statements {
                 // Try to take the result as a single Value (which might be an Array)
                 let result: Result<surrealdb::Value, _> = response.take(i);
                 match result {
                     Ok(val) => {
                         // Convert the value to serde_json::Value then to String
                         let json_val = serde_json::to_value(&val).unwrap_or(Value::Null);
                         let json_str = serde_json::to_string(&json_val).ok();
                         results.push(SurrealResult {
                             result: json_str,
                             status: "OK".to_string(),
                             time: "0ms".to_string(),
                         });
                     },
                     Err(e) => {
                         // If taking as Vec fails, maybe it's not a query that returns rows?
                         // But generic take should work.
                         results.push(SurrealResult {
                             result: None,
                             status: format!("Error: {}", e),
                             time: "0ms".to_string(),
                         });
                     }
                 }
            }
            Ok(results)
        }
        Err(e) => Err(format!("System Error: {}", e)),
    }
}

#[flutter_rust_bridge::frb(init)]
pub fn init_app() {
    // Default utilities - feel free to customize
    flutter_rust_bridge::setup_default_user_utils();
}
