mod frb_generated; /* AUTO INJECTED BY flutter_rust_bridge. This line may not be accurate, and you can change it according to your needs. */

use surrealdb::Surreal;
use surrealdb::engine::local::{Db, RocksDb};

use serde::{Deserialize, Serialize};


#[derive(Serialize, Deserialize)]
pub struct SurrealResult {
    pub result: Option<String>,
    pub status: String,
    pub time: String,
}

#[derive(Clone)]
pub struct SurrealDatabase {
    pub db: Surreal<Db>,
}


pub async fn connect_db(path: String) -> Result<SurrealDatabase, String> {
    let db = match Surreal::new::<RocksDb>(path).await {
        Ok(db) => db,
        Err(e) => return Err(format!("Error connecting to database: {}", e)),
    };

    Ok(SurrealDatabase { db })
}



use surrealdb::opt::auth::{Root, Namespace, Database};





fn create_result(result: Option<String>, status: String, start: std::time::Instant) -> SurrealResult {
    SurrealResult {
        result,
        status,
        time: format!("{:?}", start.elapsed()),
    }
}

impl SurrealDatabase {
    // --- Session Methods ---

    pub async fn use_ns(&self, ns: String) -> Result<Vec<SurrealResult>, String> {
        let start = std::time::Instant::now();
        match self.db.use_ns(ns).await {
            Ok(_) => Ok(vec![create_result(Some("Namespace selected".to_string()), "OK".to_string(), start)]),
            Err(e) => Err(e.to_string()),
        }
    }

    pub async fn use_db(&self, db: String) -> Result<Vec<SurrealResult>, String> {
        let start = std::time::Instant::now();
        match self.db.use_db(db).await {
            Ok(_) => Ok(vec![create_result(Some("Database selected".to_string()), "OK".to_string(), start)]),
            Err(e) => Err(e.to_string()),
        }
    }

    // --- Auth Methods ---

    pub async fn authenticate(&self, token: String) -> Result<Vec<SurrealResult>, String> {
        let start = std::time::Instant::now();
        match self.db.authenticate(token).await {
            Ok(_) => Ok(vec![create_result(Some("Authenticated".to_string()), "OK".to_string(), start)]),
            Err(e) => Err(e.to_string()),
        }
    }

    pub async fn invalidate(&self) -> Result<Vec<SurrealResult>, String> {
        let start = std::time::Instant::now();
        match self.db.invalidate().await {
            Ok(_) => Ok(vec![create_result(Some("Invalidated".to_string()), "OK".to_string(), start)]),
            Err(e) => Err(e.to_string()),
        }
    }

    pub async fn signin_root(&self, username: String, password: String) -> Result<Vec<SurrealResult>, String> {
        let start = std::time::Instant::now();
        let creds = Root {
            username: &username,
            password: &password,
        };
        match self.db.signin(creds).await {
            Ok(jwt) => {
                 let json_str = serde_json::to_string(&jwt).unwrap_or_else(|_| format!("{:?}", jwt));
                 Ok(vec![create_result(Some(json_str), "OK".to_string(), start)])
            },
            Err(e) => Err(e.to_string()),
        }
    }

    pub async fn signin_namespace(&self, username: String, password: String, namespace: String) -> Result<Vec<SurrealResult>, String> {
        let start = std::time::Instant::now();
        let creds = Namespace {
            username: &username,
            password: &password,
            namespace: &namespace,
        };
        match self.db.signin(creds).await {
            Ok(jwt) => {
                 let json_str = serde_json::to_string(&jwt).unwrap_or_else(|_| format!("{:?}", jwt));
                 Ok(vec![create_result(Some(json_str), "OK".to_string(), start)])
            },
            Err(e) => Err(e.to_string()),
        }
    }

    pub async fn signin_database(&self, username: String, password: String, namespace: String, database: String) -> Result<Vec<SurrealResult>, String> {
        let start = std::time::Instant::now();
        let creds = Database {
            username: &username,
            password: &password,
            namespace: &namespace,
            database: &database,
        };
        match self.db.signin(creds).await {
            Ok(jwt) => {
                 let json_str = serde_json::to_string(&jwt).unwrap_or_else(|_| format!("{:?}", jwt));
                 Ok(vec![create_result(Some(json_str), "OK".to_string(), start)])
            },
            Err(e) => Err(e.to_string()),
        }
    }

    // --- General Methods ---
    
    pub async fn health(&self) -> Result<Vec<SurrealResult>, String> {
        let start = std::time::Instant::now();
        match self.db.health().await {
            Ok(_) => Ok(vec![create_result(Some("Healthy".to_string()), "OK".to_string(), start)]),
            Err(e) => Err(e.to_string()),
        }
    }

    pub async fn version(&self) -> Result<Vec<SurrealResult>, String> {
        let start = std::time::Instant::now();
        match self.db.version().await {
            Ok(v) => Ok(vec![create_result(Some(v.to_string()), "OK".to_string(), start)]),
            Err(e) => Err(e.to_string()),
        }
    }

    // --- Query Method ---

    pub async fn query_db(&self, query: String) -> Result<Vec<SurrealResult>, String> {
        let start = std::time::Instant::now();
        let mut results = Vec::new();
        match self.db.query(query).await {
            Ok(mut response) => {
                 let num_statements = response.num_statements();
                 for i in 0..num_statements {
                     let result: Result<surrealdb::Value, _> = response.take(i);
                     match result {
                         Ok(val) => {
                             let json_val = serde_json::to_value(&val).unwrap_or(serde_json::Value::Null);
                             let json_str = serde_json::to_string(&json_val).ok();
                             results.push(create_result(json_str, "OK".to_string(), start));
                         },
                         Err(e) => {
                             results.push(create_result(None, format!("Error: {}", e), start));
                         }
                     }
                }
                Ok(results)
            }
            Err(e) => Err(format!("System Error: {}", e)),
        }
    }
}

#[flutter_rust_bridge::frb(init)]
pub fn init_app() {
    // Default utilities - feel free to customize
    flutter_rust_bridge::setup_default_user_utils();
}
