//! Query edge-update service.
//!
//! The circuit emits a [`ViewDelta`] every time a registered query's window
//! changes (an ingest step, or the initial snapshot at registration). Each
//! delta's `_00_list_ref` edge writes were previously flushed in their own
//! SurrealDB transaction — one per record — so SurrealDB fired LIVE
//! notifications per record and a freshly-connecting client streamed its whole
//! window over ~10s of serialized round-trips.
//!
//! This module is the dedicated service that fixes that: callers push delta
//! batches onto an [`mpsc`] channel; [`run_edge_update_service`] **throttles**
//! them over a configurable window and writes the **aggregated** edges — every
//! statement for every buffered delta, including subquery child edges — in
//! **one** SurrealDB transaction per flush.
//!
//! The pieces are split so the logic is fully unit-testable without a database:
//!   - [`build_edge_batch`] — pure: deltas → edge statements + bindings.
//!   - [`wrap_in_transaction`] — pure: statements → one `BEGIN…COMMIT`.
//!   - [`Batcher`] — pure: the buffer/size-cap state machine.
//!   - [`EdgeSink`] — the flush boundary (real impl writes to SurrealDB; tests
//!     use a recording mock).

use std::time::Duration;

use surrealdb::types::RecordId;
use tokio::sync::mpsc;
use tracing::debug;

use ssp::circuit::{SubqueryOp, ViewDelta};
use ssp_protocol::RefMode;

use crate::{format_incantation_id, parse_record_id, tables};

/// Cap on buffered deltas between flushes. A sustained flood flushes early once
/// it crosses this, so the buffer (and the resulting transaction) stay bounded.
pub const MAX_EDGE_BATCH: usize = 4096;

/// Looks up the stored version for a record key. Abstracts `Circuit.store` so
/// [`build_edge_batch`] is pure and unit-testable without a circuit.
pub trait RecordVersions {
    fn version_of(&self, key: &str) -> i64;
}

/// The aggregated edge-write statements + record-id bindings for a batch of
/// view deltas (primary window edges and subquery child edges).
#[derive(Debug, Default, PartialEq)]
pub struct EdgeBatch {
    pub statements: Vec<String>,
    pub bindings: Vec<(String, RecordId)>,
    pub created: u64,
    pub updated: u64,
    pub deleted: u64,
}

impl EdgeBatch {
    pub fn is_empty(&self) -> bool {
        self.statements.is_empty()
    }
}

/// Build the `_00_list_ref` edge-write statements for a batch of deltas. PURE —
/// no IO. Each delta binds its `_00_query` record id as `$from{idx}` (unique
/// across the batch) and routes to the owner's per-user list_ref table.
/// Mirrors the per-statement SQL the SSP has always emitted; the only change is
/// that the whole batch is built (and later committed) together.
pub fn build_edge_batch(
    deltas: &[&ViewDelta],
    mode: RefMode,
    versions: &impl RecordVersions,
) -> EdgeBatch {
    let mut batch = EdgeBatch::default();

    for (idx, delta) in deltas.iter().enumerate() {
        // Skip deltas with no primary-window change (preserves long-standing
        // behavior: a delta carrying only subquery items is not emitted here).
        if delta.additions.is_empty() && delta.updates.is_empty() && delta.removals.is_empty() {
            continue;
        }

        let incantation_id = format_incantation_id(&delta.query_id);
        let list_ref_tbl = tables::list_ref_table(mode, &delta.auth_id);

        let Some(from_id) = parse_record_id(&incantation_id) else {
            tracing::error!(incantation_id = %incantation_id, "Invalid incantation ID format - skipping view");
            continue;
        };

        let binding_name = format!("from{}", idx);
        batch.bindings.push((binding_name.clone(), from_id));

        // Additions (Created)
        for id in &delta.additions {
            if parse_record_id(id).is_none() {
                tracing::error!(target: "ssp::edges", record_id = %id, view_id = %delta.query_id, "Invalid record ID format - skipping edge create");
                continue;
            }
            let version = versions.version_of(id);
            batch.created += 1;
            batch.statements.push(format!(
                "RELATE ${binding}->{list_ref}->{out} SET version = {version}, clientId = (SELECT VALUE clientId FROM ${binding} LIMIT 1)[0], auth_id = (SELECT VALUE auth_id FROM ${binding} LIMIT 1)[0]",
                binding = binding_name, list_ref = list_ref_tbl, out = id, version = version,
            ));
        }

        // Updates (Updated)
        for id in &delta.updates {
            if parse_record_id(id).is_none() {
                tracing::error!(target: "ssp::edges", record_id = %id, view_id = %delta.query_id, "Invalid record ID format - skipping edge update");
                continue;
            }
            let version = versions.version_of(id);
            batch.updated += 1;
            batch.statements.push(format!(
                "UPDATE {list_ref} SET version = {version} WHERE in = ${binding} AND out = {out}",
                list_ref = list_ref_tbl, version = version, binding = binding_name, out = id,
            ));
        }

        // Removals (Deleted)
        for id in &delta.removals {
            if parse_record_id(id).is_none() {
                tracing::error!(target: "ssp::edges", record_id = %id, view_id = %delta.query_id, "Invalid record ID format - skipping edge delete");
                continue;
            }
            batch.deleted += 1;
            batch.statements.push(format!(
                "DELETE ${binding}->{list_ref} WHERE out = {out}",
                binding = binding_name, list_ref = list_ref_tbl, out = id,
            ));
        }

        // Subquery child edges. Processed AFTER main records so parent list_ref
        // entries exist in the same transaction.
        for item in &delta.subquery_items {
            if parse_record_id(&item.id).is_none() {
                tracing::error!(target: "ssp::edges", record_id = %item.id, view_id = %delta.query_id, "Invalid subquery record ID format - skipping");
                continue;
            }
            match item.op {
                SubqueryOp::Add => {
                    let version = versions.version_of(&item.id);
                    batch.created += 1;
                    batch.statements.push(format!(
                        "RELATE ${binding}->{list_ref}->{id} SET \
                         version = {version}, \
                         clientId = (SELECT VALUE clientId FROM ${binding} LIMIT 1)[0], \
                         auth_id = (SELECT VALUE auth_id FROM ${binding} LIMIT 1)[0], \
                         parent = (SELECT VALUE id FROM {list_ref} WHERE in = ${binding} AND out = {parent} LIMIT 1)[0], \
                         parent_rel = '{alias}'",
                        binding = binding_name, list_ref = list_ref_tbl, id = item.id,
                        version = version, parent = item.parent_key, alias = item.alias,
                    ));
                }
                SubqueryOp::Update => {
                    let version = versions.version_of(&item.id);
                    batch.updated += 1;
                    batch.statements.push(format!(
                        "UPDATE {list_ref} SET version = {version} WHERE in = ${binding} AND out = {id}",
                        list_ref = list_ref_tbl, binding = binding_name, id = item.id, version = version,
                    ));
                }
                SubqueryOp::Remove => {
                    batch.deleted += 1;
                    batch.statements.push(format!(
                        "DELETE ${binding}->{list_ref} WHERE out = {id}",
                        binding = binding_name, list_ref = list_ref_tbl, id = item.id,
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

/// Pure buffer/size-cap state machine for the throttler. Keeps the timing-free
/// part of the service unit-testable.
#[derive(Default)]
pub struct Batcher {
    buf: Vec<ViewDelta>,
    max_batch: usize,
}

impl Batcher {
    pub fn new(max_batch: usize) -> Self {
        Self { buf: Vec::new(), max_batch }
    }

    /// Buffer a batch of deltas. Returns `Some(batch)` to flush NOW if the
    /// buffer has reached `max_batch` (`max_batch == 0` means no size cap).
    pub fn push(&mut self, deltas: Vec<ViewDelta>) -> Option<Vec<ViewDelta>> {
        self.buf.extend(deltas);
        if self.max_batch != 0 && self.buf.len() >= self.max_batch {
            Some(std::mem::take(&mut self.buf))
        } else {
            None
        }
    }

    /// Drain the buffer for a window-tick / shutdown flush. `None` when empty,
    /// so the caller never starts an empty transaction.
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

/// The flush boundary: where a coalesced delta batch is turned into the
/// aggregated transaction and written. The real implementation
/// ([`SurrealEdgeSink`]) writes to SurrealDB; unit tests use a recording mock.
pub trait EdgeSink {
    fn flush(&self, deltas: Vec<ViewDelta>) -> impl std::future::Future<Output = ()> + Send;
}

/// Run the edge-update service: drain `rx`, coalescing pushed delta batches and
/// flushing them through `sink` as one aggregated transaction every `window`
/// (or immediately when `window` is zero, or early once `max_batch` is hit).
/// Returns when all senders are dropped, after a final flush of the remainder.
pub async fn run_edge_update_service<S>(
    mut rx: mpsc::UnboundedReceiver<Vec<ViewDelta>>,
    sink: S,
    window: Duration,
    max_batch: usize,
) where
    S: EdgeSink + Send + Sync + 'static,
{
    // Throttling disabled — flush every received batch immediately.
    if window.is_zero() {
        while let Some(deltas) = rx.recv().await {
            if !deltas.is_empty() {
                sink.flush(deltas).await;
            }
        }
        return;
    }

    let mut batcher = Batcher::new(max_batch);
    let mut ticker = tokio::time::interval(window);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            maybe = rx.recv() => match maybe {
                Some(deltas) => {
                    if let Some(ready) = batcher.push(deltas) {
                        sink.flush(ready).await;
                    }
                }
                // All senders dropped (shutdown): flush the remainder and stop.
                None => {
                    if let Some(remainder) = batcher.take() {
                        sink.flush(remainder).await;
                    }
                    break;
                }
            },
            _ = ticker.tick() => {
                if let Some(batch) = batcher.take() {
                    sink.flush(batch).await;
                }
            }
        }
    }

    debug!("edge-update service stopped");
}

/// Real [`EdgeSink`]: builds the aggregated transaction (reading record
/// versions from the circuit) and writes it to SurrealDB via
/// [`crate::update_all_edges`].
pub struct SurrealEdgeSink {
    pub db: crate::SharedDb,
    pub processor: std::sync::Arc<tokio::sync::RwLock<ssp::circuit::Circuit>>,
    pub metrics: std::sync::Arc<crate::metrics::Metrics>,
    pub mode: RefMode,
}

impl EdgeSink for SurrealEdgeSink {
    async fn flush(&self, deltas: Vec<ViewDelta>) {
        let refs: Vec<&ViewDelta> = deltas.iter().collect();
        let circuit = self.processor.read().await;
        crate::update_all_edges(&self.db, &refs, &self.metrics, &circuit, self.mode).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssp::circuit::SubqueryDeltaItem;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    /// Constant version source.
    struct ConstV(i64);
    impl RecordVersions for ConstV {
        fn version_of(&self, _key: &str) -> i64 {
            self.0
        }
    }

    /// Map-backed version source (defaults to 1 for unknown keys).
    struct MapV(HashMap<String, i64>);
    impl RecordVersions for MapV {
        fn version_of(&self, key: &str) -> i64 {
            *self.0.get(key).unwrap_or(&1)
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

    // ---- build_edge_batch ---------------------------------------------------

    #[test]
    fn build_additions_emit_relate_with_binding() {
        let mut d = delta("q1", "");
        d.additions = vec!["game:a".into(), "game:b".into()];
        let b = build_edge_batch(&[&d], RefMode::Single, &ConstV(7));

        assert_eq!(b.created, 2);
        assert_eq!(b.updated, 0);
        assert_eq!(b.deleted, 0);
        assert_eq!(b.statements.len(), 2);
        assert_eq!(b.bindings.len(), 1);
        assert_eq!(b.bindings[0].0, "from0");
        assert!(b.statements[0].starts_with("RELATE $from0->"));
        assert!(b.statements[0].contains("->game:a SET version = 7"));
        assert!(b.statements[1].contains("->game:b SET version = 7"));
    }

    #[test]
    fn build_updates_and_removals() {
        let mut d = delta("q1", "");
        d.updates = vec!["game:u".into()];
        d.removals = vec!["game:r".into()];
        let b = build_edge_batch(&[&d], RefMode::Single, &MapV(HashMap::from([("game:u".to_string(), 9)])));

        assert_eq!((b.created, b.updated, b.deleted), (0, 1, 1));
        assert!(b.statements.iter().any(|s| s.starts_with("UPDATE ") && s.contains("version = 9") && s.contains("out = game:u")));
        assert!(b.statements.iter().any(|s| s.starts_with("DELETE $from0->") && s.contains("out = game:r")));
    }

    #[test]
    fn build_aggregates_multiple_deltas_with_unique_bindings() {
        let mut d0 = delta("q0", "");
        d0.additions = vec!["game:a".into()];
        let mut d1 = delta("q1", "");
        d1.additions = vec!["game:b".into()];

        let b = build_edge_batch(&[&d0, &d1], RefMode::Single, &ConstV(1));
        assert_eq!(b.created, 2);
        assert_eq!(b.bindings.len(), 2);
        assert_eq!(b.bindings[0].0, "from0");
        assert_eq!(b.bindings[1].0, "from1");
        assert!(b.statements[0].contains("$from0"));
        assert!(b.statements[1].contains("$from1"));
    }

    #[test]
    fn build_subquery_items_emit_parent_aware_edges() {
        let mut d = delta("q1", "");
        // A delta with subquery items must also carry a primary change, else it
        // is skipped (long-standing guard); give it one addition.
        d.additions = vec!["thread:t".into()];
        d.subquery_items = vec![
            SubqueryDeltaItem { id: "comment:c".into(), parent_key: "thread:t".into(), alias: "comments".into(), op: SubqueryOp::Add },
            SubqueryDeltaItem { id: "comment:u".into(), parent_key: "thread:t".into(), alias: "comments".into(), op: SubqueryOp::Update },
            SubqueryDeltaItem { id: "comment:r".into(), parent_key: "thread:t".into(), alias: "comments".into(), op: SubqueryOp::Remove },
        ];
        let b = build_edge_batch(&[&d], RefMode::Single, &ConstV(1));

        // 1 primary addition + 1 subquery add + 1 update + 1 remove.
        assert_eq!((b.created, b.updated, b.deleted), (2, 1, 1));
        assert!(b.statements.iter().any(|s| s.contains("->comment:c SET") && s.contains("parent_rel = 'comments'")));
        assert!(b.statements.iter().any(|s| s.starts_with("UPDATE ") && s.contains("out = comment:u")));
        assert!(b.statements.iter().any(|s| s.starts_with("DELETE ") && s.contains("out = comment:r")));
    }

    #[test]
    fn build_skips_invalid_ids_but_keeps_valid() {
        let mut d = delta("q1", "");
        d.additions = vec!["not-a-record-id".into(), "game:ok".into()];
        let b = build_edge_batch(&[&d], RefMode::Single, &ConstV(1));
        assert_eq!(b.created, 1);
        assert_eq!(b.statements.len(), 1);
        assert!(b.statements[0].contains("->game:ok SET"));
    }

    #[test]
    fn build_skips_delta_without_primary_change() {
        // Only subquery items, no additions/updates/removals → skipped entirely.
        let mut d = delta("q1", "");
        d.subquery_items = vec![SubqueryDeltaItem {
            id: "comment:c".into(), parent_key: "thread:t".into(), alias: "comments".into(), op: SubqueryOp::Add,
        }];
        let b = build_edge_batch(&[&d], RefMode::Single, &ConstV(1));
        assert!(b.is_empty());
        assert!(b.bindings.is_empty());
    }

    // ---- wrap_in_transaction ------------------------------------------------

    #[test]
    fn wrap_empty_is_none() {
        assert_eq!(wrap_in_transaction(&[]), None);
    }

    #[test]
    fn wrap_joins_in_one_transaction() {
        let q = wrap_in_transaction(&["A".into(), "B".into()]).unwrap();
        assert_eq!(q, "BEGIN TRANSACTION;\nA;\nB;\nCOMMIT TRANSACTION;");
    }

    // ---- Batcher ------------------------------------------------------------

    #[test]
    fn batcher_accumulates_until_taken() {
        let mut b = Batcher::new(0); // no size cap
        assert!(b.push(vec![delta("a", "")]).is_none());
        assert!(b.push(vec![delta("b", "")]).is_none());
        assert_eq!(b.pending(), 2);
        let taken = b.take().unwrap();
        assert_eq!(taken.len(), 2);
        assert_eq!(b.pending(), 0);
        assert!(b.take().is_none()); // empty → None
    }

    #[test]
    fn batcher_flushes_early_at_cap() {
        let mut b = Batcher::new(2);
        assert!(b.push(vec![delta("a", "")]).is_none());
        let ready = b.push(vec![delta("b", "")]).expect("cap reached");
        assert_eq!(ready.len(), 2);
        assert_eq!(b.pending(), 0);
    }

    // ---- run_edge_update_service (with a recording mock sink) ---------------

    #[derive(Clone, Default)]
    struct MockSink {
        flushes: Arc<Mutex<Vec<Vec<ViewDelta>>>>,
    }
    impl MockSink {
        fn flush_count(&self) -> usize {
            self.flushes.lock().unwrap().len()
        }
        fn total_deltas(&self) -> usize {
            self.flushes.lock().unwrap().iter().map(|b| b.len()).sum()
        }
    }
    impl EdgeSink for MockSink {
        async fn flush(&self, deltas: Vec<ViewDelta>) {
            self.flushes.lock().unwrap().push(deltas);
        }
    }

    #[tokio::test]
    async fn service_flushes_remainder_on_shutdown() {
        let (tx, rx) = mpsc::unbounded_channel();
        let sink = MockSink::default();
        // Long window so only the shutdown path flushes (deterministic).
        let handle = tokio::spawn(run_edge_update_service(rx, sink.clone(), Duration::from_secs(3600), 0));
        tx.send(vec![delta("a", ""), delta("b", "")]).unwrap();
        drop(tx); // shutdown
        handle.await.unwrap();
        assert_eq!(sink.flush_count(), 1);
        assert_eq!(sink.total_deltas(), 2);
    }

    #[tokio::test]
    async fn service_flushes_early_at_cap() {
        let (tx, rx) = mpsc::unbounded_channel();
        let sink = MockSink::default();
        // Cap of 2, long window: the size cap (not the timer) drives the flush.
        let handle = tokio::spawn(run_edge_update_service(rx, sink.clone(), Duration::from_secs(3600), 2));
        tx.send(vec![delta("a", "")]).unwrap();
        tx.send(vec![delta("b", "")]).unwrap(); // crosses the cap → flush
        // Give the task a moment to process the cap flush.
        for _ in 0..10 {
            if sink.flush_count() == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert_eq!(sink.flush_count(), 1);
        assert_eq!(sink.total_deltas(), 2);
        drop(tx);
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn service_coalesces_within_window() {
        let (tx, rx) = mpsc::unbounded_channel();
        let sink = MockSink::default();
        let window = Duration::from_millis(60);
        let handle = tokio::spawn(run_edge_update_service(rx, sink.clone(), window, MAX_EDGE_BATCH));
        // Two pushes well within one window → coalesced into a single flush.
        tx.send(vec![delta("a", "")]).unwrap();
        tx.send(vec![delta("b", "")]).unwrap();
        tokio::time::sleep(Duration::from_millis(140)).await; // past one window
        assert_eq!(sink.flush_count(), 1, "both pushes should batch into one flush");
        assert_eq!(sink.total_deltas(), 2);
        // A later push lands in a fresh window → a second, separate flush.
        tx.send(vec![delta("c", "")]).unwrap();
        tokio::time::sleep(Duration::from_millis(140)).await;
        assert_eq!(sink.flush_count(), 2);
        drop(tx);
        handle.await.unwrap();
    }
}
