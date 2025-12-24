use rust_lib_flutter_surrealdb_engine::{connect_db, SurrealResult};

#[tokio::main]
async fn main() {
    let path = "/Users/timohty/projekts/spooky/packages/flutter_core/example/db";
    println!("Connecting to {}", path);
    
    match connect_db(path.to_string()).await {
        Ok(db) => {
            println!("Connected successfully");
            
            println!("Defining Root User...");
            match db.setup_root_user("root".to_string(), "root".to_string()).await {
                Ok(_) => println!("Root user created"),
                Err(e) => println!("Error creating root user: {}", e),
            }

            println!("Defining Namespace and Database explicitely...");
            match db.query_db("DEFINE NAMESPACE mein_projek; DEFINE DATABASE mein_projek; USE NS mein_projek DB mein_projek; CREATE test_table SET name='persistence_check'".to_string(), None).await {
                Ok(_) => println!("Definitions (NS, DB) and test record executed"),
                Err(e) => println!("Error defining ns/db: {}", e),
            }

            println!("Selecting Namespace...");
            match db.use_ns("mein_projek".to_string()).await {
                Ok(_) => println!("Namespace selected"),
                Err(e) => println!("Error selecting namespace: {}", e),
            }

            println!("Selecting Database...");
            match db.use_db("mein_projek".to_string()).await {
                Ok(_) => println!("Database selected"),
                Err(e) => println!("Error selecting database: {}", e),
            }

            println!("Checking if created (INFO FOR DB)...");
            match db.query_db("INFO FOR DB".to_string(), None).await {
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
