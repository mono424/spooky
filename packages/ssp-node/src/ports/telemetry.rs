use super::MaybeSendSync;

/// Metrics facade. The VM shell wraps its OpenTelemetry instruments (OTLP/
/// tonic stays entirely in the shell — not wasm-buildable); the CF shell
/// starts with [`NoopTelemetry`] (Workers Analytics Engine later).
pub trait Telemetry: MaybeSendSync {
    fn counter(&self, name: &'static str, value: u64);
    fn histogram_ms(&self, name: &'static str, value: f64);
    fn gauge_add(&self, name: &'static str, delta: i64);
}

pub struct NoopTelemetry;

impl Telemetry for NoopTelemetry {
    fn counter(&self, _name: &'static str, _value: u64) {}
    fn histogram_ms(&self, _name: &'static str, _value: f64) {}
    fn gauge_add(&self, _name: &'static str, _delta: i64) {}
}
