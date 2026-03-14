//! Example: configuring nebula-log with OpenTelemetry OTLP export.
//!
//! This example shows how to connect `nebula-log` to an OpenTelemetry
//! collector (e.g. the OpenTelemetry Collector, Jaeger, Grafana Tempo, or any
//! backend that supports OTLP/gRPC).
//!
//! # Enabling the feature
//!
//! Add to `Cargo.toml`:
//!
//! ```toml
//! [dependencies]
//! nebula-log = { version = "…", features = ["telemetry"] }
//! ```
//!
//! # Running with a live collector
//!
//! ```text
//! OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4317 \
//!   cargo run --example otlp_setup -p nebula-log --features telemetry
//! ```
//!
//! Without the env variable the endpoint defaults to `http://localhost:4317`.
//! Set it to `"disabled"` to skip OTLP initialisation entirely (useful in CI).
//!
//! # Trace correlation
//!
//! When OTLP is active, every `tracing::span!` / `#[instrument]` call is
//! correlated with an OpenTelemetry trace.  Structured fields on spans become
//! OTel span attributes, and W3C `traceparent` headers are propagated
//! automatically via `global::set_text_map_propagator`.

use nebula_log::{Config, Format, TelemetryConfig, WriterConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // ── 1. Read collector endpoint from the environment ───────────────────────
    //
    // `OTEL_EXPORTER_OTLP_ENDPOINT` is the standard variable used by all
    // OTLP-compatible tooling.  Set it to "disabled" to skip OTLP init.
    let otlp_endpoint =
        std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").unwrap_or_else(|_| "disabled".to_string());

    tracing::debug!(
        endpoint = %otlp_endpoint,
        "OTLP endpoint resolved from environment"
    );

    // ── 2. Build a production config with OTLP tracing ───────────────────────

    let mut cfg = Config::production();
    cfg.format = Format::Json;
    cfg.writer = WriterConfig::Stderr;

    // Global resource attributes (OTel semantic conventions).
    cfg.fields.service = Some("my-service".to_string());
    cfg.fields.env = Some("production".to_string());
    cfg.fields.version = Some(env!("CARGO_PKG_VERSION").to_string());

    // Attach telemetry config.  `otlp_endpoint: None` falls back to the
    // `OTEL_EXPORTER_OTLP_ENDPOINT` env var (see `DefaultTelemetryConfig`).
    cfg.telemetry = Some(TelemetryConfig {
        otlp_endpoint: Some(otlp_endpoint), // "disabled" → no-op
        service_name: "my-service".to_string(),
        sampling_rate: 0.1, // sample 10 % of traces in production
    });

    // ── 3. Initialise — this builds and registers the OTLP tracing layer ──────
    let _guard = nebula_log::init_with(cfg)?;

    // ── 4. Emit a traced span ─────────────────────────────────────────────────
    //
    // With a live collector, this span will appear in Jaeger / Tempo under the
    // service name "my-service".
    let span = tracing::info_span!("process_request", request_id = "req-001");
    let _entered = span.enter();

    tracing::info!(user_id = 42, "processing request");

    {
        let child = tracing::debug_span!("database_query", table = "workflows");
        let _c = child.enter();
        tracing::debug!("running SELECT query");
        // … real work here …
    }

    tracing::info!("request complete");

    // The `_guard` drop triggers graceful shutdown of the OTLP batch exporter,
    // flushing any buffered spans before the process exits.
    Ok(())
}
