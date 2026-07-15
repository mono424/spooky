//! Per-view latency state now lives in the portable core
//! (`ssp_node::view_metrics`). Re-exported for existing `crate::view_metrics::…`.
pub use ssp_node::view_metrics::{ViewMetrics, ViewMetricsState, MATERIALIZATION_SAMPLE_WINDOW};
