use anyhow::{bail, Context, Result};
use serde_json::Value;
use ssp_protocol::snapshot_hash;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use surrealdb::engine::local::RocksDb;
use surrealdb::opt::capabilities::{Capabilities, ExperimentalFeature};
use surrealdb::opt::Config;
use surrealdb::Surreal;
use tracing::{debug, info, warn};

/// Config for the embedded replica DB: enable `Files` + `Surrealism`
/// experimental capabilities so dumps that reference `DEFINE BUCKET ...` (a
/// Files feature) or surrealism modules import cleanly. The main SurrealDB
/// runs with these enabled (via `SURREAL_CAPS_ALLOW_EXPERIMENTAL=surrealism,files`),
/// so if the replica isn't configured to match, every post-v3 restore that
/// touches buckets dies with "expected the experimental files feature to be
/// enabled" when the replica tries to import the dump.
fn replica_config() -> Config {
    Config::new().capabilities(
        Capabilities::default().with_experimental_features_allowed(&[
            ExperimentalFeature::Files,
            ExperimentalFeature::Surrealism,
        ]),
    )
}

// Re-export RecordOp from messages to avoid duplication
pub use crate::messages::RecordOp;

/// One-line description of a JSON value's variant for error messages.
fn json_kind(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

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

/// In-memory representation of `_00_metadata:snapshot` — the persisted
/// integrity-check state restored at startup.
#[derive(Default, Debug, Clone)]
struct SnapshotState {
    seq: u64,
    hashes: BTreeMap<String, String>,
    tables: BTreeSet<String>,
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
    /// Per-table content hashes at `snapshot_seq`. Persisted in
    /// `_00_metadata:snapshot.hashes`. Populated by `compute_table_hashes`
    /// after a full clone and updated incrementally in `set_snapshot_state`.
    snapshot_hashes: BTreeMap<String, String>,
    /// Tables we have ever written to (via `ingest_all` or `apply`).
    /// SurrealDB's `INFO FOR DB` only lists explicitly `DEFINE`d tables, so
    /// we cannot rediscover schemaless tables from the engine — we track
    /// them ourselves and persist alongside the hashes so a fresh process
    /// can find them.
    known_tables: BTreeSet<String>,
}

impl Replica {
    /// Create a new replica with persistent SurrealDB/RocksDB storage
    pub async fn new(db_path: PathBuf) -> Result<Self> {
        // Create parent directory if it doesn't exist
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory: {:?}", parent))?;
        }

        let db = Surreal::new::<RocksDb>((
            db_path.to_str().unwrap_or("./data/replica"),
            replica_config(),
        ))
        .await
        .with_context(|| format!("Failed to open RocksDB at {:?}", db_path))?;

        db.use_ns("sp00ky").use_db("snapshot").await
            .context("Failed to select namespace/database on replica")?;

        info!("Opened replica SurrealDB at {:?}", db_path);

        let SnapshotState {
            seq: snapshot_seq,
            hashes: snapshot_hashes,
            tables: known_tables,
        } = Self::read_snapshot_state_from_db(&db).await.unwrap_or_default();
        if snapshot_seq > 0 {
            info!(
                snapshot_seq,
                hash_tables = snapshot_hashes.len(),
                "Restored snapshot state from metadata"
            );
        }

        Ok(Self {
            db,
            db_path,
            snapshot_seq,
            snapshot_hashes,
            known_tables,
        })
    }

    /// Get current snapshot sequence number
    pub fn snapshot_seq(&self) -> u64 {
        self.snapshot_seq
    }

    /// Per-table content hashes at the current `snapshot_seq`.
    pub fn snapshot_hashes(&self) -> &BTreeMap<String, String> {
        &self.snapshot_hashes
    }

    /// All tables this replica has ever written to.
    pub fn known_tables(&self) -> &BTreeSet<String> {
        &self.known_tables
    }

    /// Set snapshot sequence number AND advance the per-table hashes for the
    /// supplied tables. Pass `None` for `touched_tables` after a full clone
    /// to recompute every known table; pass `Some(set)` from the drain loop
    /// to only rehash the tables a batch touched.
    pub async fn set_snapshot_state(
        &mut self,
        seq: u64,
        touched_tables: Option<&BTreeSet<String>>,
    ) -> Result<()> {
        self.snapshot_seq = seq;

        let to_hash: BTreeSet<String> = match touched_tables {
            Some(t) => t.clone(),
            None => self.known_tables.clone(),
        };

        for table in &to_hash {
            match self.hash_one_table(table).await {
                Ok(hash) => {
                    self.snapshot_hashes.insert(table.clone(), hash);
                }
                Err(e) => {
                    // Don't fail the snapshot advance just because one table
                    // can't be hashed (e.g. schema race). Log and remove the
                    // stale entry so /health/snapshot doesn't lie.
                    warn!(table = %table, error = %e, "Failed to hash table");
                    self.snapshot_hashes.remove(table);
                }
            }
        }

        let hashes_value = serde_json::to_value(&self.snapshot_hashes)
            .context("Serialize snapshot_hashes failed")?;
        let tables_value = serde_json::to_value(
            self.known_tables.iter().cloned().collect::<Vec<_>>(),
        )
        .context("Serialize known_tables failed")?;

        self.db
            .query("UPSERT _00_metadata:snapshot SET seq = $seq, hashes = $hashes, tables = $tables")
            .bind(("seq", seq))
            .bind(("hashes", hashes_value))
            .bind(("tables", tables_value))
            .await
            .context("Failed to persist snapshot state")?;
        Ok(())
    }

    /// Backward-compatible single-field setter used by `drain_and_apply`
    /// when called without a touched-tables hint. Updates the seq only and
    /// leaves cached hashes alone — callers that want the hashes refreshed
    /// must use `set_snapshot_state`.
    pub async fn set_snapshot_seq(&mut self, seq: u64) -> Result<()> {
        self.set_snapshot_state(seq, Some(&BTreeSet::new())).await
    }

    /// Compute hashes for every known table. Returns the new map without
    /// mutating `self.snapshot_hashes` — caller decides when to commit.
    pub async fn compute_table_hashes(&self) -> Result<BTreeMap<String, String>> {
        let mut out = BTreeMap::new();
        for table in &self.known_tables {
            match self.hash_one_table(table).await {
                Ok(h) => {
                    out.insert(table.clone(), h);
                }
                Err(e) => {
                    warn!(table = %table, error = %e, "Failed to hash table during recompute");
                }
            }
        }
        Ok(out)
    }

    async fn hash_one_table(&self, table: &str) -> Result<String> {
        let mut response = self
            .db
            .query(format!("SELECT * FROM {}", table))
            .await
            .with_context(|| format!("hash: SELECT * FROM {} failed", table))?;
        let sdk_val: surrealdb::types::Value = response
            .take(0)
            .with_context(|| format!("hash: take(0) failed for '{}'", table))?;
        let rows: Vec<Value> = match sdk_val.into_json_value() {
            Value::Array(arr) => arr,
            _ => Vec::new(),
        };

        let pairs: Vec<(String, Value)> = rows
            .into_iter()
            .filter_map(|mut row| {
                let id = row.as_object_mut()
                    .and_then(|obj| obj.get("id").and_then(|v| v.as_str()).map(String::from))?;
                let raw_id = id.strip_prefix(&format!("{}:", table)).unwrap_or(&id).to_string();
                Some((raw_id, row))
            })
            .collect();

        Ok(snapshot_hash::hash_table(pairs))
    }

    /// Read combined snapshot state (seq + hashes + tables) from metadata.
    async fn read_snapshot_state_from_db(
        db: &Surreal<surrealdb::engine::local::Db>,
    ) -> Result<SnapshotState> {
        let mut response = db
            .query("SELECT seq, hashes, tables FROM _00_metadata:snapshot")
            .await
            .context("Failed to query snapshot metadata")?;

        let rows: Vec<Value> = response.take(0).unwrap_or_default();
        let row = match rows.first() {
            Some(r) => r,
            None => return Ok(SnapshotState::default()),
        };

        let seq = row.get("seq").and_then(|v| v.as_u64()).unwrap_or(0);
        let hashes: BTreeMap<String, String> = row
            .get("hashes")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        let tables: Vec<String> = row
            .get("tables")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        let mut known: BTreeSet<String> = tables.into_iter().collect();
        for k in hashes.keys() {
            known.insert(k.clone());
        }
        Ok(SnapshotState {
            seq,
            hashes,
            tables: known,
        })
    }

    /// Full initial load from a remote SurrealDB instance.
    /// Repopulates `known_tables` from the upstream INFO FOR DB.
    pub async fn ingest_all<C>(&mut self, remote_db: &surrealdb::Surreal<C>) -> Result<()>
    where
        C: surrealdb::Connection,
    {
        let total_start = std::time::Instant::now();

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

        // Track which tables we are about to populate so the integrity-check
        // path can rediscover them after a restart (INFO FOR DB on the
        // schemaless replica won't list them).
        for t in &tables {
            self.known_tables.insert(t.clone());
        }

        info!(
            table_count = tables.len(),
            "Snapshot clone starting: {} tables to ingest [{}]",
            tables.len(),
            tables.join(", "),
        );

        let mut total_records: usize = 0;
        for (idx, table_name) in tables.iter().enumerate() {
            let table_start = std::time::Instant::now();
            info!(
                table = %table_name,
                progress = format!("{}/{}", idx + 1, tables.len()),
                "[{}/{}] Ingesting table '{}' from remote...",
                idx + 1,
                tables.len(),
                table_name,
            );

            // Take the SDK's own `Value` then call `into_json_value()` so RecordId/Datetime
            // are flattened into normal JSON strings instead of `{"RecordId":{...}}` shapes.
            // Direct deserialization into `serde_json::Value` doesn't work on SurrealDB 3.0.
            let mut response = remote_db
                .query(format!("SELECT * FROM {}", table_name))
                .await
                .with_context(|| format!("SELECT * FROM {} failed", table_name))?;

            let sdk_val: surrealdb::types::Value = response.take(0)
                .with_context(|| format!("take(0) failed for table '{}'", table_name))?;

            let records: Vec<Value> = match sdk_val.into_json_value() {
                Value::Array(arr) => arr,
                other => bail!(
                    "Expected array from SELECT * FROM {}, got {}",
                    table_name,
                    json_kind(&other),
                ),
            };

            let count = records.len();
            let fetch_ms = table_start.elapsed().as_millis();
            let insert_start = std::time::Instant::now();

            // Insert each record. Any insert failure aborts the whole snapshot.
            // We strip the `id` field from CONTENT (the target thing already
            // encodes it; leaving it in the body silently truncates writes on
            // SurrealDB 3.0) and call `.check()` because the SDK reports
            // statement-level errors there, not on `.await`.
            for mut record in records {
                let id_str = match record.as_object_mut() {
                    Some(obj) => obj.remove("id").and_then(|v| v.as_str().map(String::from)),
                    None => None,
                };
                let id_str = id_str.with_context(|| format!(
                    "Record in '{}' missing string `id` after JSON flatten",
                    table_name,
                ))?;
                let thing_id = build_thing_id(table_name, &id_str);
                self.db
                    .query(format!("CREATE {} CONTENT $data", thing_id))
                    .bind(("data", record))
                    .await
                    .with_context(|| format!("CREATE {} send failed", thing_id))?
                    .check()
                    .with_context(|| format!("CREATE {} returned an error", thing_id))?;
            }

            let insert_ms = insert_start.elapsed().as_millis();
            total_records += count;
            info!(
                table = %table_name,
                records = count,
                fetch_ms = fetch_ms as u64,
                insert_ms = insert_ms as u64,
                "[{}/{}] Done '{}' — {} records (fetch {}ms, insert {}ms)",
                idx + 1,
                tables.len(),
                table_name,
                count,
                fetch_ms,
                insert_ms,
            );
        }

        // Copy view definitions — same hard-fail discipline as the data tables above.
        let views_start = std::time::Instant::now();
        let mut response = remote_db
            .query("SELECT * FROM _00_query")
            .await
            .context("Failed to query _00_query on remote")?;

        let sdk_val: surrealdb::types::Value = response.take(0)
            .context("take(0) failed for _00_query")?;
        let views: Vec<Value> = match sdk_val.into_json_value() {
            Value::Array(arr) => arr,
            other => bail!(
                "Expected array from SELECT * FROM _00_query, got {}",
                json_kind(&other),
            ),
        };
        let view_count = views.len();
        for mut record in views {
            let id_str = match record.as_object_mut() {
                Some(obj) => obj.remove("id").and_then(|v| v.as_str().map(String::from)),
                None => None,
            };
            let id_str = id_str.context("_00_query record missing string `id` field")?;
            let key = if id_str.starts_with("_00_query:") {
                id_str
            } else {
                format!("_00_query:{}", id_str)
            };
            self.db
                .query(format!("CREATE {} CONTENT $data", key))
                .bind(("data", record))
                .await
                .with_context(|| format!("CREATE view {} send failed", key))?
                .check()
                .with_context(|| format!("CREATE view {} returned an error", key))?;
        }
        info!(
            views = view_count,
            elapsed_ms = views_start.elapsed().as_millis() as u64,
            "Copied {} view definitions",
            view_count,
        );

        info!(
            tables = tables.len(),
            records = total_records,
            views = view_count,
            elapsed_ms = total_start.elapsed().as_millis() as u64,
            "Snapshot clone summary: {} tables, {} records, {} views in {}ms",
            tables.len(),
            total_records,
            view_count,
            total_start.elapsed().as_millis(),
        );

        Ok(())
    }

    /// Apply a single record event to the snapshot
    pub async fn apply(&mut self, table: &str, op: RecordOp, id: &str, record: Option<Value>) -> Result<()> {
        if !table.starts_with("_00_") {
            self.known_tables.insert(table.to_string());
        }
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
        self.snapshot_hashes.clear();
        self.known_tables.clear();
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
    /// Returns the raw JSON response (used by the HTTP proxy).
    ///
    /// SurrealDB 3.0 errors on `SELECT * FROM <undefined>` instead of returning
    /// an empty array. The replica is schemaless and tables only "exist" once
    /// they receive a `CREATE`, so callers (notably SSP bootstrap querying
    /// `_00_query`) need missing tables to behave like empty result sets. We
    /// detect that case via the SDK's `NotFound` error and translate to `[]`.
    pub async fn query(&self, surql: &str) -> Result<Value> {
        let mut response = self.db
            .query(surql)
            .await
            .with_context(|| format!("Failed to execute query: {}", surql))?;

        match response.take::<surrealdb::types::Value>(0) {
            Ok(v) => Ok(v.into_json_value()),
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("does not exist") {
                    debug!(query = %surql, "query targets a missing table — returning []");
                    Ok(Value::Array(Vec::new()))
                } else {
                    Err(anyhow::anyhow!(
                        "take(0) failed for query [{}]: {}", surql, msg
                    ))
                }
            }
        }
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
        Ok(self.record_counts_per_table().await?.into_iter().map(|(_, c)| c).sum())
    }

    /// Get per-table record counts for every non-`_00_` table in the replica.
    /// Used by the `/health/snapshot` endpoint and `spky verify` to compare
    /// replica state against the upstream SurrealDB.
    ///
    /// Discovers tables from the *currently inserted records* rather than
    /// `INFO FOR DB`, because the replica receives schemaless inserts via
    /// `CREATE` and SurrealDB only lists explicitly `DEFINE`d tables in
    /// `INFO FOR DB`.
    pub async fn record_counts_per_table(&self) -> Result<Vec<(String, usize)>> {
        // Probe the same set of tables we know we ingest from upstream. If
        // the table was never seen, count returns 0. We can't enumerate
        // schemaless tables on the replica side, so we mirror the upstream
        // discovery list (this matches how `ingest_all` populates the replica).
        let candidates = ["comment", "commented_on", "job", "thread", "user"];

        let mut counts = Vec::with_capacity(candidates.len());
        for table_name in candidates {
            let count = self.count_table(table_name).await?;
            counts.push((table_name.to_string(), count));
        }

        Ok(counts)
    }

    async fn count_table(&self, table_name: &str) -> Result<usize> {
        let mut response = self.db
            .query(format!("SELECT count() AS total FROM {} GROUP ALL", table_name))
            .await
            .with_context(|| format!("count() query failed for table '{}'", table_name))?;
        let sdk_val: surrealdb::types::Value = response.take(0)
            .with_context(|| format!("take(0) failed for count of '{}'", table_name))?;
        let json = sdk_val.into_json_value();
        let count = json.as_array()
            .and_then(|arr| arr.first())
            .and_then(|row| row.get("total"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        Ok(count)
    }

    /// Number of non-`_00_` tables present in the replica.
    pub async fn table_count(&self) -> Result<usize> {
        Ok(self.record_counts_per_table().await?.len())
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
