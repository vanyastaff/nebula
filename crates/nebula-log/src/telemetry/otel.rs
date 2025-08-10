//! OpenTelemetry integration

#[cfg(feature = "telemetry")]
use crate::{config::TelemetryConfig, Result};
use opentelemetry::global;
use opentelemetry::trace::TracerProvider;
use opentelemetry_sdk::{
    propagation::TraceContextPropagator,
    trace::{Sampler, SdkTracerProvider},
};
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::Layer;

/// Build OpenTelemetry layer
pub fn build_layer(
    config: &TelemetryConfig,
) -> Result<Option<impl Layer<tracing_subscriber::Registry>>> {
    // Check if endpoint is configured
    let endpoint_str = match &config.otlp_endpoint {
        Some(endpoint) if !endpoint.is_empty() => endpoint.clone(),
        _ => match std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT") {
            Ok(endpoint) if !endpoint.is_empty() => endpoint,
            _ => "http://localhost:4317".to_string(),
        },
    };

    // Skip if explicitly disabled
    if endpoint_str == "disabled" || endpoint_str.is_empty() {
        return Ok(None);
    }

    // Set up propagator
    global::set_text_map_propagator(TraceContextPropagator::new());

    // Configure sampler based on sampling rate
    let sampler = if config.sampling_rate >= 1.0 {
        Sampler::AlwaysOn
    } else if config.sampling_rate <= 0.0 {
        Sampler::AlwaysOff
    } else {
        Sampler::TraceIdRatioBased(config.sampling_rate)
    };

    // Create a simple tracer provider with just the sampler
    // This is a minimal implementation that works with OpenTelemetry 0.30.0
    let provider = SdkTracerProvider::builder()
        .with_sampler(sampler)
        .build();
    
    // Get a tracer from the provider directly (not using global registry)
    // This ensures we get an SdkTracer which implements PreSampledTracer
    let tracer = provider.tracer("nebula-log");

    // Set the provider as global (for context propagation)
    global::set_tracer_provider(provider);

    // Create OpenTelemetry tracing layer
    let layer = OpenTelemetryLayer::new(tracer);

    Ok(Some(layer))
}

/// Shutdown OpenTelemetry provider
pub fn shutdown() {
    // In OpenTelemetry 0.30.0, there's no direct shutdown function
    // The provider will be cleaned up when dropped
    // This is a no-op
}