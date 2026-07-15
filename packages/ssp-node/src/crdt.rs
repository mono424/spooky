//! Server-side CRDT merge for `_00_crdt` columns (ported from
//! `apps/ssp/src/crdt.rs` onto the [`Db`] port).
//!
//! Clients POST incremental Loro update bytes to `/crdt/apply`. The server
//! holds an LRU cache of `LoroDoc`s keyed by `(record_id, field)`, hydrates
//! from SurrealDB on miss, imports the update natively, exports a fresh
//! snapshot, and writes it back to the record's `_00_crdt[<field>]` column.
//! The resulting record `UPDATE` flows through the event/sync pipeline to all
//! subscribed clients.
//!
//! Port change: record ids cross the JSON `Db` port as `type::record($tb,
//! $key)` string binds instead of `surrealdb::RecordId` params.

use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use loro::{ExportMode, LoroDoc};
use lru::LruCache;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::Mutex;
use tracing::debug;

use crate::ports::Db;

// ---------- Public types ----------

#[derive(Deserialize, Debug)]
pub struct ApplyRequest {
    pub table: String,
    pub record_id: String,
    pub field: String,
    /// Base64-encoded Loro update bytes.
    pub update: String,
    /// Loro peer id of the originating client (string form of u64).
    pub peer: String,
}

#[derive(Serialize, Debug)]
pub struct ApplyResponse {
    pub rev: u64,
}

// ---------- Allow-list ----------

/// Map of `table -> set of CRDT-annotated field names`. Built by the shell
/// from `SPKY_CRDT_FIELDS` (the core never reads env). Permissive = allow every
/// `(table, field)` (dev mode).
#[derive(Debug, Clone, Default)]
pub struct CrdtAllowList {
    by_table: HashMap<String, std::collections::HashSet<String>>,
    permissive: bool,
}

impl CrdtAllowList {
    /// Permissive allow-list (every field allowed).
    pub fn permissive() -> Self {
        Self { by_table: HashMap::new(), permissive: true }
    }

    /// Restrictive allow-list from a `{table: [fields]}` map.
    pub fn from_map(map: HashMap<String, Vec<String>>) -> Self {
        let by_table = map
            .into_iter()
            .map(|(t, fs)| (t, fs.into_iter().collect()))
            .collect();
        Self { by_table, permissive: false }
    }

    pub fn allows(&self, table: &str, field: &str) -> bool {
        if self.permissive {
            return true;
        }
        self.by_table.get(table).is_some_and(|s| s.contains(field))
    }
}

// ---------- Cache ----------

type DocSlot = Arc<Mutex<LoroDoc>>;

/// LRU of in-memory `LoroDoc`s keyed by `(record_id, field)`. Each slot is its
/// own `Arc<Mutex>` so the LRU lock is held only briefly during lookup.
pub struct CrdtCache {
    inner: Mutex<LruCache<(String, String), DocSlot>>,
    allow: CrdtAllowList,
}

impl CrdtCache {
    pub fn new(capacity: usize, allow: CrdtAllowList) -> Self {
        let cap = NonZeroUsize::new(capacity.max(1)).unwrap();
        Self { inner: Mutex::new(LruCache::new(cap)), allow }
    }

    async fn get_or_hydrate(&self, db: &dyn Db, record_id: &str, field: &str) -> Result<DocSlot> {
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
    pub async fn apply(&self, db: &dyn Db, req: &ApplyRequest) -> Result<ApplyResponse> {
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
        let rev =
            write_field_snapshot(db, &req.record_id, &req.field, &snapshot_b64, &req.peer).await?;

        debug!(rev, snapshot_bytes = snapshot_bytes.len(), "crdt apply persisted");
        Ok(ApplyResponse { rev })
    }
}

// ---------- SurrealDB I/O (via the Db port) ----------

/// Validate a `table:key` record id before binding it.
fn valid_record_id(id: &str) -> Result<()> {
    match id.split_once(':') {
        Some((t, k)) if !t.is_empty() && !k.is_empty() => Ok(()),
        _ => Err(anyhow!("invalid record id '{id}'")),
    }
}

/// Read `_00_crdt[<field>].snapshot` from the record. `Ok(None)` if the column
/// or field entry is missing. `type::record($id)` (single-arg) parses the full
/// `table:key` string faithfully — preserving numeric vs string key type,
/// which the old `RecordId::parse_simple` bind did.
async fn read_field_snapshot(db: &dyn Db, record_id: &str, field: &str) -> Result<Option<String>> {
    valid_record_id(record_id)?;
    let results = db
        .query(
            "SELECT VALUE _00_crdt FROM ONLY type::record($id)",
            &[("id", json!(record_id))],
        )
        .await
        .context("read _00_crdt failed")?;
    Ok(results.into_iter().next().and_then(|crdt| {
        crdt.get(field)
            .and_then(|f| f.get("snapshot"))
            .and_then(|s| s.as_str())
            .map(|s| s.to_string())
    }))
}

/// Read-modify-write the `_00_crdt` column with the new field state. Returns
/// the new `rev`. NOT atomic across concurrent writes for *different* fields on
/// the same record (documented pre-existing limitation).
async fn write_field_snapshot(
    db: &dyn Db,
    record_id: &str,
    field: &str,
    snapshot_b64: &str,
    peer: &str,
) -> Result<u64> {
    valid_record_id(record_id)?;

    let results = db
        .query(
            "SELECT VALUE _00_crdt FROM ONLY type::record($id)",
            &[("id", json!(record_id))],
        )
        .await
        .context("read _00_crdt for write failed")?;
    let existing = results.into_iter().next();

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

    db.query(
        "UPDATE type::record($id) SET _00_crdt = $crdt",
        &[("id", json!(record_id)), ("crdt", crdt)],
    )
    .await
    .context("UPDATE _00_crdt failed")?;

    Ok(new_rev)
}
