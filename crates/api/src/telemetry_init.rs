//! API binary telemetry bootstrap.
//!
//! Installs the W3C `TraceContextPropagator` (from `opentelemetry_sdk::propagation`)
//! **and** wires a `tracing` Subscriber that includes
//! [`tracing_opentelemetry::OpenTelemetryLayer`]. Both pieces are mandatory for in-process W3C
//! propagation to actually take effect:
//!
//! - `set_text_map_propagator` alone covers `inject_context` / `extract`, but
//!   [`tracing_opentelemetry::OpenTelemetrySpanExt::set_parent`] and
//!   [`tracing_opentelemetry::OpenTelemetrySpanExt::context`] are layer-driven — without
//!   `OpenTelemetryLayer` in the Subscriber they are silent no-ops, so inbound parent attach,
//!   response header injection, and control-queue carrier capture would all produce empty trace
//!   ids in production binaries even though the wiring code looks correct.
//!
//! When `OTEL_EXPORTER_OTLP_ENDPOINT` is set (and not empty / not `"disabled"`), the tracer
//! provider is built with an OTLP `SpanExporter` over gRPC tonic so spans are pushed to a
//! collector. With no endpoint configured the provider remains exporter-less and spans live
//! in-process only (the default behaviour preserved for unit tests and dev runs without a
//! collector).
//!
//! Per ADR-0050, the single binary install site is this module; ADR-0046's metrics/telemetry
//! boundary keeps OTLP wiring out of `nebula-metrics` (see `nebula_metrics::otlp` for the
//! metrics-side seam that mirrors this contract).

use std::{sync::Arc, time::Duration};

use nebula_metrics::{
    MetricsRegistry, OtlpInitError, OtlpMetricsConfig, OtlpMetricsExporter, OtlpMetricsGuard,
};
use opentelemetry::{KeyValue, global, trace::TracerProvider as _};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{
    Resource,
    propagation::TraceContextPropagator,
    trace::{SdkTracerProvider, Tracer},
};
use tracing_subscriber::{EnvFilter, Layer, layer::SubscriberExt, util::SubscriberInitExt};

/// Default service name reported via the `service.name` OTel resource attribute when neither
/// the env var nor a caller override sets one.
const DEFAULT_SERVICE_NAME: &str = "nebula-api";

/// Default tracer instrumentation name (the `library` name in OTel terms). Distinct from the
/// `service.name` resource attribute: the tracer name identifies *which library* produced the
/// span, while `service.name` identifies the deploying process.
const TRACER_INSTRUMENTATION_NAME: &str = "nebula-api";

/// OTel env var consumed by [`init_api_telemetry`] to opt in to OTLP span shipping.
///
/// Empty / whitespace-only / the literal `"disabled"` all map to "OTLP off", matching the
/// convention in `nebula_log::telemetry::otel::resolve_endpoint_from`. The default behaviour
/// (env unset) is exporter-less: spans get OTel ids in-process but no network traffic.
const OTLP_ENDPOINT_ENV: &str = "OTEL_EXPORTER_OTLP_ENDPOINT";

/// Optional env var that overrides the default `service.name` resource attribute. Empty values
/// fall back to the default so an operator can clear the override without unsetting the var.
const SERVICE_NAME_ENV: &str = "OTEL_SERVICE_NAME";

/// Optional env var that overrides the default OTLP metrics export interval (in seconds).
/// Non-numeric / zero / missing values fall back to [`DEFAULT_METRICS_EXPORT_INTERVAL`].
const METRICS_INTERVAL_ENV: &str = "NEBULA_METRICS_OTLP_INTERVAL_SECS";

/// Fallback metrics export interval applied when the env override is absent or invalid.
/// Mirrors the OTel SDK default of 60s while still allowing operators to tighten the loop in
/// development environments via the env var above.
const DEFAULT_METRICS_EXPORT_INTERVAL: Duration = Duration::from_mins(1);

/// Handle returned from [`init_api_telemetry`] so the binary can deterministically shut down
/// any installed `SdkTracerProvider` (and, once attached, the OTLP metrics pipeline) on
/// graceful shutdown.
///
/// When OTLP shipping is not configured the guard wraps `None` and `Drop` is a no-op; when
/// it *is* configured, dropping the guard calls `provider.shutdown()` which flushes the batch
/// span processor and tears down the tonic exporter task. Holding the guard in `main` ensures
/// spans buffered at the moment of shutdown reach the collector before the process exits.
///
/// Once a metrics registry is wired via [`TelemetryGuard::attach_metrics_exporter`], the same
/// guard owns the metrics OTLP pipeline and drops it in the same `Drop` sequence.
#[derive(Default)]
pub struct TelemetryGuard {
    provider: Option<SdkTracerProvider>,
    metrics_guard: Option<OtlpMetricsGuard>,
}

impl std::fmt::Debug for TelemetryGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TelemetryGuard")
            .field("traces_attached", &self.provider.is_some())
            .field("metrics_attached", &self.metrics_guard.is_some())
            .finish()
    }
}

impl TelemetryGuard {
    /// Returns `true` if an OTLP exporter is attached to the tracer provider.
    ///
    /// Tests and operators can use this to assert the env-gated install actually wired an
    /// exporter (as opposed to silently falling through to the exporter-less path).
    #[must_use]
    pub fn has_exporter(&self) -> bool {
        self.provider.is_some()
    }

    /// Returns `true` if the OTLP metrics pipeline is attached.
    #[must_use]
    pub fn has_metrics_exporter(&self) -> bool {
        self.metrics_guard.is_some()
    }

    /// Attach an OTLP metrics pipeline backed by `registry`, sharing the same endpoint and
    /// `service.name` as the trace exporter.
    ///
    /// No-op (and `Ok(())`) when `OTEL_EXPORTER_OTLP_ENDPOINT` is unset / opt-out, mirroring
    /// the trace-side install policy so dev environments without a collector keep working
    /// without explicit configuration.
    ///
    /// # Errors
    ///
    /// Returns [`OtlpInitError::ExporterBuild`] when the OTLP metric exporter cannot be
    /// constructed (malformed endpoint, missing tonic runtime).
    pub fn attach_metrics_exporter(
        &mut self,
        registry: Arc<MetricsRegistry>,
    ) -> Result<(), OtlpInitError> {
        let Some(endpoint) = resolve_otlp_endpoint() else {
            return Ok(());
        };
        let cfg = OtlpMetricsConfig::new(endpoint)
            .with_service_name(resolve_service_name())
            .with_export_interval(resolve_metrics_export_interval());
        let guard = OtlpMetricsExporter::install(registry, cfg)?;
        self.metrics_guard = Some(guard);
        Ok(())
    }

    /// Explicitly shut down both the tracer provider and the metrics pipeline, flushing any
    /// buffered exports.
    ///
    /// Called automatically on drop, but exposed so binaries that want a deterministic
    /// shutdown point (e.g. after the axum server returns) can drain telemetry before the
    /// process exits.
    pub fn shutdown(&mut self) {
        // Drop metrics first so the discovery task observes the stop flag before the tracer
        // pipeline goes away (the two are independent, but ordering keeps shutdown logs
        // predictable).
        if let Some(mut metrics) = self.metrics_guard.take() {
            metrics.shutdown();
        }
        if let Some(provider) = self.provider.take()
            && let Err(err) = provider.shutdown()
        {
            // The subscriber may already be torn down when shutdown runs at process exit
            // time, so `tracing::error!` is not guaranteed to surface. Route through
            // `eprintln!` to match the `nebula_log` convention for the same edge.
            eprintln!("nebula_api::TelemetryGuard: tracer provider shutdown error: {err}");
        }
    }
}

impl Drop for TelemetryGuard {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Initialise the API binary's telemetry stack:
///
/// 1. registers the W3C Trace Context text-map propagator;
/// 2. builds an [`SdkTracerProvider`], wiring an OTLP `SpanExporter` over gRPC tonic when
///    `OTEL_EXPORTER_OTLP_ENDPOINT` is set, or returning an exporter-less provider for
///    in-process-only span ids;
/// 3. installs a global `tracing` Subscriber composed of `EnvFilter` → fmt layer →
///    [`tracing_opentelemetry::OpenTelemetryLayer`];
/// 4. returns a [`TelemetryGuard`] that the caller must hold for the lifetime of the process
///    to flush buffered spans on graceful shutdown.
///
/// Idempotent: re-installing the propagator is safe (last write wins), and the Subscriber
/// install uses `try_init` so a second call (e.g. in a test harness that pre-installs one)
/// returns quietly instead of panicking. When the Subscriber install fails (already-installed
/// case) and an OTLP exporter was just built, the provider is shut down immediately to avoid
/// leaking the background batch processor task.
///
/// `RUST_LOG` is honoured by `EnvFilter`; falls back to `info` when unset or malformed.
pub fn init_api_telemetry() -> TelemetryGuard {
    global::set_text_map_propagator(TraceContextPropagator::new());

    let (provider, tracer, otlp_attached) = build_tracer_provider();

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let fmt_layer = tracing_subscriber::fmt::layer().with_filter(filter);
    let otel_layer = tracing_opentelemetry::OpenTelemetryLayer::new(tracer);

    // `RUST_LOG` gates human-readable logging only. The OpenTelemetry layer must still see
    // request spans so W3C propagation keeps working even when operators run with
    // `RUST_LOG=warn` or stricter.
    //
    // `try_init` returns `Err` when a subscriber is already installed (common in tests). That
    // is not a fatal startup failure, but the error is surfaced via `eprintln!` so operators
    // see double-init mishaps in CI logs even before any `tracing` subscriber accepts events.
    let install_result = tracing_subscriber::registry()
        .with(fmt_layer)
        .with(otel_layer)
        .try_init();

    if let Err(err) = install_result {
        eprintln!(
            "nebula_api::init_api_telemetry: subscriber already installed — re-init skipped ({err})"
        );
        // Subscriber install failed: shut the provider down immediately so the OTLP exporter
        // task does not outlive its observers. Returning an empty guard would leak the batch
        // processor. Mirrors the `nebula_log::telemetry::otel::shutdown_unused_provider` edge.
        if otlp_attached && let Err(shutdown_err) = provider.shutdown() {
            eprintln!(
                "nebula_api::init_api_telemetry: unused tracer provider shutdown error: {shutdown_err}"
            );
        }
        return TelemetryGuard::default();
    }

    TelemetryGuard {
        provider: if otlp_attached { Some(provider) } else { None },
        metrics_guard: None,
    }
}

/// Build an [`SdkTracerProvider`] (with the OTLP exporter attached when configured) and the
/// matching tracer.
///
/// Returns `(provider, tracer, otlp_attached)` so the caller can decide whether to hold the
/// provider in the guard or let it drop immediately. The exporter-less path still returns a
/// usable tracer so spans get OTel ids and the response `traceparent` header keeps working
/// even when no collector is configured.
fn build_tracer_provider() -> (SdkTracerProvider, Tracer, bool) {
    let resource = build_resource();
    let mut builder = SdkTracerProvider::builder().with_resource(resource);
    let mut otlp_attached = false;

    if let Some(endpoint) = resolve_otlp_endpoint() {
        match build_otlp_span_exporter(&endpoint) {
            Ok(exporter) => {
                // Batch export needs an active Tokio runtime — mirror the `nebula_log` fallback
                // so callers without a runtime (rare, but supported in some test harnesses) get
                // a simple exporter instead of a runtime panic.
                if tokio::runtime::Handle::try_current().is_ok() {
                    builder = builder.with_batch_exporter(exporter);
                } else {
                    builder = builder.with_simple_exporter(exporter);
                }
                otlp_attached = true;
            },
            Err(err) => {
                eprintln!(
                    "nebula_api::init_api_telemetry: failed to build OTLP span exporter for `{endpoint}` ({err}) — falling back to exporter-less tracer"
                );
            },
        }
    }

    let provider = builder.build();
    let tracer = provider.tracer(TRACER_INSTRUMENTATION_NAME);
    (provider, tracer, otlp_attached)
}

fn build_otlp_span_exporter(
    endpoint: &str,
) -> Result<opentelemetry_otlp::SpanExporter, opentelemetry_otlp::ExporterBuildError> {
    opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint)
        .build()
}

fn build_resource() -> Resource {
    let service_name = resolve_service_name();
    Resource::builder_empty()
        .with_attributes([KeyValue::new("service.name", service_name)])
        .build()
}

fn resolve_service_name() -> String {
    std::env::var(SERVICE_NAME_ENV)
        .ok()
        .and_then(|v| {
            let trimmed = v.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_owned())
            }
        })
        .unwrap_or_else(|| DEFAULT_SERVICE_NAME.to_owned())
}

fn resolve_otlp_endpoint() -> Option<String> {
    let raw = std::env::var(OTLP_ENDPOINT_ENV).ok()?;
    normalise_otlp_endpoint(&raw)
}

fn resolve_metrics_export_interval() -> Duration {
    std::env::var(METRICS_INTERVAL_ENV)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|secs| *secs > 0)
        .map_or(DEFAULT_METRICS_EXPORT_INTERVAL, Duration::from_secs)
}

/// Apply the standard opt-in/opt-out rules to a candidate OTLP endpoint string.
///
/// Empty, whitespace-only, and the literal `"disabled"` all map to `None` (opt-out). Any other
/// value is returned trimmed. Pure helper for unit testing — the env-reading wrapper sits in
/// [`resolve_otlp_endpoint`].
fn normalise_otlp_endpoint(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("disabled") {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_endpoint_is_opt_out() {
        assert_eq!(normalise_otlp_endpoint(""), None);
        assert_eq!(normalise_otlp_endpoint("   "), None);
    }

    #[test]
    fn disabled_endpoint_is_opt_out_case_insensitive() {
        assert_eq!(normalise_otlp_endpoint("disabled"), None);
        assert_eq!(normalise_otlp_endpoint("DISABLED"), None);
        assert_eq!(normalise_otlp_endpoint("  Disabled  "), None);
    }

    #[test]
    fn populated_endpoint_is_returned_trimmed() {
        assert_eq!(
            normalise_otlp_endpoint("  http://collector:4317  "),
            Some("http://collector:4317".to_owned()),
        );
    }
}
