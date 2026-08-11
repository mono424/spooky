//! VM (tokio/axum) implementations of the `ssp-node` platform ports.
//!
//! This is the entire tokio/reqwest/OTel-facing adapter layer — the future
//! Cloudflare shell (`apps/ssp-cf`) provides its own equivalents
//! (AlarmScheduler / FetchHttp / spawn_local / …) against the same traits.
//! See `docs/platform-architecture.md`.

mod backend_health;
mod db;
mod http;
mod scheduler;
mod spawn;
mod telemetry;

pub use backend_health::MaintenanceBackendHealth;
pub use db::SurrealSdkDb;
pub use http::ReqwestHttp;
pub use scheduler::TokioScheduler;
pub use spawn::TokioSpawner;
pub use telemetry::OtelTelemetry;

use std::sync::Arc;

/// Assemble the VM platform bundle. `timer_rx` is the delivery side of the
/// scheduler: the shell drains it and dispatches each fired `TimerKind` to
/// the node (today: a placeholder drain loop — wired to `on_timer` as core
/// migration proceeds).
pub fn vm_platform(
    db: crate::SharedDb,
    metrics: Arc<crate::metrics::Metrics>,
) -> (ssp_node::Platform, tokio::sync::mpsc::UnboundedReceiver<ssp_node::TimerKind>) {
    let (scheduler, timer_rx) = TokioScheduler::new();
    let platform = ssp_node::Platform {
        db: Arc::new(SurrealSdkDb::new(db)),
        http: Arc::new(ReqwestHttp::new()),
        scheduler: Arc::new(scheduler),
        spawner: Arc::new(TokioSpawner),
        telemetry: Arc::new(OtelTelemetry::new(metrics)),
        circuit_store: circuit_store(),
    };
    (platform, timer_rx)
}

/// Snapshot store for the VM.
///
/// Without one, every restart is a cold rebuild: the SSP re-pages the whole
/// database over HTTP while the scheduler holds the tenant's sync frozen under
/// `drain_lock` for up to its bootstrap budget. With one, a restart restores
/// the snapshot and catches up only the rows past its resume point.
///
/// Enabled by pointing `SPKY_SSP_SNAPSHOT_DIR` at a writable directory —
/// writable being checked, not assumed, because a missing volume or a
/// read-only root is a real deployment state and the right response is to run
/// without snapshots rather than to fail startup. The snapshot is a cache;
/// SurrealDB remains the source of truth.
fn circuit_store() -> Arc<dyn ssp_node::CircuitStore> {
    let Some(dir) = std::env::var_os("SPKY_SSP_SNAPSHOT_DIR") else {
        return Arc::new(ssp_node::NoopCircuitStore);
    };
    let dir = std::path::PathBuf::from(dir);
    if !ssp_node::DiskCircuitStore::probe_writable(&dir) {
        tracing::warn!(
            dir = %dir.display(),
            "SPKY_SSP_SNAPSHOT_DIR is not writable — restarts will do a full rebuild"
        );
        return Arc::new(ssp_node::NoopCircuitStore);
    }
    tracing::info!(dir = %dir.display(), "Circuit snapshots enabled");
    Arc::new(ssp_node::DiskCircuitStore::new(&dir))
}
