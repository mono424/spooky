use std::process::{Child, Command};
use tokio::sync::Mutex;
use std::time::Duration;
use std::path::Path;
use surrealdb::engine::any::Any;
use surrealdb::engine::local::{Mem, Db};
use surrealdb::Surreal;
use crate::internal::{auth, crud, query};

// --- Enums & Structs ---

/// Storage strategy for the database
pub enum StorageMode {
    Memory,
    Disk { path: String },
    Remote { url: String },
    /// Starts a local sidecar server connection (Desktop only)
    DevSidecar { path: String, port: u16 },
}

/// Guard to ensure the child process is killed when the struct is dropped
struct ServerGuard {
    process: Child,
}

impl Drop for ServerGuard {
    fn drop(&mut self) {
        #[cfg(unix)]
        self.graceful_shutdown();

        // Always force kill as backup
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

impl ServerGuard {
    #[cfg(unix)]
    fn graceful_shutdown(&mut self) {
        use std::time::Instant;
        
        let pid = self.process.id();
        let _ = Command::new("kill").args(&["-TERM", &pid.to_string()]).output();

        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(2000) {
            if let Ok(Some(_)) = self.process.try_wait() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

// Helper enum to hold different client types
#[derive(Clone)]
pub enum SurrealClient {
    Any(Surreal<Any>),
    Mem(Surreal<Db>),
}

pub struct SurrealDb {
    db: Mutex<Option<SurrealClient>>,
    #[allow(dead_code)] 
    server_guard: Mutex<Option<ServerGuard>>,
}

// --- Implementation ---

impl SurrealDb {
    
    // =================================================================
    // Connection & Setup
    // =================================================================

    pub async fn connect(mode: StorageMode) -> anyhow::Result<SurrealDb> {
        let (db_client, server_guard) = match mode {
            StorageMode::Memory => {
                // Live Query Fix: Use Mem engine directly for in-memory databases
                let db = Surreal::new::<Mem>(()).await?;
                (SurrealClient::Mem(db), None)
            },
            StorageMode::Remote { url } => {
                let db = surrealdb::engine::any::connect(url).await?;
                (SurrealClient::Any(db), None)
            },
            StorageMode::Disk { path } => {
                Self::ensure_dir_exists(&path);
                let db = surrealdb::engine::any::connect(format!("surrealkv://{}", path)).await?;
                (SurrealClient::Any(db), None)
            },
            StorageMode::DevSidecar { path, port } => {
                Self::ensure_dir_exists(&path);
                let (db, guard) = Self::spawn_sidecar_server(&path, port).await?;
                (SurrealClient::Any(db), guard)
            }
        };

        Ok(SurrealDb {
            db: Mutex::new(Some(db_client)),
            server_guard: Mutex::new(server_guard),
        })
    }

    pub async fn close(&self) -> anyhow::Result<()> {
        let mut guard = self.db.lock().await;
        *guard = None;
        let mut server = self.server_guard.lock().await;
        *server = None;
        Ok(())
    }

    // =================================================================
    // Private Helpers
    // =================================================================

    fn ensure_dir_exists(path_str: &str) {
        let path = Path::new(path_str);
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                let _ = std::fs::create_dir_all(parent);
            }
        }
    }

    /// Handles the complex logic of spawning a sidecar server securely
    async fn spawn_sidecar_server(path: &str, port: u16) -> anyhow::Result<(Surreal<Any>, Option<ServerGuard>)> {
        #[cfg(any(target_os = "android", target_os = "ios"))]
        return Err(anyhow::anyhow!("DevSidecar not supported on mobile"));

        let bind_addr = format!("0.0.0.0:{}", port); // External access allowed
        let db_url_arg = format!("surrealkv://{}", path);
        let endpoint = format!("ws://127.0.0.1:{}/rpc", port);

        // Attempt Loop: Tries multiple times to clear port and start server
        for attempt in 1..=5 {
            // 1. Kill Zombies (Unix only)
            #[cfg(unix)]
            Self::kill_zombie_processes(port).await;

            // 2. Spawn Process
            println!("DevSidecar: Spawning attempt {}/5...", attempt);
            let mut child = Command::new("surreal")
                .args(&["start", "--allow-all", "--user", "root", "--pass", "root", "--bind", &bind_addr, &db_url_arg])
                .env("SURREAL_CAPS_ALLOW_EXPERIMENTAL", "surrealism,files")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::inherit()) // Logs visible in Flutter
                .spawn()
                .map_err(|e| anyhow::anyhow!("Failed to spawn surreal: {}", e))?;

            // 3. Early Crash Check
            tokio::time::sleep(Duration::from_millis(1000)).await;
            if let Ok(Some(status)) = child.try_wait() {
                println!("DevSidecar: Crashed early (Status: {}). Retrying...", status);
                continue;
            }

            // 4. Connect Loop
            let mut loop_guard = ServerGuard { process: child };
            
            for _ in 0..20 { // ~4 seconds connection timeout
                if let Ok(Some(_)) = loop_guard.process.try_wait() { break; } // Died while connecting

                if let Ok(db) = surrealdb::engine::any::connect(&endpoint).await {
                    // 5. Auth & Return
                    db.signin(surrealdb::opt::auth::Root {
                        username: "root".to_string(),
                        password: "root".to_string(),
                    }).await?;
                    
                    return Ok((db, Some(loop_guard)));
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            
            println!("DevSidecar: Connection timed out. Cleaning up...");
            // loop_guard drops here, killing the process automatically
        }

        Err(anyhow::anyhow!("Failed to start DevSidecar after multiple attempts. Check console logs."))
    }

    #[cfg(unix)]
    async fn kill_zombie_processes(port: u16) {
        let output = Command::new("lsof").args(&["-t", &format!("-i:{}", port)]).output();
        
        if let Ok(out) = output {
            if !out.stdout.is_empty() {
                let pids = String::from_utf8_lossy(&out.stdout);
                let my_pid = std::process::id();
                
                for pid_str in pids.split_whitespace() {
                    if let Ok(pid) = pid_str.parse::<u32>() {
                        if pid != my_pid {
                            let _ = Command::new("kill").args(&["-9", &pid.to_string()]).output();
                        }
                    }
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }
    }

    pub(crate) async fn get_db_client(&self) -> anyhow::Result<SurrealClient> {
        let guard = self.db.lock().await;
        match &*guard {
            Some(client) => Ok(client.clone()),
            None => Err(anyhow::anyhow!("Database connection is closed")),
        }
    }

    // =================================================================
    // Public API Methods (Delegates)
    // =================================================================

    pub async fn use_db(&self, ns: String, db: String) -> anyhow::Result<()> {
        let guard = self.db.lock().await; 
        if let Some(client) = &*guard {
            match client {
                SurrealClient::Any(c) => c.use_ns(ns).use_db(db).await?,
                SurrealClient::Mem(c) => c.use_ns(ns).use_db(db).await?,
            };
        }
        Ok(())
    }

    pub async fn signup(&self, creds: String) -> anyhow::Result<String> {
        let guard = self.db.lock().await;
        if let Some(client) = &*guard {
             return match client {
                 SurrealClient::Any(c) => auth::signup(c, creds).await,
                 SurrealClient::Mem(c) => auth::signup(c, creds).await,
             };
        }
        Err(anyhow::anyhow!("Database not connected"))
    }

    pub async fn signin(&self, creds: String) -> anyhow::Result<String> {
        let guard = self.db.lock().await;
        if let Some(client) = &*guard {
             return match client {
                 SurrealClient::Any(c) => auth::signin(c, creds).await,
                 SurrealClient::Mem(c) => auth::signin(c, creds).await,
             };
        }
        Err(anyhow::anyhow!("Database not connected"))
    }

    pub async fn authenticate(&self, token: String) -> anyhow::Result<()> {
        let guard = self.db.lock().await;
        if let Some(client) = &*guard {
            match client {
                SurrealClient::Any(c) => auth::authenticate(c, token).await?,
                SurrealClient::Mem(c) => auth::authenticate(c, token).await?,
            };
        }
        Ok(())
    }

    pub async fn invalidate(&self) -> anyhow::Result<()> {
        let guard = self.db.lock().await;
        if let Some(client) = &*guard {
            match client {
                SurrealClient::Any(c) => auth::invalidate(c).await?,
                SurrealClient::Mem(c) => auth::invalidate(c).await?,
            };
        }
        Ok(())
    }

    pub async fn query(&self, sql: String, vars: Option<String>) -> anyhow::Result<String> {
        match self.get_db_client().await? {
            SurrealClient::Any(c) => Ok(query::query(&c, sql, vars).await?),
            SurrealClient::Mem(c) => Ok(query::query(&c, sql, vars).await?),
        }
    }

    pub async fn select(&self, resource: String) -> anyhow::Result<String> {
        match self.get_db_client().await? {
            SurrealClient::Any(c) => Ok(crud::select(&c, resource).await?),
            SurrealClient::Mem(c) => Ok(crud::select(&c, resource).await?),
        }
    }

    pub async fn create(&self, resource: String, data: Option<String>) -> anyhow::Result<String> {
         match self.get_db_client().await? {
            SurrealClient::Any(c) => Ok(crud::create(&c, resource, data).await?),
            SurrealClient::Mem(c) => Ok(crud::create(&c, resource, data).await?),
        }
    }

    pub async fn update(&self, resource: String, data: Option<String>) -> anyhow::Result<String> {
         match self.get_db_client().await? {
            SurrealClient::Any(c) => Ok(crud::update(&c, resource, data).await?),
            SurrealClient::Mem(c) => Ok(crud::update(&c, resource, data).await?),
        }
    }

    pub async fn merge(&self, resource: String, data: Option<String>) -> anyhow::Result<String> {
         match self.get_db_client().await? {
            SurrealClient::Any(c) => Ok(crud::merge(&c, resource, data).await?),
            SurrealClient::Mem(c) => Ok(crud::merge(&c, resource, data).await?),
        }
    }

    pub async fn delete(&self, resource: String) -> anyhow::Result<String> {
         match self.get_db_client().await? {
            SurrealClient::Any(c) => Ok(crud::delete(&c, resource).await?),
            SurrealClient::Mem(c) => Ok(crud::delete(&c, resource).await?),
        }
    }

    pub async fn transaction(&self, stmts: String, vars: Option<String>) -> anyhow::Result<String> {
         match self.get_db_client().await? {
            SurrealClient::Any(c) => Ok(query::transaction(&c, stmts, vars).await?),
            SurrealClient::Mem(c) => Ok(query::transaction(&c, stmts, vars).await?),
        }
    }

    pub async fn query_begin(&self) -> anyhow::Result<()> {
        match self.get_db_client().await? {
            SurrealClient::Any(c) => Ok(query::query_begin(&c).await?),
            SurrealClient::Mem(c) => Ok(query::query_begin(&c).await?),
        }
    }

    pub async fn query_commit(&self) -> anyhow::Result<()> {
        match self.get_db_client().await? {
            SurrealClient::Any(c) => Ok(query::query_commit(&c).await?),
            SurrealClient::Mem(c) => Ok(query::query_commit(&c).await?),
        }
    }

    pub async fn query_cancel(&self) -> anyhow::Result<()> {
        match self.get_db_client().await? {
            SurrealClient::Any(c) => Ok(query::query_cancel(&c).await?),
            SurrealClient::Mem(c) => Ok(query::query_cancel(&c).await?),
        }
    }

    pub async fn export(&self, path: String) -> anyhow::Result<()> {
         match self.get_db_client().await? {
            SurrealClient::Any(c) => c.export(path).await?,
            SurrealClient::Mem(c) => c.export(path).await?,
        };
        Ok(())
    }
}