use opentelemetry::{KeyValue, metrics::MeterProvider};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{
    Resource,
    metrics::{MeterProviderBuilder, PeriodicReader, SdkMeterProvider},
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};
use std::time::{Duration, Instant};

pub struct Metrics {
    pub ingest_counter: opentelemetry::metrics::Counter<u64>,
    pub ingest_duration: opentelemetry::metrics::Histogram<f64>,
    pub view_count: opentelemetry::metrics::UpDownCounter<i64>,
    pub edge_operations: opentelemetry::metrics::Counter<u64>,

    // Internal tracking for rate calculation
    ingest_total: Arc<AtomicU64>,
}

struct RateState {
    last_count: u64,
    last_tick: Instant,
}

impl Metrics {
    pub fn new(provider: &SdkMeterProvider) -> Self {
        let meter = provider.meter("ssp");

        let ingest_total = Arc::new(AtomicU64::new(0));
        let count_ref = ingest_total.clone();

        // State for rate calculation (protected by Mutex for the callback)
        let rate_state = Arc::new(Mutex::new(RateState {
            last_count: 0,
            last_tick: Instant::now(),
        }));

        // Observable Gauge for Ingestions Per Minute
        let _ingest_rate = meter
            .u64_observable_gauge("ssp_ingest_rate_per_minute")
            .with_description("Ingestion rate per minute (calculated window)")
            .with_callback(move |observer| {
                let current_total = count_ref.load(Ordering::Relaxed);

                if let Ok(mut state) = rate_state.lock() {
                    let now = Instant::now();
                    let elapsed = now.duration_since(state.last_tick).as_secs_f64();
                    // Avoid division by zero or extremely small intervals
                    if elapsed.round() >= 60.0 {
                        let delta = current_total.saturating_sub(state.last_count);
                        let rate_per_sec = delta as f64 / elapsed;
                        let rate_per_min = rate_per_sec * 60.0;
                        observer.observe(rate_per_min.round() as u64, &[]);

                        // Update state
                        state.last_count = current_total;
                        state.last_tick = now;
                    }
                }
            })
            .build();

        Self {
            ingest_counter: meter
                .u64_counter("ssp_ingest_total")
                .with_description("Total number of ingest operations")
                .build(),
            ingest_duration: meter
                .f64_histogram("ssp_ingest_duration_milliseconds")
                .with_description("Ingest operation duration")
                .build(),
            view_count: meter
                .i64_up_down_counter("ssp_views_active")
                .with_description("Number of active views")
                .build(),
            edge_operations: meter
                .u64_counter("ssp_edge_operations_total")
                .with_description("Total edge operations by type")
                .build(),
            ingest_total,
        }
    }

    pub fn inc_ingest(&self, count: u64, _: &[KeyValue]) {
        self.ingest_total.fetch_add(count, Ordering::Relaxed);
    }
}

pub fn init_metrics() -> Result<(SdkMeterProvider, Metrics), anyhow::Error> {
    let endpoint = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
        .unwrap_or_else(|_| "http://localhost:18888".to_string());

    let service_name = std::env::var("OTEL_SERVICE_NAME").unwrap_or_else(|_| "ssp".to_string());

    let resource = Resource::new(vec![KeyValue::new("service.name", service_name)]);

    let exporter = opentelemetry_otlp::MetricExporter::builder()
        .with_tonic()
        .with_endpoint(&endpoint)
        .build()?;

    let reader = PeriodicReader::builder(exporter, opentelemetry_sdk::runtime::Tokio)
        .with_interval(Duration::from_secs(15))
        .build();

    let provider = MeterProviderBuilder::default()
        .with_resource(resource)
        .with_reader(reader)
        .build();

    let metrics = Metrics::new(&provider);

    Ok((provider, metrics))
}
