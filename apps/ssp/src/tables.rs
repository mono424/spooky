//! Per-user dedicated table helpers for `_00_query` / `_00_list_ref`.
//!
//! The SurrealDB v3 LIVE-permission path silently filters notifications
//! when the inserter and the LIVE-subscriber are different sessions —
//! even when the subscriber's permission expression would pass for the
//! new row. As a workaround the SSP can be configured (via
//! `SPKY_SSP_REF_MODE=dedicated`) to use a per-user pair of tables
//! `_00_query_user_<id>` and `_00_list_ref_user_<id>` whose permission
//! rule is hardcoded against a literal user record id. The client then
//! subscribes to its own dedicated `_00_list_ref_user_<my_user_id>` so
//! no cross-session permission check happens at LIVE-notification time.
//!
//! Sanitization and the canonical table-name function live in
//! `ssp_protocol` so the client mirror and this server-side code can
//! never disagree about which name to use.

use anyhow::{Context, Result};
use ssp_protocol::{list_ref_table_for, sanitize_user_id, RefMode};
use std::collections::HashSet;
use surrealdb::{Connection, Surreal};
use tracing::warn;

/// Returns the `_00_list_ref` table name for the given mode + user.
pub fn list_ref_table(mode: RefMode, auth_id: &str) -> String {
    list_ref_table_for(mode, auth_id)
}

/// Issue idempotent `DEFINE TABLE OVERWRITE` + field DDL for the
/// per-user `_00_list_ref_user_<id>` table. `_00_query` stays in the
/// single global table — the client needs to be able to compute its
/// record id without first knowing the user, and that single
/// registration table is read by `WHERE in = <id>` queries against the
/// per-user list_ref, so the cross-session LIVE-permission gap doesn't
/// affect it.
///
/// The per-user list_ref permission rule pins to the literal user
/// record id, so a record-token caller bearing `$auth.id = user:<id>`
/// matches at SELECT/LIVE time without any session-scoped state.
/// No-op in `RefMode::Single` (the global `_00_list_ref` from the
/// migration is enough). No-op too if `auth_id` doesn't sanitize
/// cleanly — a non-alphanumeric user id can't form a SurrealDB table
/// identifier, so the SSP falls back to writing into the global table
/// for that user and logs a warning at the call site. Idempotent on
/// repeat calls because every DDL statement uses `OVERWRITE`.
pub async fn ensure_user_tables<C: Connection>(
    db: &Surreal<C>,
    mode: RefMode,
    auth_id: &str,
) -> Result<()> {
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

    let list_ref_tbl = format!("_00_list_ref_user_{}", uid);

    let ddl = format!(
        r#"
DEFINE TABLE OVERWRITE {list_ref_tbl} SCHEMALESS
    PERMISSIONS FOR select WHERE $auth.id = user:{uid},
                FOR create, update WHERE false,
                FOR delete WHERE false;
DEFINE FIELD OVERWRITE in ON TABLE {list_ref_tbl} TYPE record;
DEFINE FIELD OVERWRITE out ON TABLE {list_ref_tbl} TYPE record;
DEFINE FIELD OVERWRITE clientId ON TABLE {list_ref_tbl} TYPE string;
DEFINE FIELD OVERWRITE auth_id ON TABLE {list_ref_tbl} TYPE string DEFAULT '';
DEFINE FIELD OVERWRITE version ON TABLE {list_ref_tbl} TYPE int;
DEFINE FIELD OVERWRITE updatedAt ON TABLE {list_ref_tbl} TYPE datetime VALUE time::now() READONLY;
DEFINE FIELD OVERWRITE parent ON TABLE {list_ref_tbl} TYPE option<record<{list_ref_tbl}>>;
DEFINE FIELD OVERWRITE parent_rel ON TABLE {list_ref_tbl} TYPE option<string>;
"#,
        list_ref_tbl = list_ref_tbl,
    );

    db.query(ddl)
        .await
        .with_context(|| format!("Failed to ensure per-user list_ref table for {}", auth_id))?
        .check()
        .with_context(|| format!("ensure_user_tables: at least one DDL statement failed for {}", auth_id))?;

    Ok(())
}

/// Inverse of `ensure_user_tables`: remove the per-user
/// `_00_list_ref_user_<id>` table when the owning `user` record is
/// deleted, so dedicated tables don't accumulate forever. No-op in
/// `RefMode::Single` (no per-user tables exist) and when `auth_id`
/// doesn't sanitize (no dedicated table was ever created either).
/// Idempotent — `REMOVE TABLE IF EXISTS` swallows the missing-table
/// case.
pub async fn drop_user_tables<C: Connection>(
    db: &Surreal<C>,
    mode: RefMode,
    auth_id: &str,
) -> Result<()> {
    if mode == RefMode::Single {
        return Ok(());
    }
    let Some(uid) = sanitize_user_id(auth_id) else {
        return Ok(());
    };

    let list_ref_tbl = format!("_00_list_ref_user_{}", uid);
    let ddl = format!("REMOVE TABLE IF EXISTS {list_ref_tbl};");

    db.query(ddl)
        .await
        .with_context(|| format!("Failed to drop per-user list_ref table for {}", auth_id))?
        .check()
        .with_context(|| format!("drop_user_tables: REMOVE TABLE failed for {}", auth_id))?;

    Ok(())
}

/// Drop a user's per-user `_00_list_ref_user_<id>` table iff they have no
/// remaining registered query in `_00_query`. Called after a TTL sweep cleans a
/// query so a user's table doesn't linger once their last view expires. No-op in
/// `RefMode::Single`.
pub async fn drop_user_table_if_unused<C: Connection>(
    db: &Surreal<C>,
    mode: RefMode,
    auth_id: &str,
) -> Result<()> {
    if mode == RefMode::Single {
        return Ok(());
    }
    let counts: Vec<i64> = db
        .query("SELECT VALUE count() FROM _00_query WHERE auth_id = $a GROUP ALL")
        .bind(("a", auth_id.to_string()))
        .await
        .with_context(|| format!("count remaining queries for {}", auth_id))?
        .take(0)
        .with_context(|| format!("take remaining-query count for {}", auth_id))?;
    if counts.first().copied().unwrap_or(0) == 0 {
        drop_user_tables(db, mode, auth_id).await?;
    }
    Ok(())
}

/// Best-effort: drop every `_00_list_ref_user_<id>` table whose owner has no
/// live query in `_00_query`. Clears leftovers accumulated before TTL cleanup
/// removed expired views (the per-user table was never dropped, only its rows).
/// No-op in `RefMode::Single`.
pub async fn drop_orphaned_user_tables<C: Connection>(db: &Surreal<C>, mode: RefMode) -> Result<()> {
    if mode == RefMode::Single {
        return Ok(());
    }

    // Per-user list_ref tables currently defined. surrealdb v3 `take` only
    // supports its own value types, so go via `surrealdb::types::Value`.
    let info_val: surrealdb::types::Value = db
        .query("INFO FOR DB")
        .await
        .context("INFO FOR DB")?
        .take(0)
        .context("take INFO FOR DB")?;
    let info: serde_json::Value =
        serde_json::to_value(&info_val).unwrap_or(serde_json::Value::Null);
    // surrealdb v3 serializes `Value` externally-tagged, so the table map is at
    // `.Object.tables.Object` (each entry value is `{"String": "DEFINE ..."}`).
    let per_user: Vec<String> = info
        .get("Object")
        .and_then(|o| o.get("tables"))
        .and_then(|t| t.get("Object"))
        .and_then(|o| o.as_object())
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
    let auth_ids: Vec<String> = db
        .query("SELECT VALUE auth_id FROM _00_query")
        .await
        .context("select live auth_ids")?
        .take(0)
        .context("take live auth_ids")?;
    let live_uids: HashSet<String> = auth_ids.iter().filter_map(|a| sanitize_user_id(a)).collect();

    for tbl in per_user {
        let uid = tbl.trim_start_matches("_00_list_ref_user_");
        if !live_uids.contains(uid) {
            if let Err(e) = db.query(format!("REMOVE TABLE IF EXISTS {tbl};")).await {
                warn!(target: "ssp::tables", table = %tbl, error = %e, "drop orphaned per-user table failed");
            }
        }
    }
    Ok(())
}
