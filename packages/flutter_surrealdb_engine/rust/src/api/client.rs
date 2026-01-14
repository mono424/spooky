use std::process::{Child, Command};
use tokio::sync::Mutex;
use std::time::Duration;
use std::path::Path;
use surrealdb::engine::any::Any;
use surrealdb::Surreal;
use crate::internal::{auth, crud, queries};

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

// NOTE: We don't need the `SurrealClient` enum anymore.
// We strictly use `Surreal<Any>` for everything, which simplifies the code significantly.

pub struct SurrealDb {
    db: Mutex<Option<Surreal<Any>>>,
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
                // Use "mem://" scheme which `engine::any` supports
                let db = surrealdb::engine::any::connect("mem://").await?;
                (db, None)
            },
            StorageMode::Remote { url } => {
                let db = surrealdb::engine::any::connect(url).await?;
                (db, None)
            },
            StorageMode::Disk { path } => {
                Self::ensure_dir_exists(&path);
                // "surrealkv://" scheme
                let db = surrealdb::engine::any::connect(format!("surrealkv://{}", path)).await?;
                (db, None)
            },
            StorageMode::DevSidecar { path, port } => {
                Self::ensure_dir_exists(&path);
                // Spawns sidecar but returns a WebSocket connection (client) as Surreal<Any>
                let (db, guard) = Self::spawn_sidecar_server(&path, port).await?;
                (db, guard)
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

                // Using `any::connect` here ensures we get a Surreal<Any> instance back
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

    // --- Concurrency Optimization ---
    // Instead of returning the enum or holding the lock, we clone the client (Arc)
    // and release the lock immediately.
    pub(crate) async fn get_client(&self) -> anyhow::Result<Surreal<Any>> {
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
            client.use_ns(ns).await?;
            client.use_db(db).await?;
        }
        Ok(())
    }

    pub async fn signup(&self, creds: String) -> anyhow::Result<String> {
        let guard = self.db.lock().await;
        if let Some(client) = &*guard {
            return auth::signup(client, creds).await;
        }
        Err(anyhow::anyhow!("Database not connected"))
    }

    pub async fn signin(&self, creds: String) -> anyhow::Result<String> {
        let guard = self.db.lock().await;
        if let Some(client) = &*guard {
            return auth::signin(client, creds).await;
        }
        Err(anyhow::anyhow!("Database not connected"))
    }

    pub async fn authenticate(&self, token: String) -> anyhow::Result<()> {
        let guard = self.db.lock().await;
        if let Some(client) = &*guard {
            auth::authenticate(client, token).await?;
        }
        Ok(())
    }

    pub async fn invalidate(&self) -> anyhow::Result<()> {
        let guard = self.db.lock().await;
        if let Some(client) = &*guard {
            auth::invalidate(client).await?;
        }
        Ok(())
    }

    pub async fn query(&self, sql: String, vars: Option<String>) -> anyhow::Result<String> {
        let client = self.get_client().await?;
        Ok(queries::query(&client, sql, vars).await?)
    }

    pub async fn query_typed(&self, sql: String, vars: Option<String>) -> anyhow::Result<String> {
        let client = self.get_client().await?;
        Ok(queries::query_typed(&client, sql, vars).await?)
    }

    pub async fn select(&self, resource: String) -> anyhow::Result<String> {
        let client = self.get_client().await?;
        Ok(crud::select(&client, resource).await?)
    }

    pub async fn create(&self, resource: String, data: Option<String>) -> anyhow::Result<String> {
        let client = self.get_client().await?;
        Ok(crud::create(&client, resource, data).await?)
    }

    pub async fn update(&self, resource: String, data: Option<String>) -> anyhow::Result<String> {
        let client = self.get_client().await?;
        Ok(crud::update(&client, resource, data).await?)
    }

    pub async fn merge(&self, resource: String, data: Option<String>) -> anyhow::Result<String> {
        let client = self.get_client().await?;
        Ok(crud::merge(&client, resource, data).await?)
    }

    pub async fn delete(&self, resource: String) -> anyhow::Result<String> {
        let client = self.get_client().await?;
        Ok(crud::delete(&client, resource).await?)
    }

    pub async fn transaction(&self, stmts: String, vars: Option<String>) -> anyhow::Result<String> {
        let client = self.get_client().await?;
        Ok(queries::transaction(&client, stmts, vars).await?)
    }

    pub async fn query_begin(&self) -> anyhow::Result<()> {
        let client = self.get_client().await?;
        Ok(queries::query_begin(&client).await?)
    }

    pub async fn query_commit(&self) -> anyhow::Result<()> {
        let client = self.get_client().await?;
        Ok(queries::query_commit(&client).await?)
    }

    pub async fn query_cancel(&self) -> anyhow::Result<()> {
        let client = self.get_client().await?;
        Ok(queries::query_cancel(&client).await?)
    }

    pub async fn export(&self, path: String) -> anyhow::Result<()> {
        let client = self.get_client().await?;
        client.export(path).await?;
        Ok(())
    }

    /// Starts a pure Live Query Stream (No Snapshot) - Legacy/Standard behavior
    pub async fn connect_live_query(
        &self,
        table: String,
        sink: crate::frb_generated::StreamSink<crate::api::live_query::models::LiveQueryEvent>,
    ) -> anyhow::Result<()> {
        // Leverages the existing 'legacy' implementation in api/live_query/mod.rs
        // This ensures identical behavior to the reference commit (Handshake, Manager, etc.)
        self.live_query(table, sink).await
    }

    /// Starts a Live Query Stream WITH an initial Snapshot of the table
    pub async fn connect_live_query_with_snapshot(
        &self,
        table: String,
        sink: crate::frb_generated::StreamSink<crate::api::live_query::models::LiveQueryEvent>,
    ) -> anyhow::Result<()> {
        let client = self.get_client().await?;
        crate::api::realtime::connect_live_query_with_snapshot(client, table, sink).await
    }
}