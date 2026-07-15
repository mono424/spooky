//! Per-user dedicated table helpers for `_00_query` / `_00_list_ref`.
//!
//! The SurrealDB v3 LIVE-permission path silently filters notifications when
//! the inserter and the LIVE-subscriber are different sessions — even when the
//! subscriber's permission expression would pass for the new row. As a
//! workaround the SSP can be configured (via `SPKY_SSP_REF_MODE=dedicated`) to
//! use a per-user pair of tables `_00_query_user_<id>` and
//! `_00_list_ref_user_<id>` whose permission rule is hardcoded against a
//! literal user record id.
//!
//! Ported onto the [`Db`] port (was `apps/ssp/src/tables.rs`): DDL runs through
//! `db.query`, and errors already surface as `DbError` (the SDK's `.check()`
//! semantics fold into `SurrealSdkDb::query`). `INFO FOR DB` parsing uses the
//! port's flattened-JSON convention (`{"tables": {name: "DEFINE ..."}}`).

use anyhow::{Context, Result};
use serde_json::json;
use std::collections::HashSet;
use tracing::warn;

use ssp_protocol::{list_ref_table_for, sanitize_user_id, RefMode, ANON_AUTH_ID};

use crate::ports::Db;

/// Returns the `_00_list_ref` table name for the given mode + user.
pub fn list_ref_table(mode: RefMode, auth_id: &str) -> String {
    list_ref_table_for(mode, auth_id)
}

/// Field DDL shared by the anon and per-user `_00_list_ref_*` tables.
fn list_ref_fields(tbl: &str) -> String {
    format!(
        r#"
DEFINE FIELD OVERWRITE in ON TABLE {tbl} TYPE record;
DEFINE FIELD OVERWRITE out ON TABLE {tbl} TYPE record;
DEFINE FIELD OVERWRITE clientId ON TABLE {tbl} TYPE string;
DEFINE FIELD OVERWRITE auth_id ON TABLE {tbl} TYPE string DEFAULT '';
DEFINE FIELD OVERWRITE version ON TABLE {tbl} TYPE int;
DEFINE FIELD OVERWRITE updatedAt ON TABLE {tbl} TYPE datetime VALUE time::now() READONLY;
DEFINE FIELD OVERWRITE parent ON TABLE {tbl} TYPE option<record<{tbl}>>;
DEFINE FIELD OVERWRITE parent_rel ON TABLE {tbl} TYPE option<string>;
"#,
        tbl = tbl,
    )
}

/// Define the shared `_00_list_ref_anon` table used for unauthenticated
/// clients when `anonymousLiveQueries` is enabled. Unlike the per-user tables
/// it is `SELECT WHERE true` (world-readable): a logged-out session has
/// `$auth = NONE`, so it can't be gated on `$auth.id`. Safe because the edges
/// only reference records the client could already read one-shot (the public
/// tables are `select WHERE true`). Writes stay SSP-only (root bypasses the
/// rule). Independent of `RefMode` and never auto-dropped.
pub async fn ensure_anon_table(db: &dyn Db) -> Result<()> {
    let tbl = "_00_list_ref_anon";
    let ddl = format!(
        "DEFINE TABLE OVERWRITE {tbl} SCHEMALESS \
         PERMISSIONS FOR select WHERE true, \
                     FOR create, update WHERE false, \
                     FOR delete WHERE false;{fields}",
        tbl = tbl,
        fields = list_ref_fields(tbl),
    );
    db.query(&ddl, &[])
        .await
        .context("Failed to ensure anonymous list_ref table")?;
    Ok(())
}

/// Issue idempotent `DEFINE TABLE OVERWRITE` + field DDL for the per-user
/// `_00_list_ref_user_<id>` table. No-op in `RefMode::Single`, and when
/// `auth_id` doesn't sanitize cleanly (falls back to the global table, logged
/// at the call site). Idempotent via `OVERWRITE`.
pub async fn ensure_user_tables(db: &dyn Db, mode: RefMode, auth_id: &str) -> Result<()> {
    // Anonymous clients share the dedicated `_00_list_ref_anon` table in both
    // ref modes (the handler only ever passes the sentinel when the flag is on).
    if auth_id == ANON_AUTH_ID {
        return ensure_anon_table(db).await;
    }
    if mode == RefMode::Single {
        return Ok(());
    }
    let Some(uid) = sanitize_user_id(auth_id) else {
        warn!(
            target: "ssp::tables",
            auth_id = %auth_id,
            "auth_id is not a valid table-name segment; dedicated tables skipped, falling back to global"
        );
        return Ok(());
    };

    let tbl = format!("_00_list_ref_user_{}", uid);
    let ddl = format!(
        "DEFINE TABLE OVERWRITE {tbl} SCHEMALESS \
         PERMISSIONS FOR select WHERE $auth.id = user:{uid}, \
                     FOR create, update WHERE false, \
                     FOR delete WHERE false;{fields}",
        tbl = tbl,
        uid = uid,
        fields = list_ref_fields(&tbl),
    );
    db.query(&ddl, &[])
        .await
        .with_context(|| format!("Failed to ensure per-user list_ref table for {}", auth_id))?;
    Ok(())
}

/// Inverse of `ensure_user_tables`: remove the per-user table when the owning
/// `user` record is deleted. No-op in `RefMode::Single` / when `auth_id`
/// doesn't sanitize. Idempotent (`REMOVE TABLE IF EXISTS`).
pub async fn drop_user_tables(db: &dyn Db, mode: RefMode, auth_id: &str) -> Result<()> {
    if mode == RefMode::Single {
        return Ok(());
    }
    let Some(uid) = sanitize_user_id(auth_id) else {
        return Ok(());
    };
    let ddl = format!("REMOVE TABLE IF EXISTS _00_list_ref_user_{};", uid);
    db.query(&ddl, &[])
        .await
        .with_context(|| format!("Failed to drop per-user list_ref table for {}", auth_id))?;
    Ok(())
}

/// Drop a user's per-user table iff they have no remaining registered query in
/// `_00_query`. Called after a TTL sweep. No-op in `RefMode::Single`.
pub async fn drop_user_table_if_unused(db: &dyn Db, mode: RefMode, auth_id: &str) -> Result<()> {
    if mode == RefMode::Single {
        return Ok(());
    }
    let results = db
        .query(
            "SELECT VALUE count() FROM _00_query WHERE auth_id = $a GROUP ALL",
            &[("a", json!(auth_id))],
        )
        .await
        .with_context(|| format!("count remaining queries for {}", auth_id))?;
    let count = results
        .first()
        .and_then(first_i64)
        .unwrap_or(0);
    if count == 0 {
        drop_user_tables(db, mode, auth_id).await?;
    }
    Ok(())
}

/// Best-effort: drop every `_00_list_ref_user_<id>` table whose owner has no
/// live query in `_00_query`. No-op in `RefMode::Single`.
pub async fn drop_orphaned_user_tables(db: &dyn Db, mode: RefMode) -> Result<()> {
    if mode == RefMode::Single {
        return Ok(());
    }

    // Flattened JSON: INFO FOR DB → { "tables": { name: "DEFINE ..." } }.
    let info = db.query("INFO FOR DB", &[]).await.context("INFO FOR DB")?;
    let per_user: Vec<String> = info
        .first()
        .and_then(|v| v.get("tables"))
        .and_then(|t| t.as_object())
        .map(|m| {
            m.keys()
                .filter(|n| n.starts_with("_00_list_ref_user_"))
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    if per_user.is_empty() {
        return Ok(());
    }

    // Sanitized uids that still back a live query.
    let auth_rows = db
        .query("SELECT VALUE auth_id FROM _00_query", &[])
        .await
        .context("select live auth_ids")?;
    let live_uids: HashSet<String> = auth_rows
        .first()
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .filter_map(sanitize_user_id)
                .collect()
        })
        .unwrap_or_default();

    for tbl in per_user {
        let uid = tbl.trim_start_matches("_00_list_ref_user_");
        if !live_uids.contains(uid) {
            if let Err(e) = db.query(&format!("REMOVE TABLE IF EXISTS {tbl};"), &[]).await {
                warn!(target: "ssp::tables", table = %tbl, error = %e, "drop orphaned per-user table failed");
            }
        }
    }
    Ok(())
}

/// Extract a leading integer from a flattened query result (`GROUP ALL count()`
/// returns `[count]` or a bare number depending on shape).
fn first_i64(v: &serde_json::Value) -> Option<i64> {
    match v {
        serde_json::Value::Array(a) => a.first().and_then(|x| x.as_i64()),
        other => other.as_i64(),
    }
}
