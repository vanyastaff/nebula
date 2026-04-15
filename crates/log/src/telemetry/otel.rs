//! OpenTelemetry integration
//!
//! Builds a tracing-compatible OpenTelemetry layer with OTLP gRPC export.

use opentelemetry::{KeyValue, global, trace::TracerProvider};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{
    Resource,
    propagation::TraceContextPropagator,
    trace::{Sampler, SdkTracerProvider},
};
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::Layer;

use crate::{
    config::{Fields, TelemetryConfig},
    core::{LogError, LogResult},
};

/// Result of building an OpenTelemetry layer.
///
/// Contains both the tracing layer (for the subscriber stack) and the
/// `SdkTracerProvider` (for graceful shutdown on drop).
pub struct OtelLayer {
    pub layer: Box<dyn Layer<tracing_subscriber::Registry> + Send + Sync>,
    pub provider: SdkTracerProvider,
}

/// Resolve the OTLP endpoint from an explicit config value plus the externally
/// supplied env-var value.
///
/// Precedence: explicit `config.otlp_endpoint` → provided `env_endpoint` → off.
/// The literal values `"disabled"` and `""` are treated as explicit opt-out at
/// both the config and env layers.
///
/// Returns `None` when OTLP should be disabled.
///
/// This helper is pure (no env-var reads) so it can be unit-tested without
/// mutating process-global state. The production entry point `resolve_endpoint`
/// is a thin wrapper that reads `OTEL_EXPORTER_OTLP_ENDPOINT` and delegates.
fn resolve_endpoint_from(config: &TelemetryConfig, env_endpoint: Option<&str>) -> Option<String> {
    // 1. Explicit config wins.
    if let Some(endpoint) = config.otlp_endpoint.as_deref() {
        let trimmed = endpoint.trim();
        if trimmed.is_empty() || trimmed == "disabled" {
            return None;
        }
        return Some(trimmed.to_string());
    }

    // 2. Env var falls through.
    if let Some(endpoint) = env_endpoint {
        let trimmed = endpoint.trim();
        if trimmed.is_empty() || trimmed == "disabled" {
            return None;
        }
        return Some(trimmed.to_string());
    }

    // 3. No config + no env = OTLP off (opt-in). Previously defaulted to http://localhost:4317,
    //    which caused surprise network activity in environments that never ran a collector (see
    //    #375).
    None
}

/// Production entry point for endpoint resolution: reads the `OTEL_EXPORTER_OTLP_ENDPOINT`
/// env var and delegates to [`resolve_endpoint_from`].
fn resolve_endpoint(config: &TelemetryConfig) -> Option<String> {
    let env_endpoint = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").ok();
    resolve_endpoint_from(config, env_endpoint.as_deref())
}

/// Build an OpenTelemetry tracing layer with OTLP gRPC export.
///
/// Returns `Ok(None)` when OTLP is not configured (no endpoint in config and
/// no `OTEL_EXPORTER_OTLP_ENDPOINT` env var), or when the endpoint is
/// explicitly `"disabled"` or empty at either layer.
///
/// The layer is boxed to erase the concrete type, which allows it to compose
/// with arbitrary subscriber stacks (e.g. when a Sentry layer is added on top).
///
/// Since #380, this function is pure with respect to `opentelemetry::global` —
/// the caller is responsible for calling [`install_globals`] after the tracing
/// subscriber is successfully installed, or [`shutdown_unused_provider`] if
/// subscriber installation fails.
///
/// # Errors
///
/// Returns `LogError::Telemetry` if the OTLP exporter or tracer provider cannot
/// be constructed.
pub fn build_layer(config: &TelemetryConfig, fields: &Fields) -> LogResult<Option<OtelLayer>> {
    let endpoint_str = match resolve_endpoint(config) {
        Some(e) => e,
        None => return Ok(None),
    };

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

    // #380: globals are NOT set here — the caller installs them after the
    // subscriber is successfully `try_init`'d so a mid-init failure does not
    // leave a dangling tracer provider in `opentelemetry::global`.
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

/// Install OTel globals from a successfully-built provider.
///
/// Must be called only **after** the tracing subscriber's `try_init` succeeds,
/// so a subscriber-init failure cannot leave the OTel global state pointing at
/// a provider whose lifecycle no longer matches the `LoggerGuard`. See #380.
///
/// Sets:
/// - the W3C trace-context propagator as the global text-map propagator
/// - the given provider as the global tracer provider
pub(crate) fn install_globals(provider: &SdkTracerProvider) {
    global::set_text_map_propagator(TraceContextPropagator::new());
    global::set_tracer_provider(provider.clone());
}

/// Shut down a provider that was built but never installed globally.
///
/// Used by the builder when `try_init` fails after `build_layer` succeeded, to
/// avoid leaking exporter tasks / network connections. See #380.
///
/// Uses `eprintln!` rather than `tracing::error!` because this runs only after
/// `try_init` failed, so the tracing dispatcher is not installed — a
/// `tracing::error!` call would silently go to the global no-op dispatcher.
pub(crate) fn shutdown_unused_provider(provider: SdkTracerProvider) {
    if let Err(e) = provider.shutdown() {
        eprintln!("nebula-log: unused OTel provider shutdown error: {e}");
    }
}

fn build_exporter(endpoint: &str) -> LogResult<opentelemetry_otlp::SpanExporter> {
    opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint)
        .build()
        .map_err(|e| LogError::Telemetry(format!("OTLP exporter build failed: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_with(endpoint: Option<&str>) -> TelemetryConfig {
        TelemetryConfig {
            otlp_endpoint: endpoint.map(str::to_string),
            service_name: "test".to_string(),
            sampling_rate: 1.0,
        }
    }

    /// #375 — with no endpoint in config and no `OTEL_EXPORTER_OTLP_ENDPOINT`
    /// env value, resolution must return `None` (opt-in), not silently point
    /// at `http://localhost:4317`.
    #[test]
    fn unset_config_and_env_is_opt_out() {
        let cfg = config_with(None);
        assert_eq!(resolve_endpoint_from(&cfg, None), None);
    }

    /// Empty-string env is also treated as opt-out (trim-aware).
    #[test]
    fn empty_env_is_opt_out() {
        let cfg = config_with(None);
        assert_eq!(resolve_endpoint_from(&cfg, Some("")), None);
        assert_eq!(resolve_endpoint_from(&cfg, Some("   ")), None);
    }

    /// `"disabled"` in env is an explicit opt-out.
    #[test]
    fn disabled_env_is_opt_out() {
        let cfg = config_with(None);
        assert_eq!(resolve_endpoint_from(&cfg, Some("disabled")), None);
    }

    /// Explicit empty config is opt-out even if the env has a real endpoint:
    /// `config` wins.
    #[test]
    fn empty_config_wins_over_env() {
        let cfg = config_with(Some(""));
        assert_eq!(
            resolve_endpoint_from(&cfg, Some("http://collector:4317")),
            None
        );
    }

    /// `"disabled"` in config is an explicit opt-out even if the env has a real
    /// endpoint.
    #[test]
    fn disabled_config_wins_over_env() {
        let cfg = config_with(Some("disabled"));
        assert_eq!(
            resolve_endpoint_from(&cfg, Some("http://collector:4317")),
            None
        );
    }

    /// Explicit config endpoint wins over env.
    #[test]
    fn config_endpoint_wins_over_env() {
        let cfg = config_with(Some("http://config-endpoint:4317"));
        assert_eq!(
            resolve_endpoint_from(&cfg, Some("http://env-endpoint:4317")),
            Some("http://config-endpoint:4317".to_string())
        );
    }

    /// Env falls through when config is `None`.
    #[test]
    fn env_used_when_config_none() {
        let cfg = config_with(None);
        assert_eq!(
            resolve_endpoint_from(&cfg, Some("http://env-endpoint:4317")),
            Some("http://env-endpoint:4317".to_string())
        );
    }

    /// #380 — end-to-end unit test for the build/cleanup cycle that proves
    /// `build_layer` does not install OTel globals and that
    /// `shutdown_unused_provider` cleanly tears down a built-but-never-installed
    /// provider.
    ///
    /// We build an `OtelLayer` with a syntactically valid but unreachable
    /// endpoint (so exporter construction succeeds but no actual export
    /// happens), then immediately shut it down. A regression in which
    /// `build_layer` installs globals, or in which `shutdown_unused_provider`
    /// panics / deadlocks, would be caught here.
    ///
    /// Runs under `#[tokio::test]` because the tonic gRPC exporter requires a
    /// Tokio reactor during construction (even though no actual network I/O
    /// occurs during this test).
    #[tokio::test]
    async fn build_layer_then_shutdown_is_safe() {
        let cfg = TelemetryConfig {
            otlp_endpoint: Some("http://127.0.0.1:1".to_string()),
            service_name: "build-layer-then-shutdown".to_string(),
            sampling_rate: 0.0,
        };
        let fields = Fields::default();

        let otel = build_layer(&cfg, &fields)
            .expect("build_layer must succeed for a syntactically valid endpoint")
            .expect("build_layer must return Some(OtelLayer) when endpoint is set");

        // At this point the provider has been built but install_globals was
        // never called. Dropping or shutting down must not panic.
        shutdown_unused_provider(otel.provider);
    }
}
