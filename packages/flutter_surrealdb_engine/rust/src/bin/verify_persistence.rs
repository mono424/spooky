use rust_lib_flutter_surrealdb_engine::{connect_db, SurrealResult};

#[tokio::main]
async fn main() {
    let path = "/Users/timohty/projekts/spooky/packages/flutter_core/example/db";
    println!("Connecting to {} for verification", path);
    
    match connect_db(path.to_string()).await {
        Ok(db) => {
            println!("Connected successfully");
            
            println!("Selecting Namespace 'mein_projek'...");
            if let Err(e) = db.use_ns("mein_projek".to_string()).await {
                  println!("Error selecting namespace: {}", e);
                  return;
            }

            println!("Selecting Database 'mein_projek'...");
            if let Err(e) = db.use_db("mein_projek".to_string()).await {
                  println!("Error selecting database: {}", e);
                  return;
            }

            println!("Verifying persistence (SELECT * FROM test_table)...");
            match db.query_db("SELECT * FROM test_table".to_string(), None).await {
                Ok(results) => {
                    println!("Query successful. Results:");
                    for res in results {
                        println!("Status: {}, Result: {:?}", res.status, res.result);
                    }
                },
                Err(e) => println!("Error querying database: {}", e),
            }
        },
        Err(e) => println!("Error connecting: {}", e),
    }
}
