//! API binary telemetry bootstrap (M3.5).
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
//! The tracer provider here has **no exporter**: spans live in-process only. OTLP shipping is the
//! M9.2 gate and is layered on top by `nebula-log` when operators enable it.

use opentelemetry::global;
use opentelemetry_sdk::{propagation::TraceContextPropagator, trace::SdkTracerProvider};
use tracing_subscriber::{EnvFilter, Layer, layer::SubscriberExt, util::SubscriberInitExt};

/// Initialise the API binary's telemetry stack:
///
/// 1. registers the W3C Trace Context text-map propagator;
/// 2. builds an exporter-less [`SdkTracerProvider`] so spans get OTel ids in-process;
/// 3. installs a global `tracing` Subscriber composed of `EnvFilter` → fmt layer →
///    [`tracing_opentelemetry::OpenTelemetryLayer`].
///
/// Idempotent: re-installing the propagator is safe (last write wins), and the Subscriber install
/// uses `try_init` so a second call (e.g. in a test harness that pre-installs one) returns
/// quietly instead of panicking.
///
/// `RUST_LOG` is honoured by `EnvFilter`; falls back to `info` when unset or malformed.
pub fn init_api_telemetry() {
    global::set_text_map_propagator(TraceContextPropagator::new());

    let provider = SdkTracerProvider::builder().build();
    let tracer = opentelemetry::trace::TracerProvider::tracer(&provider, "nebula-api");

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
    if let Err(err) = tracing_subscriber::registry()
        .with(fmt_layer)
        .with(otel_layer)
        .try_init()
    {
        eprintln!(
            "nebula_api::init_api_telemetry: subscriber already installed — re-init skipped ({err})"
        );
    }
}
