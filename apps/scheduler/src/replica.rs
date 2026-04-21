use anyhow::{Context, Result};
use serde_json::Value;
use std::path::PathBuf;
use surrealdb::engine::local::RocksDb;
use surrealdb::Surreal;
use tracing::{debug, info, warn};

// Re-export RecordOp from messages to avoid duplication
pub use crate::messages::RecordOp;

/// Build a full SurrealDB thing ID, handling both `"table:id"` and bare `"id"` formats.
/// SurrealDB event triggers send IDs that already include the table prefix (e.g. `"user:abc"`),
/// so we must avoid doubling it into `"user:user:abc"`.
fn build_thing_id(table: &str, id: &str) -> String {
    let prefix = format!("{}:", table);
    if id.starts_with(&prefix) {
        id.to_string()
    } else {
        format!("{}:{}", table, id)
    }
}

/// Chunk of replica data for bootstrap
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ReplicaChunk {
    pub chunk_index: usize,
    pub table: String,
    pub records: Vec<(String, Value)>,
}

/// Persistent replica backed by embedded SurrealDB with RocksDB
pub struct Replica {
    db: Surreal<surrealdb::engine::local::Db>,
    db_path: PathBuf,
    /// Sequence number of the last event applied to this snapshot
    snapshot_seq: u64,
}

impl Replica {
    /// Create a new replica with persistent SurrealDB/RocksDB storage
    pub async fn new(db_path: PathBuf) -> Result<Self> {
        // Create parent directory if it doesn't exist
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory: {:?}", parent))?;
        }

        let db = Surreal::new::<RocksDb>(db_path.to_str().unwrap_or("./data/replica"))
            .await
            .with_context(|| format!("Failed to open RocksDB at {:?}", db_path))?;

        db.use_ns("sp00ky").use_db("snapshot").await
            .context("Failed to select namespace/database on replica")?;

        info!("Opened replica SurrealDB at {:?}", db_path);

        // Try to read snapshot_seq from metadata
        let snapshot_seq = Self::read_snapshot_seq_from_db(&db).await.unwrap_or(0);
        if snapshot_seq > 0 {
            info!(snapshot_seq, "Restored snapshot sequence from metadata");
        }

        Ok(Self {
            db,
            db_path,
            snapshot_seq,
        })
    }

    /// Get current snapshot sequence number
    pub fn snapshot_seq(&self) -> u64 {
        self.snapshot_seq
    }

    /// Set snapshot sequence number and persist it
    pub async fn set_snapshot_seq(&mut self, seq: u64) -> Result<()> {
        self.snapshot_seq = seq;
        self.db
            .query("UPSERT _00_metadata:snapshot SET seq = $seq")
            .bind(("seq", seq))
            .await
            .context("Failed to persist snapshot_seq")?;
        Ok(())
    }

    /// Read snapshot_seq from the embedded DB metadata table
    async fn read_snapshot_seq_from_db(db: &Surreal<surrealdb::engine::local::Db>) -> Result<u64> {
        let mut response = db
            .query("SELECT seq FROM _00_metadata:snapshot")
            .await
            .context("Failed to query snapshot metadata")?;

        let rows: Vec<Value> = response.take(0).unwrap_or_default();
        if let Some(row) = rows.first() {
            if let Some(seq) = row.get("seq").and_then(|v| v.as_u64()) {
                return Ok(seq);
            }
        }
        Ok(0)
    }

    /// Full initial load from a remote SurrealDB instance
    pub async fn ingest_all<C>(&mut self, remote_db: &surrealdb::Surreal<C>) -> Result<()>
    where
        C: surrealdb::Connection,
    {
        // Discover tables from remote
        let mut response = remote_db
            .query("INFO FOR DB")
            .await
            .context("Failed to query INFO FOR DB on remote")?;

        let info: Vec<Value> = response.take(0).unwrap_or_default();
        let info = info.into_iter().next().unwrap_or_default();

        let tables: Vec<String> = match info.get("tables") {
            Some(Value::Object(tables_map)) => tables_map
                .keys()
                .filter(|name| !name.starts_with("_00_"))
                .cloned()
                .collect(),
            _ => {
                // Fallback to known tables
                vec!["thread".to_string(), "job".to_string(), "user".to_string()]
            }
        };

        for table_name in &tables {
            info!("Ingesting table '{}' from remote...", table_name);

            let records: Vec<Value> = match remote_db
                .query(format!("SELECT * FROM {}", table_name))
                .await
            {
                Ok(mut response) => response.take(0).unwrap_or_default(),
                Err(e) => {
                    warn!("Skipping table '{}': failed to select: {}", table_name, e);
                    continue;
                }
            };

            let count = records.len();

            // Insert each record into the local embedded DB
            for record in records {
                if let Some(id) = record.get("id") {
                    let id_str = id.to_string().trim_matches('"').to_string();
                    let thing_id = build_thing_id(table_name, &id_str);
                    // Use CREATE with the full record data
                    if let Err(e) = self.db
                        .query(format!("CREATE {} CONTENT $data", thing_id))
                        .bind(("data", record))
                        .await
                    {
                        debug!("Failed to insert {}: {}", thing_id, e);
                    }
                }
            }

            // Also copy _00_query table for views
            info!("Ingested {} records from '{}'", count, table_name);
        }

        // Copy view definitions
        let mut response = remote_db
            .query("SELECT * FROM _00_query")
            .await
            .context("Failed to query _00_query on remote")?;

        let views: Vec<Value> = response.take(0).unwrap_or_default();
        for view in &views {
            if let Some(id) = view.get("id") {
                let id_str = id.to_string().trim_matches('"').to_string();
                let key = if id_str.starts_with("_00_query:") {
                    id_str.clone()
                } else {
                    format!("_00_query:{}", id_str)
                };
                if let Err(e) = self.db
                    .query(format!("CREATE {} CONTENT $data", key))
                    .bind(("data", view.clone()))
                    .await
                {
                    debug!("Failed to insert view {}: {}", key, e);
                }
            }
        }
        info!("Copied {} view definitions", views.len());

        Ok(())
    }

    /// Apply a single record event to the snapshot
    pub async fn apply(&self, table: &str, op: RecordOp, id: &str, record: Option<Value>) -> Result<()> {
        let thing_id = build_thing_id(table, id);
        match op {
            RecordOp::Create => {
                if let Some(data) = record {
                    self.db
                        .query(format!("CREATE {} CONTENT $data", thing_id))
                        .bind(("data", data))
                        .await
                        .with_context(|| format!("Failed to CREATE {}", thing_id))?;
                }
            }
            RecordOp::Update => {
                if let Some(data) = record {
                    self.db
                        .query(format!("UPDATE {} MERGE $data", thing_id))
                        .bind(("data", data))
                        .await
                        .with_context(|| format!("Failed to UPDATE {}", thing_id))?;
                }
            }
            RecordOp::Delete => {
                self.db
                    .query(format!("DELETE {}", thing_id))
                    .await
                    .with_context(|| format!("Failed to DELETE {}", thing_id))?;
            }
        }

        debug!("Applied {:?} for {}", op, thing_id);
        Ok(())
    }

    /// Export the replica to a file using SurrealDB's native export.
    /// Produces a standard SurrealQL dump importable via `surreal import`.
    pub async fn export_to_file(&self, path: &std::path::Path) -> Result<()> {
        self.db
            .export(path)
            .await
            .with_context(|| format!("Failed to export replica to {:?}", path))?;
        Ok(())
    }

    /// Import a SurrealQL dump file into the replica. Caller must ensure the
    /// underlying DB is empty (call `reset` first) — `import` executes the
    /// statements from the file and will error on duplicate records.
    pub async fn import_from_file(&self, path: &std::path::Path) -> Result<()> {
        self.db
            .import(path)
            .await
            .with_context(|| format!("Failed to import replica from {:?}", path))?;
        Ok(())
    }

    /// Wipe the replica's logical contents in place via SurrealQL. Resets
    /// `snapshot_seq` to 0. The caller must hold the write lock on the replica.
    ///
    /// We deliberately do NOT drop + reopen the RocksDB handle here: RocksDB's
    /// `LOCK` file is released lazily after all handles drop, and the old
    /// `Surreal<Db>` is an Arc'd handle that SurrealDB keeps alive beyond our
    /// assignment — so reopening at the same path immediately races with the
    /// prior lock and fails with "No locks available". REMOVE DATABASE +
    /// DEFINE DATABASE achieves the same logical empty state without touching
    /// the filesystem and mirrors how the main remote DB is wiped in
    /// `restore::execute_restore_inner`.
    pub async fn reset(&mut self) -> Result<()> {
        self.db
            .query("REMOVE DATABASE IF EXISTS snapshot; DEFINE DATABASE snapshot;")
            .await
            .context("Failed to wipe replica database")?;
        self.db
            .use_db("snapshot")
            .await
            .context("Failed to re-select replica database after wipe")?;
        self.snapshot_seq = 0;
        info!(path = ?self.db_path, "Replica reset (REMOVE DATABASE)");
        Ok(())
    }

    /// Re-read `snapshot_seq` from the embedded metadata table. Useful after
    /// importing a dump — the imported `_00_metadata:snapshot` row carries the
    /// seq from the time of backup.
    pub async fn reload_snapshot_seq(&mut self) -> Result<u64> {
        let seq = Self::read_snapshot_seq_from_db(&self.db).await.unwrap_or(0);
        self.snapshot_seq = seq;
        Ok(seq)
    }

    /// Run an arbitrary SurrealQL query against the snapshot DB
    /// Returns the raw JSON response (used by the HTTP proxy)
    pub async fn query(&self, surql: &str) -> Result<Value> {
        let mut response = self.db
            .query(surql)
            .await
            .with_context(|| format!("Failed to execute query: {}", surql))?;

        // Try to take the first result set
        let result: Vec<Value> = response.take(0).unwrap_or_default();
        Ok(Value::Array(result))
    }

    /// Serialize all records for SSP bootstrap (chunked)
    pub async fn iter_chunks(&self, chunk_size: usize) -> Result<Vec<ReplicaChunk>> {
        // Discover tables
        let mut response = self.db
            .query("INFO FOR DB")
            .await
            .context("Failed to query INFO FOR DB on replica")?;

        let info: Vec<Value> = response.take(0).unwrap_or_default();
        let info = info.into_iter().next().unwrap_or_default();

        let tables: Vec<String> = match info.get("tables") {
            Some(Value::Object(tables_map)) => tables_map
                .keys()
                .filter(|name| !name.starts_with("_00_"))
                .cloned()
                .collect(),
            _ => vec!["thread".to_string(), "job".to_string(), "user".to_string()],
        };

        let mut chunks = Vec::new();
        let mut chunk_index = 0;

        for table_name in tables {
            let mut response = self.db
                .query(format!("SELECT * FROM {}", table_name))
                .await
                .with_context(|| format!("Failed to select from replica table '{}'", table_name))?;

            let records: Vec<Value> = response.take(0).unwrap_or_default();
            let mut current_chunk = Vec::new();

            for record in records {
                let id = record.get("id")
                    .map(|v| v.to_string().trim_matches('"').to_string())
                    .unwrap_or_default();
                current_chunk.push((id, record));

                if current_chunk.len() >= chunk_size {
                    chunks.push(ReplicaChunk {
                        chunk_index,
                        table: table_name.clone(),
                        records: std::mem::take(&mut current_chunk),
                    });
                    chunk_index += 1;
                }
            }

            if !current_chunk.is_empty() {
                chunks.push(ReplicaChunk {
                    chunk_index,
                    table: table_name,
                    records: current_chunk,
                });
                chunk_index += 1;
            }
        }

        Ok(chunks)
    }

    /// Get total record count across all tables
    pub async fn record_count(&self) -> Result<usize> {
        let tables = vec!["thread", "job", "user", "comment"];
        let mut total = 0;

        for table_name in tables {
            let mut response = self.db
                .query(format!("SELECT count() as total FROM {} GROUP ALL", table_name))
                .await?;

            let rows: Vec<Value> = response.take(0).unwrap_or_default();
            if let Some(row) = rows.first() {
                if let Some(count) = row.get("total").and_then(|v| v.as_u64()) {
                    total += count as usize;
                }
            }
        }

        Ok(total)
    }

    /// Get number of tables
    pub fn table_count(&self) -> usize {
        4
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn insert_thread(db: &Surreal<surrealdb::engine::local::Db>, title: &str) -> Result<()> {
        db.query(format!("CREATE thread SET title = '{}'", title))
            .await?;
        Ok(())
    }

    async fn count_threads(db: &Surreal<surrealdb::engine::local::Db>) -> Result<usize> {
        let mut resp = db.query("SELECT count() FROM thread GROUP ALL").await?;
        let rows: Vec<Value> = resp.take(0).unwrap_or_default();
        Ok(rows
            .first()
            .and_then(|r| r.get("count"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize)
    }

    /// Reset must wipe data in place without tripping RocksDB's file lock, and
    /// the handle must stay usable. This would have caught the original bug
    /// (dropping + reopening at the same path failed with "No locks available").
    #[tokio::test]
    async fn reset_wipes_data_and_stays_usable() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let mut replica = Replica::new(tmp.path().join("replica")).await?;

        insert_thread(&replica.db, "hello").await?;
        assert_eq!(count_threads(&replica.db).await?, 1);
        replica.set_snapshot_seq(42).await?;

        replica.reset().await?;

        assert_eq!(replica.snapshot_seq(), 0);
        assert_eq!(replica.reload_snapshot_seq().await?, 0);
        assert_eq!(count_threads(&replica.db).await?, 0);

        insert_thread(&replica.db, "world").await?;
        assert_eq!(count_threads(&replica.db).await?, 1);

        replica.reset().await?;
        assert_eq!(count_threads(&replica.db).await?, 0);

        Ok(())
    }

    /// Full backup-restore shape: export → reset → import on a different path.
    #[tokio::test]
    async fn reset_then_import_round_trips_data() -> Result<()> {
        let src_tmp = tempfile::tempdir()?;
        let src = Replica::new(src_tmp.path().join("src")).await?;
        insert_thread(&src.db, "hello").await?;

        let dump = src_tmp.path().join("dump.surql");
        src.export_to_file(&dump).await?;

        let dst_tmp = tempfile::tempdir()?;
        let mut dst = Replica::new(dst_tmp.path().join("dst")).await?;
        insert_thread(&dst.db, "stale").await?;
        assert_eq!(count_threads(&dst.db).await?, 1);

        dst.reset().await?;
        assert_eq!(count_threads(&dst.db).await?, 0);

        dst.import_from_file(&dump).await?;

        let mut resp = dst.db.query("SELECT title FROM thread").await?;
        let rows: Vec<Value> = resp.take(0).unwrap_or_default();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].get("title").and_then(|v| v.as_str()), Some("hello"));

        Ok(())
    }
}
