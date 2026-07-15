//! Query edge-update service now lives in the portable core
//! (`ssp_node::edges`, over the `Db`/`Scheduler`/`Telemetry` ports).
//! Re-exported so existing `crate::edge_updates::…` paths keep working.
//! The shell's `update_all_edges` (in `lib.rs`) is a thin label-preserving
//! wrapper over `ssp_node::edges::build_edge_batch` + `wrap_in_transaction`.
pub use ssp_node::edges::{
    build_edge_batch, run_edge_update_service, wrap_in_transaction, Batcher, CircuitVersions,
    EdgeBatch, EdgeSink, RecordVersions, SurrealEdgeSink, MAX_EDGE_BATCH,
};
