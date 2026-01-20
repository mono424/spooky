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

    // 1. Hole Config aus Environment oder nimm Defaults (Fallback)
    let otlp_endpoint = env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
        .unwrap_or_else(|_| "http://localhost:18888".to_string());
        
    let service_name = env::var("OTEL_SERVICE_NAME")
        .unwrap_or_else(|_| "unknown-service".to_string());

    // 1. Set global propagator
    global::set_text_map_propagator(TraceContextPropagator::new());

    // 2. Define resource
    let resource = Resource::new(vec![
        KeyValue::new("service.name", service_name.clone()), // Hier kommt der Name aus Docker rein!
    ]);

    // 3. Create OTLP exporter for Traces
    let trace_exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(&otlp_endpoint) // Dynamischer Endpoint
        .build()?;

    // 4. Create TracerProvider
    let tracer_provider = SdkTracerProvider::builder()
        .with_batch_exporter(trace_exporter, runtime::Tokio)
        .with_sampler(Sampler::AlwaysOn)
        .with_id_generator(RandomIdGenerator::default())
        .with_resource(resource.clone())
        .build();

    // 5. Get tracer from provider BEFORE setting it as global
    let tracer = tracer_provider.tracer(service_name);
    
    // 6. Set global tracer provider
    global::set_tracer_provider(tracer_provider);

    // 7. Create telemetry layer (Traces)
    let telemetry_layer = tracing_opentelemetry::layer().with_tracer(tracer);

    // --- LOGGING ---

    // 8. Create OTLP exporter for Logs
    let log_exporter = opentelemetry_otlp::LogExporter::builder()
        .with_tonic()
        .with_endpoint(&otlp_endpoint) // Dynamischer Endpoint
        .build()?;

    // 9. Create LoggerProvider
    let logger_provider = SdkLoggerProvider::builder()
        .with_batch_exporter(log_exporter, runtime::Tokio)
        .with_resource(resource)
        .build();
    
    // 10. Create Log Bridge Layer
    let log_layer = OpenTelemetryTracingBridge::new(&logger_provider);

    // 11. Env Filter
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "ssp=debug,axum=info,tower_http=info".into());

    // 12. Initialize Registry
    Registry::default()
        .with(env_filter)
        //.with(tracing_subscriber::fmt::layer()) // Disabled Console Logging
        .with(telemetry_layer)
        .with(log_layer)
        .init();

    Ok(())
}