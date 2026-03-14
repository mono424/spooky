use opentelemetry::{global, trace::TracerProvider, KeyValue};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{
    propagation::TraceContextPropagator,
    runtime,
    trace::{RandomIdGenerator, Sampler, TracerProvider as SdkTracerProvider},
    logs::LoggerProvider as SdkLoggerProvider,
    Resource,
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, Registry};
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use std::env;

pub fn init_tracing() -> Result<(), anyhow::Error> {
    let otlp_endpoint = env::var("OTEL_EXPORTER_OTLP_ENDPOINT").ok();

    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| {
            if otlp_endpoint.is_some() {
                "ssp=debug,axum=info,tower_http=info,opentelemetry_sdk=warn".into()
            } else {
                "ssp=debug,axum=info,tower_http=info".into()
            }
        });

    if let Some(endpoint) = otlp_endpoint {
        let service_name = env::var("OTEL_SERVICE_NAME")
            .unwrap_or_else(|_| "ssp".to_string());

        global::set_text_map_propagator(TraceContextPropagator::new());

        let resource = Resource::new(vec![
            KeyValue::new("service.name", service_name.clone()),
        ]);

        // Traces
        let trace_exporter = opentelemetry_otlp::SpanExporter::builder()
            .with_tonic()
            .with_endpoint(&endpoint)
            .build()?;

        let tracer_provider = SdkTracerProvider::builder()
            .with_batch_exporter(trace_exporter, runtime::Tokio)
            .with_sampler(Sampler::AlwaysOn)
            .with_id_generator(RandomIdGenerator::default())
            .with_resource(resource.clone())
            .build();

        let tracer = tracer_provider.tracer(service_name);
        global::set_tracer_provider(tracer_provider);

        let telemetry_layer = tracing_opentelemetry::layer().with_tracer(tracer);

        // Logs
        let log_exporter = opentelemetry_otlp::LogExporter::builder()
            .with_tonic()
            .with_endpoint(&endpoint)
            .build()?;

        let logger_provider = SdkLoggerProvider::builder()
            .with_batch_exporter(log_exporter, runtime::Tokio)
            .with_resource(resource)
            .build();

        let log_layer = OpenTelemetryTracingBridge::new(&logger_provider);

        Registry::default()
            .with(env_filter)
            .with(tracing_subscriber::fmt::layer())
            .with(telemetry_layer)
            .with(log_layer)
            .init();
    } else {
        // Console-only: no OTLP exporters
        Registry::default()
            .with(env_filter)
            .with(tracing_subscriber::fmt::layer())
            .init();
    }

    Ok(())
}
