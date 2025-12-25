// rust/src/api/client.rs

use std::sync::Mutex;
use surrealdb::engine::any::Any;
use surrealdb::Surreal;
use crate::internal::{auth, crud, query};

/// Storage strategy for the database
pub enum StorageMode {
    Memory,
    Disk { path: String },
    Remote { url: String },
}

// The main class exposed to Flutter
pub struct SurrealDb {
    // Wrapped in Mutex<Option> to allow explicit closing
    db: Mutex<Option<Surreal<Any>>>,
}

impl SurrealDb {
    // --- Helper ---
    fn get_db(&self) -> anyhow::Result<Surreal<Any>> {
        let guard = self
            .db
            .lock()
            .map_err(|e| anyhow::anyhow!("Mutex poisoned: {}", e))?;
        if let Some(db) = &*guard {
            Ok(db.clone())
        } else {
            Err(anyhow::anyhow!("Database connection is closed"))
        }
    }

    // --- Connection ---
    pub async fn connect(mode: StorageMode) -> anyhow::Result<SurrealDb> {
        let endpoint = match &mode {
            StorageMode::Memory => "mem://".to_string(),
            StorageMode::Disk { path } => {
                // Ensure directory exists
                let path_obj = std::path::Path::new(path);
                if let Some(parent) = path_obj.parent() {
                    if !parent.exists() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                }
                format!("surrealkv://{}", path)
            }
            StorageMode::Remote { url } => url.clone(),
        };

        // Connect using the 'Any' engine (supports both mem and surrealkv)
        let db = surrealdb::engine::any::connect(&endpoint).await?;

        Ok(SurrealDb {
            db: Mutex::new(Some(db)),
        })
    }

    pub fn close(&self) -> anyhow::Result<()> {
        let mut guard = self
            .db
            .lock()
            .map_err(|e| anyhow::anyhow!("Mutex poisoned: {}", e))?;
        *guard = None;
        Ok(())
    }

    pub async fn use_db(&self, namespace: String, database: String) -> anyhow::Result<()> {
        let db = self.get_db()?;
        db.use_ns(namespace).use_db(database).await?;
        Ok(())
    }

    // --- Authentication ---

    pub async fn signup(&self, credentials_json: String) -> anyhow::Result<String> {
        let db = self.get_db()?;
        Ok(auth::signup(&db, credentials_json).await?)
    }

    pub async fn signin(&self, credentials_json: String) -> anyhow::Result<String> {
        let db = self.get_db()?;
        Ok(auth::signin(&db, credentials_json).await?)
    }

    pub async fn authenticate(&self, token: String) -> anyhow::Result<()> {
        let db = self.get_db()?;
        Ok(auth::authenticate(&db, token).await?)
    }

    pub async fn invalidate(&self) -> anyhow::Result<()> {
        let db = self.get_db()?;
        Ok(auth::invalidate(&db).await?)
    }

    // --- Data Methods (CRUD) ---

    /// Execute a raw SQL query.
    /// `vars` should be a JSON string of bind variables, e.g. `{"id": "...", "val": 123}`.
    /// Execute a raw SQL query.
    /// `vars` should be a JSON string of bind variables, e.g. `{"id": "...", "val": 123}`.
    pub async fn query(&self, sql: String, vars: Option<String>) -> anyhow::Result<String> {
        let db = self.get_db()?;
        Ok(query::query(&db, sql, vars).await?)
    }

    // --- Simplified CRUD Shortcuts ---

    pub async fn select(&self, resource: String) -> anyhow::Result<String> {
        let db = self.get_db()?;
        Ok(crud::select(&db, resource).await?)
    }

    pub async fn create(&self, resource: String, data: Option<String>) -> anyhow::Result<String> {
        let db = self.get_db()?;
        Ok(crud::create(&db, resource, data).await?)
    }

    pub async fn update(&self, resource: String, data: Option<String>) -> anyhow::Result<String> {
        let db = self.get_db()?;
        Ok(crud::update(&db, resource, data).await?)
    }

    pub async fn merge(&self, resource: String, data: Option<String>) -> anyhow::Result<String> {
        let db = self.get_db()?;
        Ok(crud::merge(&db, resource, data).await?)
    }

    pub async fn delete(&self, resource: String) -> anyhow::Result<String> {
        let db = self.get_db()?;
        Ok(crud::delete(&db, resource).await?)
    }

    // --- Transaction ---

    pub async fn transaction(&self, statements: String, vars: Option<String>) -> anyhow::Result<String> {
        let db = self.get_db()?;
        Ok(query::transaction(&db, statements, vars).await?)
    }

    pub async fn query_begin(&self) -> anyhow::Result<()> {
        let db = self.get_db()?;
        Ok(query::query_begin(&db).await?)
    }

    pub async fn query_commit(&self) -> anyhow::Result<()> {
        let db = self.get_db()?;
        Ok(query::query_commit(&db).await?)
    }

    pub async fn query_cancel(&self) -> anyhow::Result<()> {
        let db = self.get_db()?;
        Ok(query::query_cancel(&db).await?)
    }
}
