use super::MaybeSendSync;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, thiserror::Error)]
pub enum CircuitStoreError {
    /// No snapshot persisted yet — a truly cold start (rebuild from the DB).
    #[error("not found")]
    NotFound,
    #[error("transport: {0}")]
    Transport(String),
    /// Stored blob failed to parse / restore — treat as cold and rebuild.
    #[error("corrupt: {0}")]
    Corrupt(String),
}

/// Where a restored circuit snapshot sits relative to the current database, so
/// cold-start can bound catch-up and decide restore-vs-rebuild.
///
/// There is NO global mutation sequence for a standalone node (the scheduler's
/// `snapshot_seq` is cluster-only; SurrealDB's `_00_rv`/`_00_version` are
/// per-record). So the resume point is content-oriented, not a stream cursor:
/// staleness + per-table content hashes + the highest per-record version
/// folded into the snapshot. Catch-up selects rows newer than
/// `max_row_version` and a content-hash verify (falling back to a full
/// re-scan) guarantees convergence — a lossy/absent `_00_rv` degrades to
/// rebuild, never silent divergence.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResumePoint {
    /// Epoch-ms the snapshot was checkpointed. Gates staleness.
    pub saved_at_epoch_ms: u64,
    /// Per-table content hash at checkpoint (`Circuit::compute_table_hashes`).
    pub table_hashes: BTreeMap<String, String>,
    /// Highest `_00_rv` folded into the snapshot, per table. `None`/absent for
    /// a table ⇒ catch-up re-scans that table in full.
    pub max_row_version: BTreeMap<String, i64>,
}

/// Persist / restore the DBSP circuit across process (or Durable Object)
/// lifetimes. Generalizes the cold-start / eviction problem into a port: a
/// long-lived host (VM) supplies a noop (the process holds the circuit); an
/// ephemeral host (edge/DO/serverless) supplies a durable blob store.
///
/// The blob is the output of `ssp::circuit::Circuit::save()` (a JSON string;
/// the operator DAG is rebuilt from plans on `restore`, not serialized).
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
pub trait CircuitStore: MaybeSendSync {
    /// Persist blob + resume point atomically, overwriting the previous
    /// snapshot (write-temp-then-rename on disk; single put on a KV store).
    async fn save(&self, blob: &str, point: &ResumePoint) -> Result<(), CircuitStoreError>;

    /// Load the latest snapshot. `NotFound` = cold start (caller rebuilds).
    async fn load(&self) -> Result<(String, ResumePoint), CircuitStoreError>;

    /// Drop the snapshot (used by `/reset` and after a divergence wipe).
    async fn clear(&self) -> Result<(), CircuitStoreError>;
}

/// A `CircuitStore` that never persists — every `load` is a cold start. Used by
/// long-lived hosts (the VM) whose process already holds the circuit in memory,
/// so `bootstrap()` always takes the rebuild-from-DB branch (today's behavior).
pub struct NoopCircuitStore;

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl CircuitStore for NoopCircuitStore {
    async fn save(&self, _blob: &str, _point: &ResumePoint) -> Result<(), CircuitStoreError> {
        Ok(())
    }
    async fn load(&self) -> Result<(String, ResumePoint), CircuitStoreError> {
        Err(CircuitStoreError::NotFound)
    }
    async fn clear(&self) -> Result<(), CircuitStoreError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Minimal in-memory store for exercising the round-trip contract.
    #[derive(Default)]
    struct MemStore(Mutex<Option<(String, ResumePoint)>>);

    #[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
    #[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
    impl CircuitStore for MemStore {
        async fn save(&self, blob: &str, point: &ResumePoint) -> Result<(), CircuitStoreError> {
            *self.0.lock().unwrap() = Some((blob.to_string(), point.clone()));
            Ok(())
        }
        async fn load(&self) -> Result<(String, ResumePoint), CircuitStoreError> {
            self.0.lock().unwrap().clone().ok_or(CircuitStoreError::NotFound)
        }
        async fn clear(&self) -> Result<(), CircuitStoreError> {
            *self.0.lock().unwrap() = None;
            Ok(())
        }
    }

    #[tokio::test]
    async fn noop_store_is_always_cold() {
        let s = NoopCircuitStore;
        s.save("blob", &ResumePoint::default()).await.unwrap();
        assert!(matches!(s.load().await, Err(CircuitStoreError::NotFound)));
    }

    #[tokio::test]
    async fn mem_store_round_trips_blob_and_point() {
        let s = MemStore::default();
        assert!(matches!(s.load().await, Err(CircuitStoreError::NotFound)));

        let mut point = ResumePoint { saved_at_epoch_ms: 123, ..Default::default() };
        point.table_hashes.insert("thread".into(), "b3:abc".into());
        point.max_row_version.insert("thread".into(), 7);
        s.save("SNAPSHOT", &point).await.unwrap();

        let (blob, got) = s.load().await.unwrap();
        assert_eq!(blob, "SNAPSHOT");
        assert_eq!(got.saved_at_epoch_ms, 123);
        assert_eq!(got.table_hashes.get("thread").map(String::as_str), Some("b3:abc"));
        assert_eq!(got.max_row_version.get("thread"), Some(&7));

        s.clear().await.unwrap();
        assert!(matches!(s.load().await, Err(CircuitStoreError::NotFound)));
    }
}
