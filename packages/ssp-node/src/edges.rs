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
use tracing::{debug, error, warn};

use crate::db_retry::query_retrying;

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

/// Rounds a batch's leftovers are carried into the next window before they
/// are given up on (see [`run_edge_update_service`]).
pub const MAX_EDGE_CARRY: u32 = 5;

/// Build + execute the aggregated edge transaction for a batch of deltas
/// through the [`Db`] port, and NEVER silently drop them.
///
/// Returns the deltas that could not be written. The transaction is retried
/// through [`query_retrying`] (SurrealDB's optimistic conflicts are the
/// ordinary case on these rows: the TTL sweep, the view metrics and client
/// heartbeats all write `_00_query` / `_00_list_ref_*`); once the budget is
/// out the batch is split in halves and each half retried on its own, so one
/// poisoned statement cannot take the other 4095 down with it. A single delta
/// that still fails is returned to the caller, logged with its view.
///
/// Before this, a failed transaction was one `error!` and the deltas were
/// gone: the clients subscribed to those views never received the row (a
/// message, a call's `accepted`) until something re-materialized the view,
/// which in practice was a reload.
pub async fn write_deltas_resilient(
    db: &dyn Db,
    deltas: Vec<ViewDelta>,
    circuit: &Circuit,
    mode: RefMode,
    telemetry: &dyn Telemetry,
) -> Vec<ViewDelta> {
    if deltas.is_empty() {
        return Vec::new();
    }
    let refs: Vec<&ViewDelta> = deltas.iter().collect();
    let batch = build_edge_batch(&refs, mode, &CircuitVersions(circuit));
    if batch.is_empty() {
        return Vec::new();
    }
    let Some(full_query) = wrap_in_transaction(&batch.statements) else {
        return Vec::new();
    };
    let op_count = batch.statements.len();
    let binds: Vec<(&str, Value)> = batch
        .bindings
        .iter()
        .map(|(name, key)| (name.as_str(), json!(key)))
        .collect();

    debug!(
        created = batch.created,
        updated = batch.updated,
        deleted = batch.deleted,
        views = deltas.len(),
        "Processing edge operations"
    );

    match query_retrying(db, &full_query, &binds).await {
        Ok(_) => {
            telemetry.counter("edge_operations", batch.created + batch.updated + batch.deleted);
            debug!(target: "ssp::edges", operations = op_count, "Edge update transaction completed");
            Vec::new()
        }
        Err(e) if deltas.len() > 1 => {
            // Split on ANY error, not just a conflict: a statement that is
            // wrong (not merely contended) must be isolated, not retried as
            // part of everything else forever.
            telemetry.counter("edge_batch_split", 1);
            warn!(target: "ssp::edges", error = %e, views = deltas.len(), operations = op_count, "Edge update transaction failed after retries; splitting the batch");
            let mut left = deltas;
            let right = left.split_off(left.len() / 2);
            let mut leftovers = Box::pin(write_deltas_resilient(db, left, circuit, mode, telemetry)).await;
            leftovers.extend(Box::pin(write_deltas_resilient(db, right, circuit, mode, telemetry)).await);
            leftovers
        }
        Err(e) => {
            error!(target: "ssp::edges", error = %e, view_id = %deltas[0].query_id, operations = op_count, "Edge delta not written after retries");
            deltas
        }
    }
}

/// Fire-and-forget form of [`write_deltas_resilient`] over borrowed deltas;
/// leftovers are counted and logged. Kept for the direct-write callers and
/// the existing tests.
pub async fn run_edge_writes(
    db: &dyn Db,
    deltas: &[&ViewDelta],
    circuit: &Circuit,
    mode: RefMode,
    telemetry: &dyn Telemetry,
) {
    let owned: Vec<ViewDelta> = deltas.iter().map(|d| (*d).clone()).collect();
    let left = write_deltas_resilient(db, owned, circuit, mode, telemetry).await;
    if !left.is_empty() {
        telemetry.counter("edge_deltas_dropped", left.len() as u64);
        error!(target: "ssp::edges", views = left.len(), "edge deltas dropped after retries");
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

    /// Re-queue deltas a flush could not write, AHEAD of anything buffered
    /// since: a carried RELATE must land before a newer DELETE for the same
    /// view, or the edge would resurrect.
    pub fn push_front(&mut self, deltas: Vec<ViewDelta>) {
        if deltas.is_empty() {
            return;
        }
        let mut merged = deltas;
        merged.append(&mut self.buf);
        self.buf = merged;
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
/// `flush` returns the deltas it could NOT write; the service carries them
/// into the next window (see [`MAX_EDGE_CARRY`]).
#[cfg(not(target_arch = "wasm32"))]
pub trait EdgeSink: Send + Sync {
    fn flush(&self, deltas: Vec<ViewDelta>) -> impl std::future::Future<Output = Vec<ViewDelta>> + Send;
}
#[cfg(target_arch = "wasm32")]
pub trait EdgeSink {
    fn flush(&self, deltas: Vec<ViewDelta>) -> impl std::future::Future<Output = Vec<ViewDelta>>;
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
    // Leftovers of a flush ride at the front of the next batch. Bounded: after
    // MAX_EDGE_CARRY consecutive rounds a delta still failing is dropped and
    // counted, so a permanently broken statement cannot pin the queue.
    let mut carry_rounds: u32 = 0;
    let mut carry = |batcher: &mut Batcher, left: Vec<ViewDelta>| {
        if left.is_empty() {
            carry_rounds = 0;
            return;
        }
        carry_rounds += 1;
        if carry_rounds > MAX_EDGE_CARRY {
            error!(target: "ssp::edges", views = left.len(), rounds = carry_rounds, "edge deltas dropped after carry rounds");
            // Dropped for good: the next flush starts a fresh count.
            carry_rounds = 0;
            return;
        }
        debug!(target: "ssp::edges", views = left.len(), rounds = carry_rounds, "carrying unwritten edge deltas into the next window");
        batcher.push_front(left);
    };

    if window.is_zero() {
        let mut batcher = Batcher::new(0);
        while let Some(deltas) = rx.recv().await {
            batcher.push(deltas);
            if let Some(ready) = batcher.take() {
                let left = sink.flush(ready).await;
                carry(&mut batcher, left);
            }
        }
        if let Some(remainder) = batcher.take() {
            sink.flush(remainder).await;
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
                            let left = sink.flush(ready).await;
                            carry(&mut batcher, left);
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
                        let left = sink.flush(batch).await;
                        carry(&mut batcher, left);
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
    async fn flush(&self, deltas: Vec<ViewDelta>) -> Vec<ViewDelta> {
        let circuit = self.processor.read().await;
        write_deltas_resilient(self.db.as_ref(), deltas, &circuit, self.mode, self.telemetry.as_ref()).await
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
