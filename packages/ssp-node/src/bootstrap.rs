//! Standalone circuit rebuild from the database, over the [`Db`] port.
//!
//! Moved from the VM shell's `self_bootstrap_with_metadata` (the Direct path).
//! The cluster/proxy path stays in `apps/ssp` (it reads rows from the
//! scheduler's HTTP proxy, not the DB). This is the load [`crate::Runtime::bootstrap`]
//! runs on a cold start when the `CircuitStore` has no usable snapshot.

use std::sync::Arc;

use anyhow::Context;
use serde_json::{json, Value};
use tokio::sync::RwLock;
use tracing::{info, warn};

use ssp::circuit::view::OutputFormat;
use ssp::circuit::{Change, ChangeSet, Circuit, Record};

use crate::ports::Db;

/// Keyset-paginated page query for the bootstrap scan (ordered by id, never
/// OFFSET — lossy under concurrent writes).
pub fn bootstrap_page_query(table: &str, page_size: usize, after_id: Option<&str>) -> String {
    match after_id {
        None => format!("SELECT * FROM {table} ORDER BY id LIMIT {page_size}"),
        Some(id) => {
            let raw = id.strip_prefix(&format!("{table}:")).unwrap_or(id);
            format!("SELECT * FROM {table} WHERE id > type::record('{table}', '{raw}') ORDER BY id LIMIT {page_size}")
        }
    }
}

/// Target table of a `DEFINE FIELD … TYPE record<X>` link (single, simple X).
pub fn parse_link_target(define_field: &str) -> Option<String> {
    let lower = define_field.to_lowercase();
    let rec_idx = lower.find("record<")?;
    let after = &define_field[rec_idx + "record<".len()..];
    let close = after.find('>')?;
    let inner = after[..close].trim();
    if inner.is_empty()
        || inner.contains('|')
        || !inner.chars().all(|c| c.is_alphanumeric() || c == '_')
    {
        return None;
    }
    Some(inner.to_string())
}

/// Pull `PERMISSIONS FOR select WHERE <expr>` text from a `DEFINE TABLE` string
/// (raw text so `prepare_registration_dbsp` routes it through the same
/// converter as user queries). FULL/absent → `"true"`; NONE/no-select → `"false"`.
pub fn extract_select_permission_text(define_table: &str) -> String {
    let def = define_table.trim().trim_end_matches(';');
    let upper = def.to_uppercase();
    let Some(perm_idx) = upper.find("PERMISSIONS") else {
        return "true".into();
    };
    let perm_section = def[perm_idx + "PERMISSIONS".len()..].trim();
    let perm_upper = perm_section.to_uppercase();
    if perm_upper.starts_with("FULL") {
        return "true".into();
    }
    if perm_upper.starts_with("NONE") {
        return "false".into();
    }

    let lower = perm_section.to_lowercase();
    let mut clause_starts: Vec<usize> = Vec::new();
    for (i, _) in lower.match_indices("for ") {
        if i == 0 || lower.as_bytes()[i - 1].is_ascii_whitespace() {
            clause_starts.push(i);
        }
    }
    if clause_starts.is_empty() {
        warn!(target: "ssp::policy", def = %def, "PERMISSIONS clause has no FOR clauses; denying");
        return "false".into();
    }
    for (idx, &start) in clause_starts.iter().enumerate() {
        let end = clause_starts.get(idx + 1).copied().unwrap_or(perm_section.len());
        let clause = &perm_section[start..end];
        let lower_clause = clause.to_lowercase();
        let where_idx = lower_clause.find("where");
        let header = match where_idx {
            Some(w) => &clause[..w],
            None => clause,
        };
        if !header.to_lowercase().contains("select") {
            continue;
        }
        let Some(w) = where_idx else {
            return "true".into();
        };
        let body = clause[w + "where".len()..]
            .trim()
            .trim_end_matches(',')
            .trim_end_matches(';')
            .trim()
            .to_string();
        if body.is_empty() {
            return "true".into();
        }
        return body;
    }
    "false".into()
}

/// First statement's result as a single flattened `Value`.
async fn q1(db: &dyn Db, surql: &str) -> anyhow::Result<Value> {
    Ok(db
        .query(surql, &[])
        .await
        .with_context(|| format!("query failed: {surql}"))?
        .into_iter()
        .next()
        .unwrap_or(Value::Null))
}

/// Full standalone rebuild: discover tables + permissions + link targets via
/// `INFO FOR DB`/`INFO FOR TABLE`, page every syncable table into the circuit,
/// re-register persisted views from `_00_query`, and reseed catch-up hashes.
/// Everything over the [`Db`] port — no net/fs/env.
pub async fn rebuild_from_db(
    db: &dyn Db,
    processor: &Arc<RwLock<Circuit>>,
    page_size: usize,
) -> anyhow::Result<()> {
    info!("Starting circuit rebuild from DB");

    // 1. Tables + their DEFINE strings (skip _00_* and @nosync tables).
    let info_json = q1(db, "INFO FOR DB").await.context("INFO FOR DB")?;
    let table_defs: Vec<(String, String)> = match info_json.get("tables") {
        Some(Value::Object(tables_map)) => tables_map
            .iter()
            .filter(|(name, _)| !ssp_protocol::table_excluded_from_sync(name))
            .filter(|(name, def)| {
                let nosync =
                    def.as_str().map(ssp_protocol::define_str_is_nosync).unwrap_or(false);
                if nosync {
                    info!(table = %name, "Excluding @nosync table from rebuild");
                }
                !nosync
            })
            .map(|(name, def)| (name.clone(), def.as_str().unwrap_or("").to_string()))
            .collect(),
        _ => vec![],
    };
    let tables: Vec<String> = table_defs.iter().map(|(n, _)| n.clone()).collect();
    info!(count = tables.len(), "Discovered tables: {:?}", tables);

    // 1b. Per-table select-permission text → circuit.
    {
        let mut circuit = processor.write().await;
        for (name, def) in &table_defs {
            let permission = extract_select_permission_text(def);
            info!(target: "ssp::policy", table = %name, permission = %permission, "registered table permission");
            circuit.set_permission(name, permission);
        }
    }

    // 1c. Record-link map (field -> target table) from INFO FOR TABLE.
    {
        let mut resolved: Vec<(String, String, String)> = Vec::new();
        for table in &tables {
            let info = match q1(db, &format!("INFO FOR TABLE {}", table)).await {
                Ok(v) => v,
                Err(e) => {
                    warn!(target: "ssp::policy", table = %table, error = %e, "INFO FOR TABLE failed; skipping link map");
                    continue;
                }
            };
            let Some(fields) = info.get("fields").and_then(|f| f.as_object()) else {
                continue;
            };
            for (field_name, def) in fields {
                if let Some(target) = def.as_str().and_then(parse_link_target) {
                    resolved.push((table.clone(), field_name.clone(), target));
                }
            }
        }
        if !resolved.is_empty() {
            let mut circuit = processor.write().await;
            for (table, field, target) in &resolved {
                circuit.set_link_target(table.clone(), field.clone(), target.clone());
            }
        }
    }

    // 2. Page each table's rows into the circuit store.
    for table in &tables {
        let mut record_count = 0usize;
        let mut after_id: Option<String> = None;
        loop {
            let result = q1(db, &bootstrap_page_query(table, page_size, after_id.as_deref()))
                .await
                .with_context(|| format!("page-query {table}"))?;
            let rows: Vec<Value> = match result {
                Value::Array(arr) => arr,
                _ => vec![],
            };
            let n = rows.len();
            if n == 0 {
                break;
            }
            let next_after = rows
                .last()
                .and_then(|row| row.get("id"))
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let records: Vec<Record> = rows
                .into_iter()
                .filter_map(|row| {
                    let id = row.get("id")?.as_str()?.to_string();
                    Some(Record::new(table, &id, row))
                })
                .collect();
            processor.write().await.load(records);
            record_count += n;
            if n < page_size {
                break;
            }
            match next_after {
                Some(id) => after_id = Some(id),
                None => break,
            }
        }
        info!(table = %table, records = record_count, "Loaded table data");
    }

    // 3. Re-register persisted views from _00_query. A missing table (virgin
    //    DB — e.g. a fresh ephemeral host) means "no persisted views", not a
    //    bootstrap failure.
    let views = match q1(db, "SELECT * FROM _00_query").await {
        Ok(Value::Array(arr)) => arr,
        Ok(_) => vec![],
        Err(e) => {
            warn!(error = %e, "read _00_query failed (no persisted views?) — skipping view re-registration");
            vec![]
        }
    };
    info!(count = views.len(), "Found persisted views in _00_query");
    for view_row in views {
        let view_id = match view_row.get("id") {
            Some(Value::String(s)) => s.clone(),
            Some(v) => v.to_string().trim_matches('"').to_string(),
            None => continue,
        };
        let raw_id = view_id.strip_prefix("_00_query:").unwrap_or(&view_id).to_string();
        let Some(surql) = view_row.get("surql").and_then(|v| v.as_str()) else {
            warn!(view_id = %raw_id, "Skipping view with missing surql");
            continue;
        };
        let get = |k: &str| view_row.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
        let auth_id = get("auth_id");
        let payload = json!({
            "id": raw_id,
            "surql": surql,
            "clientId": get("clientId"),
            "authId": auth_id,
            "ttl": view_row.get("ttl").and_then(|v| v.as_str()).unwrap_or("30m"),
            "lastActiveAt": get("lastActiveAt"),
            "params": view_row.get("params").cloned().unwrap_or(json!({})),
        });
        let prep = {
            let circuit = processor.read().await;
            ssp::service::view::prepare_registration_dbsp(
                payload,
                circuit.permissions(),
                circuit.link_targets(),
            )
        };
        match prep {
            Ok(data) => {
                let mut circuit = processor.write().await;
                circuit.add_query_with_auth(
                    data.plan,
                    data.safe_params,
                    Some(OutputFormat::Streaming),
                    auth_id.clone(),
                );
                info!(view_id = %raw_id, auth_id = %auth_id, "Re-registered view");
            }
            Err(e) => warn!(target: "ssp::policy", view_id = %raw_id, error = %e, "Failed to re-register view"),
        }
    }

    // Seed catch-up XOR accumulators from the bulk-loaded rows (bypassed by
    // `Circuit::load`), before any replay/ingest.
    processor.write().await.reseed_catchup_hashes();
    Ok(())
}

/// Incremental catch-up after a snapshot restore: for each table load rows
/// whose `_00_rv` is newer than the snapshot's `max_row_version`, then reseed.
/// A table without a tracked `max_row_version` catches up from `-1` (every row
/// carrying an `_00_rv`). Tables/rows without `_00_rv` are not caught here —
/// the staleness gate + the 503-during-bootstrap window bound that risk, and a
/// too-old snapshot rebuilds in full instead (see [`crate::Runtime::bootstrap`]).
pub async fn catch_up_from_db(
    db: &dyn Db,
    processor: &Arc<RwLock<Circuit>>,
    point: &crate::ports::ResumePoint,
) -> anyhow::Result<()> {
    // Discover current syncable tables (metadata may have changed since the
    // snapshot; refresh permissions/links cheaply too).
    let info_json = q1(db, "INFO FOR DB").await.context("INFO FOR DB (catch-up)")?;
    let table_defs: Vec<(String, String)> = match info_json.get("tables") {
        Some(Value::Object(m)) => m
            .iter()
            .filter(|(name, _)| !ssp_protocol::table_excluded_from_sync(name))
            .filter(|(_, def)| !def.as_str().map(ssp_protocol::define_str_is_nosync).unwrap_or(false))
            .map(|(n, d)| (n.clone(), d.as_str().unwrap_or("").to_string()))
            .collect(),
        _ => vec![],
    };
    {
        let mut circuit = processor.write().await;
        for (name, def) in &table_defs {
            circuit.set_permission(name, extract_select_permission_text(def));
        }
    }
    // Link targets are also dropped by `Circuit::restore` — re-seed them.
    for (table, _) in &table_defs {
        if let Ok(info) = q1(db, &format!("INFO FOR TABLE {}", table)).await {
            if let Some(fields) = info.get("fields").and_then(|f| f.as_object()) {
                let mut circuit = processor.write().await;
                for (field_name, def) in fields {
                    if let Some(target) = def.as_str().and_then(parse_link_target) {
                        circuit.set_link_target(table.clone(), field_name.clone(), target);
                    }
                }
            }
        }
    }

    for (table, _) in &table_defs {
        let since = point.max_row_version.get(table).copied().unwrap_or(-1);
        let q = format!("SELECT * FROM {table} WHERE _00_rv > {since}");
        let rows: Vec<Value> = match q1(db, &q).await? {
            Value::Array(arr) => arr,
            _ => vec![],
        };
        if rows.is_empty() {
            continue;
        }
        let n = rows.len();
        // Replay through `step` (not bulk `load`) so the RESTORED views update:
        // a row absent from the snapshot is a Create (adds membership), one
        // already present is an Update (content-only, membership unchanged).
        let mut circuit = processor.write().await;
        let changes: Vec<Change> = rows
            .into_iter()
            .filter_map(|row| {
                let id = row.get("id")?.as_str()?.to_string();
                Some(if circuit.contains(table, &id) {
                    Change::update(table, &id, row)
                } else {
                    Change::create(table, &id, row)
                })
            })
            .collect();
        circuit.step(ChangeSet { changes });
        info!(table = %table, caught_up = n, since_rv = since, "Catch-up stepped rows");
    }

    processor.write().await.reseed_catchup_hashes();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_query_uses_ordered_keyset_not_offset() {
        assert_eq!(bootstrap_page_query("game", 200, None), "SELECT * FROM game ORDER BY id LIMIT 200");
        let next = bootstrap_page_query("game", 200, Some("game:abc"));
        assert_eq!(next, "SELECT * FROM game WHERE id > type::record('game', 'abc') ORDER BY id LIMIT 200");
        assert!(!next.contains("START"));
    }

    #[test]
    fn permission_extraction() {
        assert_eq!(extract_select_permission_text("DEFINE TABLE t"), "true");
        assert_eq!(extract_select_permission_text("DEFINE TABLE t PERMISSIONS NONE"), "false");
        assert_eq!(extract_select_permission_text("DEFINE TABLE t PERMISSIONS FULL"), "true");
        assert_eq!(
            extract_select_permission_text("DEFINE TABLE t PERMISSIONS FOR select WHERE user = $auth.id"),
            "user = $auth.id"
        );
        assert_eq!(extract_select_permission_text("DEFINE TABLE t PERMISSIONS FOR select"), "true");
        assert_eq!(extract_select_permission_text("DEFINE TABLE t PERMISSIONS FOR update WHERE true"), "false");
    }

    #[test]
    fn link_target_parse() {
        assert_eq!(parse_link_target("DEFINE FIELD owner ON t TYPE record<user>"), Some("user".into()));
        assert_eq!(parse_link_target("DEFINE FIELD x ON t TYPE record<a | b>"), None);
        assert_eq!(parse_link_target("DEFINE FIELD x ON t TYPE string"), None);
    }
}
