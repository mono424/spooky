use opentelemetry::{global, trace::TracerProvider, KeyValue};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{
    propagation::TraceContextPropagator,
    runtime,
    trace::{RandomIdGenerator, Sampler, TracerProvider as SdkTracerProvider},
    Resource,
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, Registry};

pub fn init_tracing() -> Result<(), anyhow::Error> {
    // 1. Set global propagator
    global::set_text_map_propagator(TraceContextPropagator::new());

    // 2. Define resource
    let resource = Resource::new(vec![
        KeyValue::new("service.name", "spooky-sidecar"),
    ]);

    // 3. Create OTLP exporter
    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint("http://tempo:4317")
        .build()?;

    // 4. Create TracerProvider
    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter, runtime::Tokio)
        .with_sampler(Sampler::AlwaysOn)
        .with_id_generator(RandomIdGenerator::default())
        .with_resource(resource)
        .build();

    // 5. Get tracer from provider BEFORE setting it as global
    let tracer = provider.tracer("spooky-sidecar");
    
    // 6. Set global tracer provider
    global::set_tracer_provider(provider);

    // 7. Create telemetry layer
    let telemetry_layer = tracing_opentelemetry::layer().with_tracer(tracer);

    // 8. Env Filter
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "ssp=debug,axum=info,tower_http=info".into());

    // 9. Initialize Registry
    Registry::default()
        .with(env_filter)
        .with(tracing_subscriber::fmt::layer())
        .with(telemetry_layer)
        .init();

    Ok(())
}