mod frb_generated; /* AUTO INJECTED BY flutter_rust_bridge. This line may not be accurate, and you can change it according to your needs. */

use surrealdb::Surreal;
use surrealdb::engine::local::{Db, RocksDb};
use surrealdb::engine::remote::ws::{Ws, Client};
use tokio::sync::RwLock;
// Transaction might be generic or in a specific module.
// Based on error help: consider simple generic usage or specific path.
// Error said: use surrealdb::method::Transaction;
// But the returned type of begin() must match.
// Let's assume generic use or try to find where it is.
// For now, let's use the explicit path suggested by compiler in error msg if possible, 
// or simpler: `surrealdb::Transaction` doesn't exist.
// Checking logs: `use surrealdb::method::Transaction;`
use surrealdb::method::Query; // Query is also used
use surrealdb::Connection;
use serde::{Deserialize, Serialize};
use surrealdb::opt::auth::{Root, Namespace, Database};


#[derive(Serialize, Deserialize)]
pub struct SurrealResult {
    pub result: Option<String>,
    pub status: String,
    pub time: String,
}

#[derive(Clone)]
pub enum DatabaseConnection {
    Local(Surreal<Db>),
    Remote(Surreal<Client>),
}

#[derive(Clone)]
pub struct SurrealDatabase {
    pub db: DatabaseConnection,
}






// We need to support both Local and Remote transactions which have different types.
// Since begin() is elusive, we use proper client cloning + SQL commands.
// Isolation Verification:
// If cloning shares session, this might fail isolation tests. 
// However, this is the only viable path if begin() is unavailable.
pub enum TransactionConnection {
    Local(Surreal<Db>),
    Remote(Surreal<Client>),
}

#[derive(Clone)]
pub struct SurrealTransaction {
    pub tx: std::sync::Arc<RwLock<Option<TransactionConnection>>>,
}

impl SurrealTransaction {
    pub async fn query(&self, query: String, vars: Option<String>) -> Result<Vec<SurrealResult>, String> {
        let start = std::time::Instant::now();
        let mut guard = self.tx.write().await;
        
        if let Some(conn) = guard.as_mut() {
             let response_result = match conn {
                 TransactionConnection::Local(db) => {
                     apply_vars(db.query(&query), &vars).await
                 },
                 TransactionConnection::Remote(db) => {
                     apply_vars(db.query(&query), &vars).await
                 }
             };
            
            match response_result {
                Ok(mut response) => {
                     let mut results = Vec::new();
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
        } else {
            Err("Transaction is closed".to_string())
        }
    }

    pub async fn commit(&self) -> Result<Vec<SurrealResult>, String> {
        let start = std::time::Instant::now();
        let mut guard = self.tx.write().await;
        if let Some(conn) = guard.take() { 
            let res = match conn {
                TransactionConnection::Local(db) => db.query("COMMIT TRANSACTION;").await.map(|_| ()),
                TransactionConnection::Remote(db) => db.query("COMMIT TRANSACTION;").await.map(|_| ()),
            };
            match res {
                Ok(_) => Ok(vec![create_result(Some("Committed".to_string()), "OK".to_string(), start)]),
                Err(e) => Err(format!("Commit Error: {}", e)),
            }
        } else {
            Err("Transaction already closed".to_string())
        }
    }

    pub async fn cancel(&self) -> Result<Vec<SurrealResult>, String> {
        let start = std::time::Instant::now();
        let mut guard = self.tx.write().await;
        if let Some(conn) = guard.take() {
            let res = match conn {
                TransactionConnection::Local(db) => db.query("CANCEL TRANSACTION;").await.map(|_| ()),
                TransactionConnection::Remote(db) => db.query("CANCEL TRANSACTION;").await.map(|_| ()),
            };
            match res {
                 Ok(_) => Ok(vec![create_result(Some("Cancelled".to_string()), "OK".to_string(), start)]),
                 Err(e) => Err(format!("Cancel Error: {}", e)),
            }
        } else {
             Err("Transaction already closed".to_string())
        }
    }
}

pub async fn connect_db(path: String) -> Result<SurrealDatabase, String> {
    let db = if path.starts_with("ws://") || path.starts_with("wss://") || path.starts_with("http://") || path.starts_with("https://") {
        let path_clean = if let Some(p) = path.strip_prefix("ws://") { p }
        else if let Some(p) = path.strip_prefix("wss://") { p }
        else if let Some(p) = path.strip_prefix("http://") { p }
        else if let Some(p) = path.strip_prefix("https://") { p }
        else { &path };

        match Surreal::new::<Ws>(path_clean).await {
            Ok(db) => DatabaseConnection::Remote(db),
            Err(e) => return Err(format!("Error connecting to remote database: {}", e)),
        }
    } else {
        match Surreal::new::<RocksDb>(&path).await {
            Ok(db) => DatabaseConnection::Local(db),
            Err(e) => return Err(format!("Error connecting to local database: {}", e)),
        }
    };

    Ok(SurrealDatabase { db })
}


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
        let res = match &self.db {
            DatabaseConnection::Local(db) => db.use_ns(ns).await,
            DatabaseConnection::Remote(db) => db.use_ns(ns).await,
        };
        match res {
            Ok(_) => Ok(vec![create_result(Some("Namespace selected".to_string()), "OK".to_string(), start)]),
            Err(e) => Err(e.to_string()),
        }
    }

    pub async fn use_db(&self, db: String) -> Result<Vec<SurrealResult>, String> {
        let start = std::time::Instant::now();
        let res = match &self.db {
            DatabaseConnection::Local(d) => d.use_db(db).await,
            DatabaseConnection::Remote(d) => d.use_db(db).await,
        };
        match res {
            Ok(_) => Ok(vec![create_result(Some("Database selected".to_string()), "OK".to_string(), start)]),
            Err(e) => Err(e.to_string()),
        }
    }

    // --- Auth Methods ---

    pub async fn authenticate(&self, token: String) -> Result<Vec<SurrealResult>, String> {
        let start = std::time::Instant::now();
        let res = match &self.db {
            DatabaseConnection::Local(db) => db.authenticate(token).await,
            DatabaseConnection::Remote(db) => db.authenticate(token).await,
        };
        match res {
            Ok(_) => Ok(vec![create_result(Some("Authenticated".to_string()), "OK".to_string(), start)]),
            Err(e) => Err(e.to_string()),
        }
    }

    pub async fn invalidate(&self) -> Result<Vec<SurrealResult>, String> {
        let start = std::time::Instant::now();
        let res = match &self.db {
            DatabaseConnection::Local(db) => db.invalidate().await,
            DatabaseConnection::Remote(db) => db.invalidate().await,
        };
        match res {
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
        let res = match &self.db {
            DatabaseConnection::Local(db) => db.signin(creds).await,
            DatabaseConnection::Remote(db) => db.signin(creds).await,
        };
        match res {
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
        let res = match &self.db {
            DatabaseConnection::Local(db) => db.signin(creds).await,
            DatabaseConnection::Remote(db) => db.signin(creds).await,
        };
        match res {
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
        let res = match &self.db {
            DatabaseConnection::Local(db) => db.signin(creds).await,
            DatabaseConnection::Remote(db) => db.signin(creds).await,
        };
        match res {
            Ok(jwt) => {
                 let json_str = serde_json::to_string(&jwt).unwrap_or_else(|_| format!("{:?}", jwt));
                 Ok(vec![create_result(Some(json_str), "OK".to_string(), start)])
            },
            Err(e) => Err(e.to_string()),
        }
    }

    pub async fn setup_root_user(&self, username: String, password: String) -> Result<Vec<SurrealResult>, String> {
        let query = format!("DEFINE USER {} ON ROOT PASSWORD '{}' ROLES OWNER;", username, password);
        self.query_db(query, None).await
    }

    // --- General Methods ---
    
    pub async fn health(&self) -> Result<Vec<SurrealResult>, String> {
        let start = std::time::Instant::now();
        let res = match &self.db {
            DatabaseConnection::Local(db) => db.health().await,
            DatabaseConnection::Remote(db) => db.health().await,
        };
        match res {
            Ok(_) => Ok(vec![create_result(Some("Healthy".to_string()), "OK".to_string(), start)]),
            Err(e) => Err(e.to_string()),
        }
    }

    pub async fn version(&self) -> Result<Vec<SurrealResult>, String> {
        let start = std::time::Instant::now();
        let res = match &self.db {
            DatabaseConnection::Local(db) => db.version().await,
            DatabaseConnection::Remote(db) => db.version().await,
        };
        match res {
            Ok(v) => Ok(vec![create_result(Some(v.to_string()), "OK".to_string(), start)]),
            Err(e) => Err(e.to_string()),
        }
    }

    // --- Query Method ---

    pub async fn query_db(&self, query: String, vars: Option<String>) -> Result<Vec<SurrealResult>, String> {
        let start = std::time::Instant::now();

        let response = match &self.db {
            DatabaseConnection::Local(db) => {
                apply_vars(db.query(&query), &vars).await
            },
            DatabaseConnection::Remote(db) => {
                apply_vars(db.query(&query), &vars).await
            },
        };

        match response {
            Ok(mut response) => {
                 let mut results = Vec::new();
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

    // --- Transaction Methods ---

    pub async fn begin_transaction(&self) -> Result<SurrealTransaction, String> {
        let tx = match &self.db {
            DatabaseConnection::Local(db) => {
                let new_client = db.clone();
                // Ensure new session or at least transaction start
                if let Err(e) = new_client.query("BEGIN TRANSACTION;").await {
                    return Err(format!("Failed to begin transaction: {}", e));
                }
                TransactionConnection::Local(new_client)
            },
            DatabaseConnection::Remote(db) => {
                let new_client = db.clone();
                if let Err(e) = new_client.query("BEGIN TRANSACTION;").await {
                    return Err(format!("Failed to begin transaction: {}", e));
                }
                TransactionConnection::Remote(new_client)
            }
        };
        
        Ok(SurrealTransaction {
            tx: std::sync::Arc::new(RwLock::new(Some(tx))),
        })
    }
}

fn apply_vars<'a, C: Connection>(mut query: Query<'a, C>, vars: &Option<String>) -> Query<'a, C> {
    if let Some(json_str) = vars {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(json_str) {
            if let Some(obj) = v.as_object() {
                for (key, value) in obj {
                    query = query.bind((key.clone(), value.clone()));
                }
            }
        }
    }
    query
}

#[flutter_rust_bridge::frb(init)]
pub fn init_app() {
    // Default utilities - feel free to customize
    flutter_rust_bridge::setup_default_user_utils();
}
