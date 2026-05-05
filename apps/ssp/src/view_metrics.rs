//! Per-view rolling materialization-latency state used by the ingest path
//! to compute and persist p55/p90/p99 plus running counters back onto the
//! `_00_query` row. Counters survive on the row across SSP restarts; the
//! sample window is rebuilt from scratch on boot, matching the client.

use std::collections::HashMap;
use tokio::sync::RwLock;

/// Cap on the rolling materialization-sample window kept per view in memory.
/// Mirrors the client-side window in `packages/core/src/types.ts`.
pub const MATERIALIZATION_SAMPLE_WINDOW: usize = 100;

#[derive(Default)]
pub struct ViewMetricsState {
    samples: Vec<f64>,
    pub update_count: u64,
    pub error_count: u64,
}

impl ViewMetricsState {
    pub fn record_sample(&mut self, sample_ms: f64) {
        self.samples.push(sample_ms);
        if self.samples.len() > MATERIALIZATION_SAMPLE_WINDOW {
            // Drop the oldest sample. Vec::remove(0) is fine here, the
            // window is small (<=100) so the shift is negligible.
            self.samples.remove(0);
        }
    }

    pub fn percentiles(&self) -> Option<(f64, f64, f64)> {
        if self.samples.is_empty() {
            return None;
        }
        let mut sorted = self.samples.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let pick = |q: f64| {
            let idx = ((q * sorted.len() as f64).floor() as usize).min(sorted.len() - 1);
            sorted[idx]
        };
        Some((pick(0.55), pick(0.90), pick(0.99)))
    }
}

pub type ViewMetrics = RwLock<HashMap<String, ViewMetricsState>>;
