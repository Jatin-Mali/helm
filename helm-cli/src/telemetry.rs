#[allow(dead_code)]
pub struct TelemetryConfig {
    pub enabled: bool,
    pub endpoint: String,
    pub service_name: String,
}

#[cfg(feature = "otel")]
pub fn build_otel_tracer(config: &TelemetryConfig) -> Option<opentelemetry_sdk::trace::Tracer> {
    if !config.enabled {
        return None;
    }
    let endpoint =
        std::env::var("HELM_TELEMETRY_ENDPOINT").unwrap_or_else(|_| config.endpoint.clone());
    opentelemetry_otlp::new_pipeline()
        .tracing()
        .with_exporter(
            opentelemetry_otlp::new_exporter()
                .tonic()
                .with_endpoint(endpoint),
        )
        .with_trace_config(opentelemetry_sdk::trace::Config::default().with_resource(
            opentelemetry_sdk::Resource::new(vec![opentelemetry::KeyValue::new(
                "service.name",
                config.service_name.clone(),
            )]),
        ))
        .install_batch(opentelemetry_sdk::runtime::Tokio)
        .ok()
}
