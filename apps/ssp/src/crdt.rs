//! Server-side CRDT merge for `_00_crdt` columns.
//!
//! Clients POST incremental Loro update bytes to `/crdt/apply`. The server holds an
//! LRU cache of `LoroDoc`s keyed by `(record_id, field)`, hydrates from SurrealDB on
//! miss, imports the update natively, exports a fresh snapshot, and writes it back to
//! the record's `_00_crdt[<field>]` column. The resulting record `UPDATE` flows
//! through the existing event/sync pipeline to all subscribed clients.

use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use loro::{ExportMode, LoroDoc};
use lru::LruCache;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use surrealdb::types::RecordId;
use tokio::sync::Mutex;
use tracing::{debug, instrument, warn};

use crate::SharedDb;

// ---------- Public types ----------

#[derive(Deserialize, Debug)]
pub struct ApplyRequest {
    pub table: String,
    pub record_id: String,
    pub field: String,
    /// Base64-encoded Loro update bytes (output of `doc.export({ mode: 'update', from: ... })`).
    pub update: String,
    /// Loro peer id of the originating client (string form of u64). Stored in the
    /// column so other clients can fast-path echo suppression.
    pub peer: String,
}

#[derive(Serialize, Debug)]
pub struct ApplyResponse {
    pub rev: u64,
}

// ---------- Allow-list ----------

/// Map of `table -> set of CRDT-annotated field names`. Reads from `SPKY_CRDT_FIELDS`
/// env var as JSON `{"thread":["title","content"], ...}`. If unset, every `(table,
/// field)` is allowed (dev mode); a warning is logged once.
#[derive(Debug, Clone, Default)]
pub struct CrdtAllowList {
    by_table: HashMap<String, std::collections::HashSet<String>>,
    permissive: bool,
}

impl CrdtAllowList {
    pub fn from_env() -> Self {
        match std::env::var("SPKY_CRDT_FIELDS") {
            Ok(s) if !s.is_empty() => match serde_json::from_str::<HashMap<String, Vec<String>>>(&s) {
                Ok(map) => {
                    let by_table = map
                        .into_iter()
                        .map(|(t, fs)| (t, fs.into_iter().collect()))
                        .collect();
                    Self { by_table, permissive: false }
                }
                Err(e) => {
                    warn!(error = %e, "SPKY_CRDT_FIELDS is not valid JSON, falling back to permissive mode");
                    Self { by_table: HashMap::new(), permissive: true }
                }
            },
            _ => {
                warn!("SPKY_CRDT_FIELDS not set, /crdt/apply running in permissive mode");
                Self { by_table: HashMap::new(), permissive: true }
            }
        }
    }

    pub fn allows(&self, table: &str, field: &str) -> bool {
        if self.permissive {
            return true;
        }
        self.by_table.get(table).map_or(false, |s| s.contains(field))
    }
}

// ---------- Cache ----------

type DocSlot = Arc<Mutex<LoroDoc>>;

/// LRU of in-memory `LoroDoc`s keyed by `(record_id, field)`. Each slot is its own
/// `Arc<Mutex>` so the LRU lock is held only briefly during lookup.
pub struct CrdtCache {
    inner: Mutex<LruCache<(String, String), DocSlot>>,
    allow: CrdtAllowList,
}

impl CrdtCache {
    pub fn new(capacity: usize, allow: CrdtAllowList) -> Self {
        let cap = NonZeroUsize::new(capacity.max(1)).unwrap();
        Self {
            inner: Mutex::new(LruCache::new(cap)),
            allow,
        }
    }

    /// Fetch-or-hydrate the doc for `(record_id, field)`. Hydration reads the
    /// existing snapshot out of the record's `_00_crdt[<field>].snapshot` and imports
    /// it into a fresh `LoroDoc`.
    async fn get_or_hydrate(
        &self,
        db: &SharedDb,
        record_id: &str,
        field: &str,
    ) -> Result<DocSlot> {
        let key = (record_id.to_string(), field.to_string());
        if let Some(slot) = self.inner.lock().await.get(&key).cloned() {
            return Ok(slot);
        }

        let snapshot_b64 = read_field_snapshot(db, record_id, field).await?;
        let doc = LoroDoc::new();
        if let Some(b64) = snapshot_b64 {
            let bytes = B64
                .decode(b64.as_bytes())
                .context("failed to decode hydration snapshot")?;
            doc.import(&bytes)
                .map_err(|e| anyhow!("loro import on hydrate failed: {e:?}"))?;
        }

        let slot: DocSlot = Arc::new(Mutex::new(doc));
        self.inner.lock().await.put(key, slot.clone());
        Ok(slot)
    }

    /// Apply an update and persist the resulting snapshot. Returns the new `rev`.
    #[instrument(skip(self, db, req), fields(table = %req.table, record_id = %req.record_id, field = %req.field))]
    pub async fn apply(&self, db: &SharedDb, req: &ApplyRequest) -> Result<ApplyResponse> {
        if !self.allow.allows(&req.table, &req.field) {
            bail!("field '{}.{}' is not in the CRDT allow-list", req.table, req.field);
        }

        let update_bytes = B64
            .decode(req.update.as_bytes())
            .context("failed to decode update bytes")?;

        let slot = self.get_or_hydrate(db, &req.record_id, &req.field).await?;
        let snapshot_bytes = {
            let doc = slot.lock().await;
            doc.import(&update_bytes)
                .map_err(|e| anyhow!("loro import failed: {e:?}"))?;
            doc.export(ExportMode::Snapshot)
                .map_err(|e| anyhow!("loro export failed: {e:?}"))?
        };

        let snapshot_b64 = B64.encode(&snapshot_bytes);
        let rev = write_field_snapshot(db, &req.record_id, &req.field, &snapshot_b64, &req.peer)
            .await?;

        debug!(rev, snapshot_bytes = snapshot_bytes.len(), "crdt apply persisted");
        Ok(ApplyResponse { rev })
    }
}

// ---------- SurrealDB I/O ----------

fn parse_record_id(id: &str) -> Result<RecordId> {
    RecordId::parse_simple(id).map_err(|e| anyhow!("invalid record id '{id}': {e}"))
}

/// Read `_00_crdt[<field>].snapshot` from the record. `Ok(None)` if the column or
/// field entry is missing.
async fn read_field_snapshot(
    db: &SharedDb,
    record_id: &str,
    field: &str,
) -> Result<Option<String>> {
    let id = parse_record_id(record_id)?;
    let mut response = db
        .query("SELECT VALUE _00_crdt FROM ONLY $id")
        .bind(("id", id))
        .await
        .context("read _00_crdt failed")?;
    let v: Option<Value> = response.take(0).context("decode _00_crdt failed")?;
    Ok(v.and_then(|crdt| {
        crdt.get(field)
            .and_then(|f| f.get("snapshot"))
            .and_then(|s| s.as_str())
            .map(|s| s.to_string())
    }))
}

/// Read-modify-write the `_00_crdt` column with the new field state. Returns the new
/// `rev`. NOTE: not atomic across concurrent writes for *different* fields on the same
/// record — see plan §"Cross-SSP concurrency" risk. Acceptable for PR1.
async fn write_field_snapshot(
    db: &SharedDb,
    record_id: &str,
    field: &str,
    snapshot_b64: &str,
    peer: &str,
) -> Result<u64> {
    let id = parse_record_id(record_id)?;

    let mut response = db
        .query("SELECT VALUE _00_crdt FROM ONLY $id")
        .bind(("id", id.clone()))
        .await
        .context("read _00_crdt for write failed")?;
    let existing: Option<Value> = response.take(0).context("decode _00_crdt failed")?;

    let prev_rev = existing
        .as_ref()
        .and_then(|c| c.get(field))
        .and_then(|f| f.get("rev"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let new_rev = prev_rev + 1;

    let mut crdt = match existing {
        Some(Value::Object(map)) => Value::Object(map),
        _ => json!({}),
    };
    crdt[field] = json!({
        "snapshot": snapshot_b64,
        "rev": new_rev,
        "lastPeer": peer,
    });

    db.query("UPDATE $id SET _00_crdt = $crdt")
        .bind(("id", id))
        .bind(("crdt", crdt))
        .await
        .context("UPDATE _00_crdt failed")?;

    Ok(new_rev)
}
