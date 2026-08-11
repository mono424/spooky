//! Memory-footprint baseline for the circuit.
//!
//! The SSP holds every syncable table in RAM for the whole process lifetime,
//! so its resident size is what decides whether a tenant fits under its cgroup
//! cap. These tests pin the current cost per row and per registered query so a
//! representation change has to *prove* its reduction, and so a regression that
//! quietly inflates the store fails CI instead of arriving as an OOM kill in
//! production (which leaves no log line — the container is simply SIGKILLed).
//!
//! `#[ignore]`d: they allocate hundreds of MB and take seconds. Run explicitly:
//!
//! ```sh
//! cargo test -p ssp --release --test memory_profile -- --ignored --nocapture
//! ```
//!
//! **These thresholds are meant to be ratcheted DOWN.** Each phase of the
//! zero-copy row-store work should lower them and leave the new number here as
//! its evidence. Never raise one to make a build pass: a raise means the change
//! made the store bigger, which is the thing this file exists to catch.

use ssp::algebra::ZSet;
use ssp::circuit::store::{Change, Record, Store};
use ssp::circuit::Circuit;
use ssp::operator::plan::OrderSpec;
use ssp::operator::{Operator, TopK};
use ssp::types::Path;
use serde_json::json;

/// A row shaped like real application data: a mix of strings (including record
/// references), integers, a bool, a null, and one nested object. Twelve
/// top-level fields, ULID-width ids.
fn synthetic_row(i: usize) -> serde_json::Value {
    let id = format!("{i:026}");
    json!({
        "id": format!("thread:{id}"),
        "title": "a reasonably typical title string",
        "owner": format!("user:{:026}", i % 1000),
        "status": "open",
        "slug": format!("slug-{i}"),
        "body_preview": "some preview text that is a bit longer",
        "count": i as i64,
        "score": (i % 97) as i64,
        "_00_rv": i as i64,
        "pinned": false,
        "archived_at": serde_json::Value::Null,
        "meta": { "a": 1, "b": "two", "c": true },
    })
}

fn load_rows(n: usize) -> (Circuit, usize) {
    let records: Vec<Record> = (0..n)
        .map(|i| Record::new("thread", &format!("{i:026}"), synthetic_row(i)))
        .collect();
    // Size of the same rows as JSON on the wire — the denominator for the
    // blowup ratio, and the number the flat encoding should approach.
    let json_bytes: usize = (0..n)
        .map(|i| serde_json::to_vec(&synthetic_row(i)).unwrap().len())
        .sum();
    let mut circuit = Circuit::new();
    circuit.load(records);
    (circuit, json_bytes)
}

/// Baseline measured 2026-08-11 on the parsed-`Sp00kyValue` store:
/// **~2054 B/row against ~327 B of source JSON, a 6.3x blowup.**
///
/// The threshold sits just above that. Lower it as the representation work
/// lands; the flat-encoding phase should take it under ~450.
#[test]
#[ignore = "allocates ~100MB and takes seconds; run with --ignored"]
fn store_bytes_per_row_stays_under_budget() {
    const ROWS: usize = 50_000;
    const MAX_BYTES_PER_ROW: f64 = 2_200.0;

    let (circuit, json_bytes) = load_rows(ROWS);
    let report = circuit.size_report();
    let table = &report.tables[0];

    let json_per_row = json_bytes as f64 / ROWS as f64;
    let blowup = table.total_bytes() as f64 / json_bytes as f64;

    eprintln!("rows            : {}", table.rows);
    eprintln!("source JSON     : {json_per_row:.0} B/row");
    eprintln!(
        "rows_bytes      : {:.0} B/row",
        table.rows_bytes as f64 / ROWS as f64
    );
    eprintln!(
        "zset_bytes      : {:.0} B/row",
        table.zset_bytes as f64 / ROWS as f64
    );
    eprintln!("total           : {:.0} B/row", table.bytes_per_row());
    eprintln!("blowup vs JSON  : {blowup:.2}x");
    if let Some(rss) = rss_bytes() {
        eprintln!("process RSS     : {} B", rss);
    }

    assert_eq!(table.rows, ROWS, "every row must land in the store");
    assert!(
        table.bytes_per_row() <= MAX_BYTES_PER_ROW,
        "store grew to {:.0} B/row, over the {MAX_BYTES_PER_ROW:.0} B budget. \
         If this is an intended representation change, lower the threshold; \
         if not, the store just got bigger.",
        table.bytes_per_row()
    );
}

/// The `zset` keys are a second `"table:id"` string per row, allocated
/// separately from the raw id already in `rows`. Pinned so the `Arc<str>`
/// interning phase has a number to beat.
#[test]
#[ignore = "allocates ~100MB and takes seconds; run with --ignored"]
fn zset_duplicate_key_cost_is_pinned() {
    const ROWS: usize = 50_000;
    const MAX_ZSET_BYTES_PER_ROW: f64 = 90.0;

    let (circuit, _) = load_rows(ROWS);
    let table = &circuit.size_report().tables[0];
    let per_row = table.zset_bytes as f64 / ROWS as f64;

    eprintln!("zset            : {per_row:.0} B/row");
    assert!(
        per_row <= MAX_ZSET_BYTES_PER_ROW,
        "zset grew to {per_row:.0} B/row, over the {MAX_ZSET_BYTES_PER_ROW:.0} B budget"
    );
}

/// `TopK` keeps every row that reaches it in *both* a sorted buffer and a
/// reverse index, so an `ORDER BY ... LIMIT 20` costs O(table), not O(20) —
/// **per registered query**. With enough views over one big table this term
/// exceeds the row store outright, and it is invisible in the per-table
/// numbers.
///
/// Baseline measured 2026-08-11: **242 B/row/query**, against 2054 B/row for
/// the shared store. So roughly eight windowed views over one table already
/// cost as much as the table itself.
#[test]
#[ignore = "allocates ~100MB and takes seconds; run with --ignored"]
fn topk_state_is_charged_per_query_over_the_whole_table() {
    const ROWS: usize = 50_000;
    const MAX_BYTES_PER_ROW_PER_QUERY: f64 = 400.0;

    let mut store = Store::new();
    store.ensure_collection("thread");
    let mut delta = ZSet::new();
    for i in 0..ROWS {
        let id = format!("{i:026}");
        store.apply_change(&Change::create("thread", &id, synthetic_row(i)));
        delta.insert(format!("thread:{id}"), 1);
    }

    // A window query: 20 rows out of 50k. State should be O(20); it is O(50k).
    let mut top_k = TopK::new(
        20,
        0,
        Some(vec![OrderSpec {
            field: Path::new("score"),
            direction: "DESC".into(),
        }]),
    );
    let out = top_k.step(&[&delta], &store, None);
    assert_eq!(out.len(), 20, "the window itself is small");

    let per_row = top_k.state_bytes() as f64 / ROWS as f64;
    eprintln!("topk state      : {per_row:.0} B/row/query (window is 20 rows)");
    assert!(
        per_row <= MAX_BYTES_PER_ROW_PER_QUERY,
        "TopK state grew to {per_row:.0} B/row/query, over the \
         {MAX_BYTES_PER_ROW_PER_QUERY:.0} B budget"
    );
}

/// Guards the attribution itself: if `size_report` stops seeing a component,
/// every threshold above silently passes for the wrong reason.
#[test]
fn size_report_attributes_every_component() {
    let mut circuit = Circuit::new();
    circuit.load(
        (0..64).map(|i| Record::new("thread", &format!("{i:026}"), synthetic_row(i))),
    );

    let report = circuit.size_report();
    assert_eq!(report.tables.len(), 1);
    let table = &report.tables[0];
    assert_eq!(table.table, "thread");
    assert_eq!(table.rows, 64);
    assert!(table.rows_bytes > 0, "row bodies must be counted");
    assert!(table.zset_bytes > 0, "the membership zset must be counted");
    assert_eq!(table.total_bytes(), table.rows_bytes + table.zset_bytes);
    assert_eq!(report.store_bytes, table.total_bytes());
    assert_eq!(report.total_bytes(), report.store_bytes + report.query_bytes);
    assert!(table.bytes_per_row() > 0.0);
}

/// An empty circuit must report zero rather than a floor of allocator noise,
/// so a delta against it is meaningful.
#[test]
fn empty_circuit_reports_zero() {
    let report = Circuit::new().size_report();
    assert_eq!(report.total_bytes(), 0);
    assert!(report.tables.is_empty());
    assert!(report.views.is_empty());
}

/// Best-effort process RSS, for context in the printed output only. `None`
/// off Linux, which is why no assertion depends on it.
fn rss_bytes() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let kb: u64 = status
        .lines()
        .find_map(|l| l.strip_prefix("VmRSS:"))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    Some(kb * 1024)
}
