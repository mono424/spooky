use anyhow::{bail, Context, Result};
use serde_json::Value;
use ssp_protocol::snapshot_hash;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use surrealdb::engine::local::RocksDb;
use surrealdb::opt::capabilities::{Capabilities, ExperimentalFeature};
use surrealdb::opt::Config;
use surrealdb::Surreal;
use tracing::{debug, info, trace, warn};

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

/// True if an error from SurrealDB indicates a missing namespace, database,
/// or table. Used to translate "upstream isn't initialized yet" into an empty
/// result so bootstrap can run against a brand-new SurrealDB.
pub(crate) fn is_missing_error<E: std::fmt::Display>(e: &E) -> bool {
    let msg = e.to_string();
    msg.contains("does not exist")
        || msg.contains("Table not found")
        || msg.contains("not found in this database")
        || msg.contains("The namespace")
        || msg.contains("The database")
}

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

/// Build one page of a bootstrap table scan using KEYSET pagination on the
/// record `id` (`WHERE id > <last>`), never `OFFSET`/`START`.
///
/// The remote DB is live while we page it, so offset pagination is unsafe: a
/// concurrent delete behind the offset shifts every later row up one, so the
/// next `START n` skips a row. A skipped record never lands in the replica (nor
/// in any SSP that bootstraps from it), so a later delete of that record emits
/// no removal delta and clients' live queries go stale until reload. Keyset
/// resumes from the last id seen, immune to shifts behind the cursor. Mirrors
/// `bootstrap_page_query` in apps/ssp/src/lib.rs.
fn keyset_page_query(table: &str, page_size: usize, after_id: Option<&str>) -> String {
    match after_id {
        None => format!("SELECT * FROM {table} ORDER BY id LIMIT {page_size}"),
        Some(id) => format!("SELECT * FROM {table} WHERE id > {id} ORDER BY id LIMIT {page_size}"),
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

    /// Read all rows of `table` from the replica as `(raw_id, value)` pairs, the
    /// id stripped of its `table:` prefix to match the SSP circuit's raw keys.
    /// Read-only; used both by `hash_one_table` and to seed the scheduler's
    /// catch-up projection when verifying a rejoining SSP.
    pub async fn snapshot_rows(&self, table: &str) -> Result<Vec<(String, Value)>> {
        let mut response = self
            .db
            .query(format!("SELECT * FROM {}", table))
            .await
            .with_context(|| format!("snapshot_rows: SELECT * FROM {} failed", table))?;
        let sdk_val: surrealdb::types::Value = response
            .take(0)
            .with_context(|| format!("snapshot_rows: take(0) failed for '{}'", table))?;
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

        Ok(pairs)
    }

    async fn hash_one_table(&self, table: &str) -> Result<String> {
        Ok(snapshot_hash::hash_table(self.snapshot_rows(table).await?))
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

        // Discover tables from remote. Tolerate "database doesn't exist yet"
        // by treating it as an empty table list — the scheduler may legitimately
        // be pointed at a fresh SurrealDB where neither the user schema nor
        // Phase 4a has run.
        trace!("remote query: INFO FOR DB");
        let info: Value = match remote_db.query("INFO FOR DB").await {
            Ok(mut response) => {
                let v: Vec<Value> = response.take(0).unwrap_or_default();
                v.into_iter().next().unwrap_or_default()
            }
            Err(e) if is_missing_error(&e) => {
                debug!("INFO FOR DB on missing database — treating as empty");
                Value::Null
            }
            Err(e) => return Err(anyhow::Error::from(e).context("Failed to query INFO FOR DB on remote")),
        };

        // If the upstream has no `tables` block, ingest nothing. (Previously a
        // hardcoded `[thread, job, user]` fallback lived here — actively wrong
        // for any project not happening to use those exact names.)
        //
        // Tables marked `-- @nosync` carry a `COMMENT 'sp00ky:nosync'` marker in
        // their `DEFINE TABLE` string (baked in by the CLI). They are excluded
        // from the snapshot entirely — never cloned, never added to
        // `known_tables`, never hashed. They remain in the main DB (still
        // backed up); they just don't participate in sync.
        let tables: Vec<String> = match info.get("tables") {
            Some(Value::Object(tables_map)) => tables_map
                .iter()
                .filter(|(name, _)| !name.starts_with("_00_"))
                .filter(|(name, def)| {
                    let nosync = def
                        .as_str()
                        .map(ssp_protocol::define_str_is_nosync)
                        .unwrap_or(false);
                    if nosync {
                        info!(table = %name, "Excluding @nosync table from snapshot");
                    }
                    !nosync
                })
                .map(|(name, _)| name.clone())
                .collect(),
            _ => Vec::new(),
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

            // Page the SELECT instead of pulling the whole table in one shot.
            // The SurrealDB Rust SDK's WebSocket engine inherits tungstenite's
            // default `max_message_size` of 64 MiB. A SELECT response that
            // exceeds that fails the entire query, which historically caused
            // bootstrap to stall on tables past ~60 MiB. Paging reads the
            // table as a sequence of bounded responses; we then concatenate
            // the pages into the same `Vec<Value>` the rest of the loop
            // expects.
            //
            // Page size is chosen adaptively: a one-row probe measures actual
            // serialised row size, then we pick a page count that targets
            // ~32 MiB per response (half the WS frame ceiling, comfortable
            // headroom). `SPKY_BOOTSTRAP_PAGE_SIZE` lets the operator override
            // the result.
            //
            // Take the SDK's own `Value` then call `into_json_value()` so
            // RecordId/Datetime are flattened into normal JSON strings instead
            // of `{"RecordId":{...}}` shapes. Direct deserialization into
            // `serde_json::Value` doesn't work on SurrealDB 3.0. Tolerate the
            // table disappearing between INFO FOR DB and SELECT (race) or
            // simply not existing yet — treat as zero records and move on.
            let target_page_bytes: usize = 32 * 1024 * 1024;
            let probe_row_bytes: Option<usize> = match remote_db
                .query(format!("SELECT * FROM {} LIMIT 1", table_name))
                .await
            {
                Ok(mut r) => match r.take::<surrealdb::types::Value>(0) {
                    Ok(sdk_val) => match sdk_val.into_json_value() {
                        Value::Array(arr) => arr
                            .first()
                            .map(|v| serde_json::to_vec(v).map(|b| b.len()).unwrap_or(1024)),
                        _ => None,
                    },
                    Err(_) => None,
                },
                Err(_) => None,
            };
            let auto_page_size = probe_row_bytes
                .map(|b| (target_page_bytes / b.max(1)).max(1))
                .unwrap_or(200);
            let page_size: usize = std::env::var("SPKY_BOOTSTRAP_PAGE_SIZE")
                .ok()
                .and_then(|s| s.parse().ok())
                .filter(|n: &usize| *n > 0)
                .unwrap_or(auto_page_size)
                // Hard ceiling to avoid pathological large pages even if
                // the probe misses (e.g. variable row sizes).
                .min(2000);
            if let Some(b) = probe_row_bytes {
                trace!(
                    table = %table_name,
                    probe_bytes_per_row = b,
                    page_size,
                    "bootstrap page-size auto-tuned",
                );
            }
            let mut records: Vec<Value> = Vec::new();
            // Keyset cursor: the highest `id` paged so far (`None` = first page).
            // Offset (`START n`) pagination silently drops rows when a concurrent
            // write shifts the table between page requests — and the remote DB is
            // LIVE during bootstrap — leaving the replica (and every SSP that
            // bootstraps from it) with an incomplete table. An incomplete circuit
            // can't emit a removal for a record it never loaded, so deletes of the
            // dropped rows never reach clients live. Resume by `id > $last`
            // instead: a delete behind the cursor can't shift rows ahead of it out
            // of view. Mirrors `bootstrap_page_query` in apps/ssp.
            let mut after_id: Option<String> = None;
            let table_missing;
            loop {
                let query = keyset_page_query(table_name, page_size, after_id.as_deref());
                trace!(table = %table_name, after_id = ?after_id, page_size, "remote page query: {}", query);
                let resp = remote_db.query(query).await;
                let page: Vec<Value> = match resp {
                    Ok(mut response) => match response.take::<surrealdb::types::Value>(0) {
                        Ok(sdk_val) => match sdk_val.into_json_value() {
                            Value::Array(arr) => arr,
                            other => bail!(
                                "Expected array from paged SELECT on {}, got {}",
                                table_name,
                                json_kind(&other),
                            ),
                        },
                        Err(e) if is_missing_error(&e) => {
                            debug!(table = %table_name, "remote table missing during page take — stopping");
                            table_missing = true;
                            break;
                        }
                        Err(e) => return Err(anyhow::anyhow!(
                            "take(0) failed for table '{}' page (after_id={:?}): {}",
                            table_name, after_id, e,
                        )),
                    },
                    Err(e) if is_missing_error(&e) => {
                        debug!(table = %table_name, "remote table missing during page query — stopping");
                        table_missing = true;
                        break;
                    }
                    Err(e) => return Err(anyhow::Error::from(e)
                        .context(format!(
                            "SELECT page from {} (after_id={:?}, limit={}) failed",
                            table_name, after_id, page_size,
                        ))),
                };
                let n = page.len();
                // Advance the cursor to this page's last id (page is ORDER BY id,
                // so the last row carries the max id) BEFORE consuming `page`.
                let next_after = page
                    .last()
                    .and_then(|row| row.get("id"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                records.extend(page);
                if n < page_size {
                    table_missing = false;
                    break;
                }
                // No usable id to resume from → stop rather than loop forever.
                match next_after {
                    Some(id) => after_id = Some(id),
                    None => break,
                }
            }
            // `table_missing` is captured for parity with the old single-shot
            // path. We don't use it further; the rest of the loop simply
            // proceeds with whatever records we collected (possibly zero).
            let _ = table_missing;

            let count = records.len();
            let fetch_ms = table_start.elapsed().as_millis();
            let insert_start = std::time::Instant::now();

            // Bulk-insert in batches. The previous per-record `CREATE … CONTENT`
            // loop was O(N) round-trips; for tables where each record is large
            // (e.g. tens to hundreds of KiB) the per-call cost in SurrealDB 3.0
            // was non-linear and stalled bootstrap entirely past ~60 MiB total.
            // `INSERT INTO <table> $records` collapses each batch to one
            // round-trip and lets the engine handle the value tree at bulk.
            //
            // We still validate per-record that `id` exists; we let SurrealDB
            // parse the string id (e.g. "comment:42") into the record-id field
            // on insert. `.check()` once per batch is enough because a single
            // bad record fails the whole batch (same outer semantic as the old
            // loop: any insert failure aborts the whole snapshot clone).
            const INSERT_BATCH_SIZE: usize = 500;
            let chunks: Vec<Vec<Value>> = records
                .chunks(INSERT_BATCH_SIZE)
                .map(<[Value]>::to_vec)
                .collect();
            for (chunk_idx, mut chunk) in chunks.into_iter().enumerate() {
                // Validate ids upfront so we can give a specific error on the
                // offending record, matching the old loop's behaviour. Also
                // strip the leading `<table>:` from each id — without it,
                // SurrealDB INSERT INTO treats the colon as part of an
                // escaped composite id and stores the row as
                // `<table>:`<table>:<raw>``, breaking every SELECT-by-id
                // query against the replica.
                let table_prefix = format!("{}:", table_name);
                for (within, rec) in chunk.iter_mut().enumerate() {
                    let obj = match rec.as_object_mut() {
                        Some(o) => o,
                        None => anyhow::bail!(
                            "Record {}/{} in '{}' (batch {}) is not a JSON object",
                            within,
                            chunk_idx,
                            table_name,
                            chunk_idx,
                        ),
                    };
                    let id = obj
                        .get("id")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| anyhow::anyhow!(
                            "Record {}/{} in '{}' (batch {}) missing string `id` after JSON flatten",
                            within,
                            chunk_idx,
                            table_name,
                            chunk_idx,
                        ))?
                        .to_string();
                    let raw = id.strip_prefix(&table_prefix).unwrap_or(&id).to_string();
                    obj.insert("id".to_string(), Value::String(raw));
                }
                let chunk_len = chunk.len();
                trace!(
                    table = %table_name,
                    chunk = chunk_idx,
                    records = chunk_len,
                    "bootstrap INSERT batch"
                );
                self.db
                    .query(format!("INSERT INTO {} $records RETURN NONE", table_name))
                    .bind(("records", Value::Array(chunk)))
                    .await
                    .with_context(|| format!(
                        "INSERT into {} batch {} send failed", table_name, chunk_idx,
                    ))?
                    .check()
                    .with_context(|| format!(
                        "INSERT into {} batch {} returned an error", table_name, chunk_idx,
                    ))?;
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

        // Copy view definitions. Tolerate `_00_query` not existing yet — happens
        // when the scheduler is pointed at a fresh SurrealDB before Phase 4a
        // has applied the internal Sp00ky schema.
        let views_start = std::time::Instant::now();
        trace!("remote query: SELECT * FROM _00_query");
        let views: Vec<Value> = match remote_db.query("SELECT * FROM _00_query").await {
            Ok(mut response) => match response.take::<surrealdb::types::Value>(0) {
                Ok(sdk_val) => match sdk_val.into_json_value() {
                    Value::Array(arr) => arr,
                    other => bail!(
                        "Expected array from SELECT * FROM _00_query, got {}",
                        json_kind(&other),
                    ),
                },
                Err(e) if is_missing_error(&e) => {
                    debug!("_00_query missing on remote — no views to copy");
                    Vec::new()
                }
                Err(e) => return Err(anyhow::anyhow!("take(0) failed for _00_query: {}", e)),
            },
            Err(e) if is_missing_error(&e) => {
                debug!("_00_query missing on remote — no views to copy");
                Vec::new()
            }
            Err(e) => return Err(anyhow::Error::from(e)
                .context("Failed to query _00_query on remote")),
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
                if let Some(mut data) = record {
                    // Strip `id` from the payload before CREATE: SurrealDB
                    // 3.0 takes the record id from the `thing` literal.
                    // Keeping `id` in CONTENT as the string `"user:abc"`
                    // makes SurrealDB treat the colon as part of an
                    // escaped composite id and store it as
                    // `user:`user:abc``. Subsequent SELECTs by the clean
                    // `user:abc` form return nothing — and the SSP's
                    // bootstrap loads the corrupted shape, so live
                    // queries from the client never resolve.
                    if let Some(obj) = data.as_object_mut() {
                        obj.remove("id");
                    }
                    self.db
                        .query(format!("CREATE {} CONTENT $data", thing_id))
                        .bind(("data", data))
                        .await
                        .with_context(|| format!("CREATE {} send failed", thing_id))?
                        .check()
                        .with_context(|| format!("CREATE {} returned a statement error", thing_id))?;
                }
            }
            RecordOp::Update => {
                if let Some(mut data) = record {
                    if let Some(obj) = data.as_object_mut() {
                        obj.remove("id");
                    }
                    self.db
                        .query(format!("UPDATE {} MERGE $data", thing_id))
                        .bind(("data", data))
                        .await
                        .with_context(|| format!("UPDATE {} send failed", thing_id))?
                        .check()
                        .with_context(|| format!("UPDATE {} returned a statement error", thing_id))?;
                }
            }
            RecordOp::Delete => {
                self.db
                    .query(format!("DELETE {}", thing_id))
                    .await
                    .with_context(|| format!("DELETE {} send failed", thing_id))?
                    .check()
                    .with_context(|| format!("DELETE {} returned a statement error", thing_id))?;
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

    /// Re-read snapshot state from the embedded metadata table. Useful after
    /// importing a dump — the imported `_00_metadata:snapshot` row carries the
    /// seq, hashes, and table list from the time of backup.
    pub async fn reload_snapshot_seq(&mut self) -> Result<u64> {
        let state = Self::read_snapshot_state_from_db(&self.db)
            .await
            .unwrap_or_default();
        self.snapshot_seq = state.seq;
        self.snapshot_hashes = state.hashes;
        self.known_tables = state.tables;
        Ok(self.snapshot_seq)
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
        trace!(query = %surql, "local replica query");
        let mut response = self.db
            .query(surql)
            .await
            .with_context(|| format!("Failed to execute query: {}", surql))?;

        match response.take::<surrealdb::types::Value>(0) {
            Ok(v) => Ok(v.into_json_value()),
            Err(e) => {
                if is_missing_error(&e) {
                    debug!(query = %surql, "query targets a missing table — returning []");
                    Ok(Value::Array(Vec::new()))
                } else {
                    Err(anyhow::anyhow!(
                        "take(0) failed for query [{}]: {}", surql, e
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
    /// Discovers tables from `known_tables` (populated by `ingest_all` and
    /// `apply`, persisted in `_00_metadata:snapshot.tables`). SurrealDB
    /// `INFO FOR DB` doesn't list schemaless tables we created via CREATE,
    /// so we cannot rely on the engine for discovery.
    pub async fn record_counts_per_table(&self) -> Result<Vec<(String, usize)>> {
        let mut counts = Vec::with_capacity(self.known_tables.len());
        for table_name in &self.known_tables {
            let count = self.count_table(table_name).await?;
            counts.push((table_name.clone(), count));
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

    #[test]
    fn keyset_page_query_uses_ordered_cursor_not_offset() {
        // Regression guard: replica bootstrap must page by id keyset, never by
        // OFFSET/START (lossy under the concurrent writes a live DB sees while
        // it's being paged). Mirrors the SSP bootstrap fix.
        let first = keyset_page_query("game", 200, None);
        assert_eq!(first, "SELECT * FROM game ORDER BY id LIMIT 200");

        let next = keyset_page_query("game", 200, Some("game:abc"));
        assert_eq!(
            next,
            "SELECT * FROM game WHERE id > game:abc ORDER BY id LIMIT 200"
        );

        assert!(!first.contains("START") && !next.contains("START"));
        assert!(first.contains("ORDER BY id") && next.contains("ORDER BY id"));
    }

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
