//! Query edge-update service (ported from `apps/ssp/src/edge_updates.rs`).
//!
//! The circuit emits a [`ViewDelta`] whenever a registered query's window
//! changes. Each delta's `_00_list_ref` edge writes are coalesced over a
//! window and flushed as ONE aggregated SurrealDB transaction.
//!
//! Portability note: the previous shell version bound the `_00_query`
//! incantation record id as a `surrealdb::RecordId` param (`$fromN`). The core
//! can't depend on the SDK, so the incantation now crosses the [`Db`] port as
//! a plain **string** key and the SQL wraps it in `type::record('_00_query',
//! $fromN)` — preserving the original bind safety (arbitrary keys) without a
//! RecordId. The `out`/`parent`/subquery record ids keep the existing literal
//! interpolation (they arrive already-validated from the circuit).

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, error};

use ssp::circuit::{Circuit, SubqueryOp, ViewDelta};
use ssp_protocol::RefMode;

use crate::ports::{Db, Scheduler, Telemetry};
use crate::tables;

/// Cap on buffered deltas between flushes. A sustained flood flushes early once
/// it crosses this, so the buffer (and resulting transaction) stay bounded.
pub const MAX_EDGE_BATCH: usize = 4096;

/// Looks up the stored version for a record key. Abstracts `Circuit.store` so
/// [`build_edge_batch`] is pure and unit-testable without a circuit.
pub trait RecordVersions {
    fn version_of(&self, key: &str) -> i64;
}

/// `RecordVersions` over a live circuit store.
pub struct CircuitVersions<'a>(pub &'a Circuit);
impl RecordVersions for CircuitVersions<'_> {
    fn version_of(&self, key: &str) -> i64 {
        self.0.store.get_record_version_by_key(key).unwrap_or(1)
    }
}

/// The aggregated edge-write statements + incantation key bindings for a batch
/// of deltas. `bindings` map `fromN` → the `_00_query` record KEY (bound as a
/// string; the SQL wraps it in `type::record('_00_query', $fromN)`).
#[derive(Debug, Default, PartialEq)]
pub struct EdgeBatch {
    pub statements: Vec<String>,
    pub bindings: Vec<(String, String)>,
    pub created: u64,
    pub updated: u64,
    pub deleted: u64,
}

impl EdgeBatch {
    pub fn is_empty(&self) -> bool {
        self.statements.is_empty()
    }
}

/// Structural record-id check (`table:key`, both non-empty). Replaces the
/// former `RecordId::parse_simple` guard — the SDK isn't available in the core.
fn is_valid_record_id(id: &str) -> bool {
    matches!(id.split_once(':'), Some((t, k)) if !t.is_empty() && !k.is_empty())
}

/// The `_00_query` incantation record id (`_00_query:<key>`) for a view id.
pub fn format_incantation_id(id: &str) -> String {
    let raw = id.rsplit(':').next().unwrap_or(id);
    format!("_00_query:{}", raw)
}

/// The incantation KEY (`<key>` of `_00_query:<key>`) — what gets bound.
fn incantation_key(id: &str) -> String {
    id.rsplit(':').next().unwrap_or(id).to_string()
}

/// Build the `_00_list_ref` edge-write statements for a batch of deltas. PURE.
/// Each delta binds its `_00_query` key as `$from{idx}` (unique across the
/// batch); the SQL references `type::record('_00_query', $from{idx})`.
pub fn build_edge_batch(
    deltas: &[&ViewDelta],
    mode: RefMode,
    versions: &impl RecordVersions,
) -> EdgeBatch {
    let mut batch = EdgeBatch::default();

    for (idx, delta) in deltas.iter().enumerate() {
        // Skip deltas with nothing to write. A delta that changes ONLY subquery
        // children (a comment added to a thread already in the view — the
        // parent's membership is unchanged) still carries `subquery_items` that
        // must become `_00_list_ref` edges, so it must NOT be skipped. Skipping
        // it (the old behavior) is why reverse-link children like comments never
        // synced while forward links, whose delta also carried a parent
        // addition/content-update, worked.
        if delta.additions.is_empty()
            && delta.updates.is_empty()
            && delta.removals.is_empty()
            && delta.subquery_items.is_empty()
        {
            continue;
        }

        let incantation_id = format_incantation_id(&delta.query_id);
        let list_ref = tables::list_ref_table(mode, &delta.auth_id);

        if !is_valid_record_id(&incantation_id) {
            error!(incantation_id = %incantation_id, "Invalid incantation ID format - skipping view");
            continue;
        }

        // RELATE/DELETE only accept a record id or a PARAM as the graph
        // endpoint (not a `type::record(...)` expression). So bind the key as a
        // string and `LET $fromN` to the record inside the transaction; every
        // statement then references `$fromN` exactly as the original
        // RecordId-bound code did — but nothing SDK-specific crosses the port.
        let bn = format!("from{}", idx);
        let from = format!("${bn}");
        batch
            .statements
            .push(format!("LET ${bn} = type::record('_00_query', ${bn}key)"));
        batch.bindings.push((format!("{bn}key"), incantation_key(&delta.query_id)));

        // Additions (Created)
        for id in &delta.additions {
            if !is_valid_record_id(id) {
                error!(target: "ssp::edges", record_id = %id, view_id = %delta.query_id, "Invalid record ID - skipping edge create");
                continue;
            }
            let version = versions.version_of(id);
            batch.created += 1;
            batch.statements.push(format!(
                "RELATE {from}->{list_ref}->{out} SET version = {version}, clientId = (SELECT VALUE clientId FROM {from} LIMIT 1)[0], auth_id = (SELECT VALUE auth_id FROM {from} LIMIT 1)[0]",
                from = from, list_ref = list_ref, out = id, version = version,
            ));
        }

        // Updates (Updated)
        for id in &delta.updates {
            if !is_valid_record_id(id) {
                error!(target: "ssp::edges", record_id = %id, view_id = %delta.query_id, "Invalid record ID - skipping edge update");
                continue;
            }
            let version = versions.version_of(id);
            batch.updated += 1;
            batch.statements.push(format!(
                "UPDATE {list_ref} SET version = {version} WHERE in = {from} AND out = {out}",
                list_ref = list_ref, version = version, from = from, out = id,
            ));
        }

        // Removals (Deleted)
        for id in &delta.removals {
            if !is_valid_record_id(id) {
                error!(target: "ssp::edges", record_id = %id, view_id = %delta.query_id, "Invalid record ID - skipping edge delete");
                continue;
            }
            batch.deleted += 1;
            batch.statements.push(format!(
                "DELETE {from}->{list_ref} WHERE out = {out}",
                from = from, list_ref = list_ref, out = id,
            ));
        }

        // Subquery child edges. Processed AFTER main records so parent
        // list_ref entries exist in the same transaction.
        for item in &delta.subquery_items {
            if !is_valid_record_id(&item.id) {
                error!(target: "ssp::edges", record_id = %item.id, view_id = %delta.query_id, "Invalid subquery record ID - skipping");
                continue;
            }
            match item.op {
                SubqueryOp::Add => {
                    let version = versions.version_of(&item.id);
                    batch.created += 1;
                    batch.statements.push(format!(
                        "RELATE {from}->{list_ref}->{id} SET \
                         version = {version}, \
                         clientId = (SELECT VALUE clientId FROM {from} LIMIT 1)[0], \
                         auth_id = (SELECT VALUE auth_id FROM {from} LIMIT 1)[0], \
                         parent = (SELECT VALUE id FROM {list_ref} WHERE in = {from} AND out = {parent} LIMIT 1)[0], \
                         parent_rel = '{alias}'",
                        from = from, list_ref = list_ref, id = item.id,
                        version = version, parent = item.parent_key, alias = item.alias,
                    ));
                }
                SubqueryOp::Update => {
                    let version = versions.version_of(&item.id);
                    batch.updated += 1;
                    batch.statements.push(format!(
                        "UPDATE {list_ref} SET version = {version} WHERE in = {from} AND out = {id}",
                        list_ref = list_ref, from = from, id = item.id, version = version,
                    ));
                }
                SubqueryOp::Remove => {
                    batch.deleted += 1;
                    batch.statements.push(format!(
                        "DELETE {from}->{list_ref} WHERE out = {id}",
                        from = from, list_ref = list_ref, id = item.id,
                    ));
                }
            }
        }
    }

    batch
}

/// Wrap edge statements in a single SurrealDB transaction. `None` when there is
/// nothing to write (so the caller skips the round-trip entirely).
pub fn wrap_in_transaction(statements: &[String]) -> Option<String> {
    if statements.is_empty() {
        return None;
    }
    Some(format!(
        "BEGIN TRANSACTION;\n{};\nCOMMIT TRANSACTION;",
        statements.join(";\n")
    ))
}

/// Build + execute the aggregated edge transaction for a batch of deltas
/// through the [`Db`] port. Replaces the shell's `update_all_edges`.
pub async fn run_edge_writes(
    db: &dyn Db,
    deltas: &[&ViewDelta],
    circuit: &Circuit,
    mode: RefMode,
    telemetry: &dyn Telemetry,
) {
    if deltas.is_empty() {
        return;
    }
    let batch = build_edge_batch(deltas, mode, &CircuitVersions(circuit));
    if batch.is_empty() {
        return;
    }

    telemetry.counter("edge_operations", batch.created + batch.updated + batch.deleted);
    debug!(
        created = batch.created,
        updated = batch.updated,
        deleted = batch.deleted,
        views = deltas.len(),
        "Processing edge operations"
    );

    let Some(full_query) = wrap_in_transaction(&batch.statements) else {
        return;
    };
    let op_count = batch.statements.len();
    let binds: Vec<(&str, Value)> = batch
        .bindings
        .iter()
        .map(|(name, key)| (name.as_str(), json!(key)))
        .collect();

    match db.query(&full_query, &binds).await {
        Ok(_) => debug!(target: "ssp::edges", operations = op_count, "Edge update transaction completed"),
        Err(e) => error!(target: "ssp::edges", error = %e, operations = op_count, "Edge update transaction failed - data may be out of sync"),
    }
}

/// Pure buffer/size-cap state machine for the throttler.
#[derive(Default)]
pub struct Batcher {
    buf: Vec<ViewDelta>,
    max_batch: usize,
}

impl Batcher {
    pub fn new(max_batch: usize) -> Self {
        Self { buf: Vec::new(), max_batch }
    }

    /// Buffer deltas. Returns `Some(batch)` to flush NOW if the buffer reached
    /// `max_batch` (`0` = no size cap).
    pub fn push(&mut self, deltas: Vec<ViewDelta>) -> Option<Vec<ViewDelta>> {
        self.buf.extend(deltas);
        if self.max_batch != 0 && self.buf.len() >= self.max_batch {
            Some(std::mem::take(&mut self.buf))
        } else {
            None
        }
    }

    /// Drain for a window-tick / shutdown flush. `None` when empty.
    pub fn take(&mut self) -> Option<Vec<ViewDelta>> {
        if self.buf.is_empty() {
            None
        } else {
            Some(std::mem::take(&mut self.buf))
        }
    }

    pub fn pending(&self) -> usize {
        self.buf.len()
    }
}

/// The flush boundary. The real impl ([`SurrealEdgeSink`]) writes to SurrealDB;
/// tests use a recording mock. `Send` bounds are cfg-gated: native tokio needs
/// `Send` futures, workers-rs DO futures are `!Send`.
#[cfg(not(target_arch = "wasm32"))]
pub trait EdgeSink: Send + Sync {
    fn flush(&self, deltas: Vec<ViewDelta>) -> impl std::future::Future<Output = ()> + Send;
}
#[cfg(target_arch = "wasm32")]
pub trait EdgeSink {
    fn flush(&self, deltas: Vec<ViewDelta>) -> impl std::future::Future<Output = ()>;
}

/// Drain `rx`, coalescing pushed delta batches and flushing them through
/// `sink` every `window` (or immediately when `window` is zero, or early once
/// `max_batch` is hit). The window is timed through the [`Scheduler`] port —
/// the loop is otherwise channel receives (`tokio::sync`), so it is portable.
pub async fn run_edge_update_service<S>(
    mut rx: mpsc::UnboundedReceiver<Vec<ViewDelta>>,
    sink: S,
    scheduler: Arc<dyn Scheduler>,
    window: Duration,
    max_batch: usize,
) where
    S: EdgeSink + 'static,
{
    if window.is_zero() {
        while let Some(deltas) = rx.recv().await {
            if !deltas.is_empty() {
                sink.flush(deltas).await;
            }
        }
        return;
    }

    let mut batcher = Batcher::new(max_batch);

    'windows: loop {
        // Pinned OUTSIDE the receive loop so a steady stream can't keep
        // restarting the window and starve the flush.
        let sleep_fut = scheduler.sleep(window);
        tokio::pin!(sleep_fut);

        loop {
            tokio::select! {
                maybe = rx.recv() => match maybe {
                    Some(deltas) => {
                        if let Some(ready) = batcher.push(deltas) {
                            sink.flush(ready).await;
                        }
                    }
                    None => {
                        if let Some(remainder) = batcher.take() {
                            sink.flush(remainder).await;
                        }
                        break 'windows;
                    }
                },
                _ = &mut sleep_fut => {
                    if let Some(batch) = batcher.take() {
                        sink.flush(batch).await;
                    }
                    continue 'windows;
                }
            }
        }
    }

    debug!("edge-update service stopped");
}

/// Real [`EdgeSink`]: builds + writes the aggregated transaction via the
/// [`Db`] port, reading versions from the circuit.
pub struct SurrealEdgeSink {
    pub db: Arc<dyn Db>,
    pub processor: Arc<RwLock<Circuit>>,
    pub telemetry: Arc<dyn Telemetry>,
    pub mode: RefMode,
}

impl EdgeSink for SurrealEdgeSink {
    async fn flush(&self, deltas: Vec<ViewDelta>) {
        let refs: Vec<&ViewDelta> = deltas.iter().collect();
        let circuit = self.processor.read().await;
        run_edge_writes(self.db.as_ref(), &refs, &circuit, self.mode, self.telemetry.as_ref()).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssp::circuit::{SubqueryDeltaItem, SubqueryOp, ViewDelta};

    struct ConstV(i64);
    impl RecordVersions for ConstV {
        fn version_of(&self, _key: &str) -> i64 {
            self.0
        }
    }

    fn delta(query_id: &str, auth_id: &str) -> ViewDelta {
        ViewDelta {
            query_id: query_id.to_string(),
            additions: vec![],
            removals: vec![],
            updates: vec![],
            records: vec![],
            result_hash: String::new(),
            subquery_items: vec![],
            auth_id: auth_id.to_string(),
        }
    }

    #[test]
    fn empty_delta_produces_nothing() {
        let d = delta("q:1", "user:a");
        let b = build_edge_batch(&[&d], RefMode::Single, &ConstV(1));
        assert!(b.is_empty());
        assert!(b.bindings.is_empty());
    }

    #[test]
    fn addition_binds_incantation_key_and_wraps_type_thing() {
        let mut d = delta("view:abc", "user:a");
        d.additions = vec!["user:x".to_string()];
        let b = build_edge_batch(&[&d], RefMode::Single, &ConstV(7));

        assert_eq!(b.created, 1);
        assert_eq!(b.bindings, vec![("from0key".to_string(), "abc".to_string())]);
        // statement[0] is the LET binding the incantation record.
        assert_eq!(b.statements[0], "LET $from0 = type::record('_00_query', $from0key)");
        let stmt = &b.statements[1];
        assert!(stmt.contains("RELATE $from0->_00_list_ref->user:x"), "{stmt}");
        assert!(stmt.contains("version = 7"), "{stmt}");
    }

    #[test]
    fn multiple_deltas_get_unique_bindings() {
        let mut d0 = delta("view:a", "user:1");
        d0.additions = vec!["t:1".to_string()];
        let mut d1 = delta("view:b", "user:2");
        d1.removals = vec!["t:2".to_string()];
        let b = build_edge_batch(&[&d0, &d1], RefMode::Single, &ConstV(1));
        assert_eq!(b.bindings.len(), 2);
        assert_eq!(b.bindings[0], ("from0key".to_string(), "a".to_string()));
        assert_eq!(b.bindings[1], ("from1key".to_string(), "b".to_string()));
        assert_eq!(b.created, 1);
        assert_eq!(b.deleted, 1);
    }

    #[test]
    fn subquery_add_references_parent_and_binds() {
        let mut d = delta("view:z", "user:9");
        d.additions = vec!["parent:1".to_string()];
        d.subquery_items = vec![SubqueryDeltaItem {
            id: "child:1".to_string(),
            parent_key: "parent:1".to_string(),
            alias: "kids".to_string(),
            op: SubqueryOp::Add,
        }];
        let b = build_edge_batch(&[&d], RefMode::Single, &ConstV(1));
        // one primary + one subquery add
        assert_eq!(b.created, 2);
        let sub = b.statements.iter().find(|s| s.contains("child:1")).unwrap();
        assert!(sub.contains("parent_rel = 'kids'"), "{sub}");
        assert!(sub.contains("$from0"), "{sub}");
    }

    #[test]
    fn invalid_record_ids_skipped() {
        let mut d = delta("view:q", "user:a");
        d.additions = vec!["nocolon".to_string(), "ok:1".to_string()];
        let b = build_edge_batch(&[&d], RefMode::Single, &ConstV(1));
        assert_eq!(b.created, 1, "only the valid id produced an edge");
    }

    #[test]
    fn wrap_transaction_roundtrip() {
        assert_eq!(wrap_in_transaction(&[]), None);
        let q = wrap_in_transaction(&["A".into(), "B".into()]).unwrap();
        assert!(q.starts_with("BEGIN TRANSACTION;"));
        assert!(q.contains("A;\nB"));
        assert!(q.trim_end().ends_with("COMMIT TRANSACTION;"));
    }
}
