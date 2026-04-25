//! `spky verify` — compare per-table record counts across the upstream
//! SurrealDB, the scheduler's replica snapshot, and the SSP's circuit store.
//!
//! Used to confirm that after `spky dev` (especially after `spky dev --clean`)
//! the SSP has a complete snapshot of the data the user expects.

use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use crate::backend::{self, DEFAULT_CONFIG_PATH};
use crate::dev::{SCHEDULER_PORT, SSP_PORT, SURREAL_PORT};

const HTTP_TIMEOUT: Duration = Duration::from_secs(10);
const PREFIX: &str = "[sp00ky verify]";

pub fn run() -> Result<()> {
    let config = backend::load_config(Path::new(DEFAULT_CONFIG_PATH));
    let surreal = config.resolved_surrealdb();

    println!("{} Comparing record counts across SurrealDB, scheduler, and SSP...\n", PREFIX);

    let main = fetch_main_counts(&surreal.namespace, &surreal.database, &surreal.username, &surreal.password)
        .context("Failed to fetch counts from SurrealDB")?;
    let replica = fetch_scheduler_counts().unwrap_or_else(|e| {
        eprintln!("{} Warning: scheduler unavailable: {:#}", PREFIX, e);
        BTreeMap::new()
    });
    let circuit = fetch_ssp_counts().unwrap_or_else(|e| {
        eprintln!("{} Warning: SSP unavailable: {:#}", PREFIX, e);
        BTreeMap::new()
    });

    print_table(&main, &replica, &circuit);

    let mismatches = count_mismatches(&main, &replica, &circuit);
    println!();
    if mismatches == 0 {
        println!("{} OK — all sources agree.", PREFIX);
        Ok(())
    } else {
        bail!("{} {} table(s) out of sync", PREFIX, mismatches);
    }
}

/// Counts per non-`_00_` table in the upstream SurrealDB. Source of truth.
fn fetch_main_counts(ns: &str, db: &str, user: &str, pass: &str) -> Result<BTreeMap<String, usize>> {
    let url = format!("http://localhost:{}/sql", SURREAL_PORT);
    let info: Value = surreal_query(&url, ns, db, user, pass, "INFO FOR DB")?;
    let tables = extract_tables(&info)?;

    let mut out = BTreeMap::new();
    for table in tables {
        // GROUP ALL returns [] when the table is empty, so guard with unwrap_or(0).
        let q = format!("SELECT count() AS total FROM {} GROUP ALL", table);
        let res: Value = surreal_query(&url, ns, db, user, pass, &q)?;
        let count = res.as_array()
            .and_then(|arr| arr.first())
            .and_then(|row| row.get("total"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        out.insert(table, count);
    }
    Ok(out)
}

fn extract_tables(info: &Value) -> Result<Vec<String>> {
    let tables_obj = info.get("tables")
        .and_then(|v| v.as_object())
        .context("INFO FOR DB response missing `tables`")?;
    Ok(tables_obj.keys()
        .filter(|name| !name.starts_with("_00_"))
        .cloned()
        .collect())
}

/// Wrap a single SurrealQL statement, post it to /sql, and return the first
/// statement's result payload.
fn surreal_query(url: &str, ns: &str, db: &str, user: &str, pass: &str, sql: &str) -> Result<Value> {
    let resp = ureq::post(url)
        .set("Surreal-NS", ns)
        .set("Surreal-DB", db)
        .set("Accept", "application/json")
        .set("Content-Type", "text/plain")
        .set("Authorization", &format!("Basic {}", base64_encode(&format!("{}:{}", user, pass))))
        .timeout(HTTP_TIMEOUT)
        .send_string(sql)
        .map_err(|e| anyhow::anyhow!("SurrealDB HTTP error: {}", e))?;
    let body: Vec<Value> = resp.into_json().context("Failed to parse SurrealDB response")?;
    let first = body.into_iter().next()
        .context("SurrealDB returned no statement results")?;
    if first.get("status").and_then(|v| v.as_str()) != Some("OK") {
        bail!("SurrealDB query failed: {}", first);
    }
    Ok(first.get("result").cloned().unwrap_or(Value::Null))
}

/// Minimal base64 (basic-auth header) — we don't pull in the `base64` crate
/// just for this one call.
fn base64_encode(input: &str) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = input.as_bytes();
    let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            if chunk.len() > 1 { chunk[1] } else { 0 },
            if chunk.len() > 2 { chunk[2] } else { 0 },
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | (b[2] as u32);
        out.push(TABLE[((n >> 18) & 0x3f) as usize] as char);
        out.push(TABLE[((n >> 12) & 0x3f) as usize] as char);
        out.push(if chunk.len() > 1 { TABLE[((n >> 6) & 0x3f) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[(n & 0x3f) as usize] as char } else { '=' });
    }
    out
}

/// Per-table counts from the scheduler's `/health/snapshot` endpoint.
fn fetch_scheduler_counts() -> Result<BTreeMap<String, usize>> {
    let url = format!("http://localhost:{}/health/snapshot", SCHEDULER_PORT);
    let resp = ureq::get(&url).timeout(HTTP_TIMEOUT).call()
        .map_err(|e| anyhow::anyhow!("Scheduler HTTP error: {}", e))?;
    let body: Value = resp.into_json().context("Failed to parse scheduler response")?;
    let tables = body.get("tables")
        .and_then(|v| v.as_object())
        .context("Scheduler /health/snapshot missing `tables`")?;
    Ok(tables.iter()
        .filter_map(|(k, v)| v.as_u64().map(|n| (k.clone(), n as usize)))
        .collect())
}

/// Per-table counts from SSP `/info`'s `circuit_tables` field.
fn fetch_ssp_counts() -> Result<BTreeMap<String, usize>> {
    let url = format!("http://localhost:{}/info", SSP_PORT);
    let resp = ureq::get(&url).timeout(HTTP_TIMEOUT).call()
        .map_err(|e| anyhow::anyhow!("SSP HTTP error: {}", e))?;
    let body: Value = resp.into_json().context("Failed to parse SSP response")?;
    let entities = body.as_array().context("SSP /info should return an array")?;
    let ssp_entry = entities.iter()
        .find(|e| e.get("entity").and_then(|v| v.as_str()) == Some("ssp"))
        .context("SSP /info has no ssp entity")?;
    let tables = ssp_entry.get("circuit_tables")
        .and_then(|v| v.as_object())
        .context("SSP /info missing circuit_tables (rebuild the SSP image)")?;
    Ok(tables.iter()
        .filter_map(|(k, v)| v.as_u64().map(|n| (k.clone(), n as usize)))
        .collect())
}

fn count_mismatches(
    main: &BTreeMap<String, usize>,
    replica: &BTreeMap<String, usize>,
    circuit: &BTreeMap<String, usize>,
) -> usize {
    let all_tables: std::collections::BTreeSet<&String> = main.keys()
        .chain(replica.keys())
        .chain(circuit.keys())
        .collect();
    all_tables.into_iter()
        .filter(|t| {
            let m = main.get(*t).copied();
            let r = replica.get(*t).copied();
            let c = circuit.get(*t).copied();
            !(m == r && r == c)
        })
        .count()
}

fn print_table(
    main: &BTreeMap<String, usize>,
    replica: &BTreeMap<String, usize>,
    circuit: &BTreeMap<String, usize>,
) {
    let all_tables: std::collections::BTreeSet<&String> = main.keys()
        .chain(replica.keys())
        .chain(circuit.keys())
        .collect();

    let table_w = all_tables.iter().map(|t| t.len()).max().unwrap_or(8).max(8);
    println!("  {:<width$}  {:>8}  {:>8}  {:>8}  status",
        "table", "main", "replica", "circuit", width = table_w);
    println!("  {:-<width$}  {:->8}  {:->8}  {:->8}  ------",
        "", "", "", "", width = table_w);

    for table in &all_tables {
        let m = main.get(*table).copied();
        let r = replica.get(*table).copied();
        let c = circuit.get(*table).copied();
        let status = if m == r && r == c {
            "OK"
        } else {
            "MISMATCH"
        };
        println!("  {:<width$}  {:>8}  {:>8}  {:>8}  {}",
            table,
            fmt_count(m),
            fmt_count(r),
            fmt_count(c),
            status,
            width = table_w);
    }
}

fn fmt_count(c: Option<usize>) -> String {
    match c {
        Some(n) => n.to_string(),
        None => "-".to_string(),
    }
}
