use opentelemetry::{KeyValue, metrics::MeterProvider};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{
    metrics::{MeterProviderBuilder, PeriodicReader, SdkMeterProvider},
    Resource,
};
use std::time::Duration;

pub struct Metrics {
    pub ingest_counter: opentelemetry::metrics::Counter<u64>,
    pub ingest_duration: opentelemetry::metrics::Histogram<f64>,
    pub view_count: opentelemetry::metrics::UpDownCounter<i64>,
    pub edge_operations: opentelemetry::metrics::Counter<u64>,
}

impl Metrics {
    pub fn new(provider: &SdkMeterProvider) -> Self {
        let meter = provider.meter("ssp");
        
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
        }
    }
}

pub fn init_metrics() -> Result<(SdkMeterProvider, Metrics), anyhow::Error> {
    let endpoint = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
        .unwrap_or_else(|_| "http://localhost:18888".to_string());
    
    let service_name = std::env::var("OTEL_SERVICE_NAME")
        .unwrap_or_else(|_| "ssp".to_string());
    
    let resource = Resource::new(vec![
        KeyValue::new("service.name", service_name),
    ]);
    
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
