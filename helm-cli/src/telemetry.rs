#[allow(dead_code)]
pub struct TelemetryConfig {
    pub enabled: bool,
    pub endpoint: String,
    pub service_name: String,
}

#[cfg(feature = "otel")]
pub fn build_otel_tracer(config: &TelemetryConfig) -> Option<opentelemetry_sdk::trace::Tracer> {
    use opentelemetry::trace::TracerProvider;
    use opentelemetry_otlp::WithExportConfig;

    if !config.enabled {
        return None;
    }
    let endpoint =
        std::env::var("HELM_TELEMETRY_ENDPOINT").unwrap_or_else(|_| config.endpoint.clone());
    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint)
        .build()
        .ok()?;
    let provider = opentelemetry_sdk::trace::TracerProvider::builder()
        .with_batch_exporter(exporter, opentelemetry_sdk::runtime::Tokio)
        .with_resource(opentelemetry_sdk::Resource::new(vec![
            opentelemetry::KeyValue::new("service.name", config.service_name.clone()),
        ]))
        .build();
    let tracer = provider.tracer("helm");
    opentelemetry::global::set_tracer_provider(provider);
    Some(tracer)
}
