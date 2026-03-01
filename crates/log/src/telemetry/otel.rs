//! OpenTelemetry integration
//!
//! Builds a tracing-compatible OpenTelemetry layer with OTLP gRPC export.

use crate::config::{Fields, TelemetryConfig};
use crate::core::{LogError, LogResult};
use opentelemetry::{KeyValue, global};
use opentelemetry::trace::TracerProvider;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{
    Resource,
    propagation::TraceContextPropagator,
    trace::{Sampler, SdkTracerProvider},
};
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::Layer;

/// Result of building an OpenTelemetry layer.
///
/// Contains both the tracing layer (for the subscriber stack) and the
/// `SdkTracerProvider` (for graceful shutdown on drop).
pub struct OtelLayer {
    pub layer: Box<dyn Layer<tracing_subscriber::Registry> + Send + Sync>,
    pub provider: SdkTracerProvider,
}

/// Build an OpenTelemetry tracing layer with OTLP gRPC export.
///
/// Returns `Ok(None)` when the endpoint is `"disabled"` or empty.
///
/// The layer is boxed to erase the concrete type, which allows it to compose
/// with arbitrary subscriber stacks (e.g. when a Sentry layer is added on top).
///
/// # Errors
///
/// Returns `LogError::Telemetry` if the OTLP exporter or tracer provider cannot
/// be constructed.
pub fn build_layer(
    config: &TelemetryConfig,
    fields: &Fields,
) -> LogResult<Option<OtelLayer>> {
    let endpoint_str = match &config.otlp_endpoint {
        Some(endpoint) if !endpoint.is_empty() => endpoint.clone(),
        _ => match std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT") {
            Ok(endpoint) if !endpoint.is_empty() => endpoint,
            _ => "http://localhost:4317".to_string(),
        },
    };

    if endpoint_str == "disabled" || endpoint_str.is_empty() {
        return Ok(None);
    }

    // Set up W3C trace-context propagator
    global::set_text_map_propagator(TraceContextPropagator::new());

    // Configure sampler
    let sampler = if config.sampling_rate >= 1.0 {
        Sampler::AlwaysOn
    } else if config.sampling_rate <= 0.0 {
        Sampler::AlwaysOff
    } else {
        Sampler::TraceIdRatioBased(config.sampling_rate)
    };

    // Build OTel resource from config + fields (OTel semantic conventions)
    let resource = build_resource(&config.service_name, fields);

    let provider_builder = SdkTracerProvider::builder()
        .with_sampler(sampler)
        .with_resource(resource);

    // Batch export requires an active Tokio runtime.
    // Fall back to simple export in sync contexts to avoid runtime panics.
    #[cfg(feature = "async")]
    let has_runtime = tokio::runtime::Handle::try_current().is_ok();
    #[cfg(not(feature = "async"))]
    let has_runtime = false;

    let exporter = build_exporter(&endpoint_str)?;
    let provider = if has_runtime {
        provider_builder.with_batch_exporter(exporter).build()
    } else {
        provider_builder.with_simple_exporter(exporter).build()
    };

    let tracer = provider.tracer("nebula-log");

    // Set as global provider (for context propagation)
    global::set_tracer_provider(provider.clone());

    Ok(Some(OtelLayer {
        layer: Box::new(OpenTelemetryLayer::new(tracer)),
        provider,
    }))
}

/// Build OTel `Resource` from service config and global fields.
///
/// Maps to OTel semantic conventions:
/// - `service.name` ← `TelemetryConfig::service_name`
/// - `service.version` ← `Fields::version`
/// - `deployment.environment.name` ← `Fields::env`
/// - `service.instance.id` ← `Fields::instance`
/// - `cloud.region` ← `Fields::region`
fn build_resource(service_name: &str, fields: &Fields) -> Resource {
    let mut attrs = vec![KeyValue::new("service.name", service_name.to_string())];

    if let Some(version) = &fields.version {
        attrs.push(KeyValue::new("service.version", version.clone()));
    }
    if let Some(env) = &fields.env {
        attrs.push(KeyValue::new("deployment.environment.name", env.clone()));
    }
    if let Some(instance) = &fields.instance {
        attrs.push(KeyValue::new("service.instance.id", instance.clone()));
    }
    if let Some(region) = &fields.region {
        attrs.push(KeyValue::new("cloud.region", region.clone()));
    }

    Resource::builder_empty().with_attributes(attrs).build()
}

fn build_exporter(endpoint: &str) -> LogResult<opentelemetry_otlp::SpanExporter> {
    opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint)
        .build()
        .map_err(|e| LogError::Telemetry(format!("OTLP exporter build failed: {e}")))
}
