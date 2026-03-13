# Integration Guide

How to integrate `nebula-log` into internal crates and applications.

## Typical integration points

| Layer | Integration |
|---|---|
| `api`/entry binary | Initialize once at process startup |
| `engine`/`runtime` | Emit structured spans/events only (no re-init) |
| library crates | Use tracing macros, avoid global init |

## Startup pattern

```rust
use nebula_log::prelude::*;

fn main() -> LogResult<()> {
    let _guard = nebula_log::auto_init()?;
    info!(service = "api", "service started");
    run();
    Ok(())
}

fn run() {
    // application lifecycle
}
```

## Rules for internal crates

1. Initialize logging only in top-level binary/bootstrap crate.
2. Downstream crates should only emit logs/spans.
3. Keep `LoggerGuard` alive until shutdown.
4. Prefer explicit config in production services.

## Context and event usage

- Use structured fields (`key = value`) for machine parsing.
- Use operation tracker/events for business operation boundaries.
- Prefer stable event names/fields for downstream analytics.

---

## nebula-log + nebula-telemetry

`nebula-log`'s observability hook system is the integration point for
`nebula-telemetry` and other telemetry backends.

### Wiring steps

1. Implement `ObservabilityHook` in your telemetry adapter.
2. Register it once at startup with `register_hook(Arc::new(my_hook))`.
3. Every `emit_event` call dispatches to all registered hooks.

```rust
use nebula_log::observability::{ObservabilityEvent, ObservabilityHook, register_hook};
use std::sync::Arc;

struct TelemetryAdapter; // forwards to nebula-telemetry or Prometheus

impl ObservabilityHook for TelemetryAdapter {
    fn on_event(&self, event: &dyn ObservabilityEvent) {
        // forward to your backend here
    }
}

// In your bootstrap code:
register_hook(Arc::new(TelemetryAdapter));
```

### Event bus subscription pattern

If your backend uses `nebula-eventbus`, spawn a subscriber task that
forwards `ObservabilityEvent` payloads to the bus:

```rust
// Pseudo-code — adapt to your EventBus API
let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
register_hook(Arc::new(ChannelHook { tx }));

tokio::spawn(async move {
    while let Some(event) = rx.recv().await {
        event_bus.publish(event).await;
    }
});
```

See `examples/telemetry_integration.rs` for a runnable end-to-end demo.

---

## nebula-log + OpenTelemetry OTLP

Enable distributed tracing with an OTLP-compatible collector (Jaeger,
Grafana Tempo, the OpenTelemetry Collector, etc.).

### Feature flag

```toml
# Cargo.toml
[dependencies]
nebula-log = { version = "…", features = ["telemetry"] }
```

### Configuration

```rust
use nebula_log::{Config, Format, TelemetryConfig, WriterConfig};

let mut cfg = Config::production();
cfg.format = Format::Json;
cfg.fields.service = Some("my-service".to_string());
cfg.telemetry = Some(TelemetryConfig {
    otlp_endpoint: Some("http://localhost:4317".to_string()),
    service_name: "my-service".to_string(),
    sampling_rate: 0.1,
});
let _guard = nebula_log::init_with(cfg)?;
```

### Endpoint resolution order

1. `TelemetryConfig::otlp_endpoint` (if `Some` and non-empty)
2. `OTEL_EXPORTER_OTLP_ENDPOINT` environment variable
3. Default: `http://localhost:4317`

Set the endpoint to `"disabled"` to skip OTLP initialisation entirely
(useful in CI or unit tests).

### Trace correlation

Every `tracing::span!` / `#[instrument]` becomes an OTel span.
Structured fields become OTel span attributes.
W3C `traceparent` propagation is enabled automatically.

See `examples/otlp_setup.rs` for a runnable configuration example.

---

## nebula-log + Sentry

Enable automatic error capturing and breadcrumb forwarding to Sentry.

### Feature flag

```toml
# Cargo.toml
[dependencies]
nebula-log = { version = "…", features = ["sentry"] }
```

### Environment variables

| Variable | Purpose | Default |
|----------|---------|---------|
| `SENTRY_DSN` | Sentry project DSN. Leave unset to disable Sentry. | — |
| `SENTRY_ENV` | Environment tag (`production`, `staging`, …). | `development` |
| `SENTRY_RELEASE` | Release identifier. | `CARGO_PKG_VERSION` |
| `SENTRY_TRACES_SAMPLE_RATE` | Performance-monitoring sample rate (0–1). | `0.1` |

No code changes are needed; set `SENTRY_DSN` and Sentry is active.

### Filter policy (hardcoded)

| tracing level | Sentry action |
|---|---|
| `ERROR` | Creates a Sentry **issue** |
| `WARN` | Records a Sentry **breadcrumb** |
| `INFO` / `DEBUG` / `TRACE` | Ignored by Sentry |

### Breadcrumb forwarding

Structured fields attached to `warn!` / `error!` calls are forwarded as
Sentry extra data and can be used for debugging in the Sentry UI.

See `examples/sentry_setup.rs` for a runnable configuration example.

---

## Feature Flags Reference

| Feature | What it enables | Key dependency |
|---------|----------------|----------------|
| `default` | ANSI colour output, async writer | `tokio` |
| `ansi` | Coloured terminal output | — |
| `async` | Async (Tokio-backed) non-blocking writer | `tokio` |
| `file` | File writer with rolling support | `tracing-appender`, `flake2` |
| `log-compat` | Bridges `log`-crate events into `tracing` | `tracing-log` |
| `observability` | Metrics helpers and hook APIs | `metrics` |
| `telemetry` | OpenTelemetry OTLP distributed tracing | `opentelemetry-otlp`, `tracing-opentelemetry` |
| `sentry` | Sentry error and breadcrumb capture | `sentry`, `sentry-tracing` |
| `full` | All of the above | All of the above |

---

## Telemetry/Sentry integration (quick reference)

- Enable feature flags explicitly in Cargo:
  - `telemetry` for OTLP export
  - `sentry` for Sentry
- Validate env variables at startup in deployment manifests.
- Never enable both `telemetry` and Sentry in tests; use `auto_init()` with
  `OTEL_EXPORTER_OTLP_ENDPOINT=disabled` and no `SENTRY_DSN`.

## Anti-patterns

- Multiple initialization attempts from multiple crates.
- Mixing unstructured free-text logs where structured fields are expected.
- Relying on default env parsing in critical production paths without validation.
