use std::sync::Arc;

use crate::ports::{CircuitStore, Db, HttpClient, Scheduler, Spawner, Telemetry};

/// The bundle of platform adapters a shell hands to the core.
///
/// VM shell (`apps/ssp`): SurrealSdkDb + ReqwestHttp + TokioScheduler +
/// TokioSpawner + OtelTelemetry. CF shell (`apps/ssp-cf`, future): HttpSqlDb
/// (or SDK-on-wasm) + FetchHttp + AlarmScheduler + spawn_local + Noop.
///
/// [`crate::ports::ArtifactStore`] is deliberately absent — the maintenance
/// plane is VM-only for now and wires its own storage (see `ports/artifacts.rs`).
#[derive(Clone)]
pub struct Platform {
    pub db: Arc<dyn Db>,
    pub http: Arc<dyn HttpClient>,
    pub scheduler: Arc<dyn Scheduler>,
    pub spawner: Arc<dyn Spawner>,
    pub telemetry: Arc<dyn Telemetry>,
    /// Circuit snapshot persistence. VM = `NoopCircuitStore` (process holds the
    /// circuit); ephemeral hosts supply a durable store so cold-start restores
    /// instead of always rebuilding from the DB.
    pub circuit_store: Arc<dyn CircuitStore>,
}
