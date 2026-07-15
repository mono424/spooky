use anyhow::Result;
use std::path::Path;

use crate::restore::{RestoreOutcome, RestoreProgress};

/// Host-specific hooks for the shared backup/restore workers.
///
/// The workers own everything host-independent: S3 transfer, gzip, and the
/// wipe+import/export of the main SurrealDB. Hosts implement the state they
/// alone understand — the scheduler drains its WAL into the replica and evicts
/// SSPs; a standalone SSP re-bootstraps its circuit.
#[async_trait::async_trait]
pub trait MaintenanceHost: Send + Sync + 'static {
    /// Called before the main-DB export. Scheduler: drain pending events into
    /// the replica and return `Some(snapshot_seq)`. SSP: nothing to do, `None`.
    async fn pre_backup(&self) -> Result<Option<u64>>;

    /// Gate a restore and block ingest. Must fail (leaving host state
    /// untouched) when the host is not in a restorable state; on success the
    /// host stops accepting `/ingest` traffic until `finish_restore`.
    async fn begin_restore(&self) -> Result<()>;

    /// Called after the main DB has been wiped and re-imported from
    /// `dump_path`. The host resynchronizes its own state from the restored
    /// database (scheduler: replica reset + import + seq/WAL/buffer reset +
    /// SSP eviction; SSP: circuit wipe + re-bootstrap).
    async fn post_restore(&self, dump_path: &Path) -> Result<RestoreOutcome>;

    /// Terminal status transition after a restore attempt. Only called when
    /// `begin_restore` succeeded (`progress.gate_entered`). The host decides
    /// whether it is safe to serve traffic again given how far the restore got.
    async fn finish_restore(&self, result: &Result<RestoreOutcome>, progress: RestoreProgress);

    /// Extra JSON merged into the `GET /backup/status` response
    /// (scheduler: pending-event/lag counters).
    async fn status_extras(&self) -> serde_json::Value {
        serde_json::json!({})
    }
}
