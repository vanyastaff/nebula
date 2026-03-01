# nebula-log

Structured logging and observability foundation for Nebula, built on top of `tracing`.

## Capabilities

- startup presets (`development`, `production`, env overrides)
- formats: `pretty`, `compact`, `json`, `logfmt`
- writer backends: stderr/stdout/file, fanout with failure policy
- rolling files: hourly/daily/size/size+retention
- timing utilities and macros
- observability hooks/events with typed event kinds
- optional telemetry integrations: OpenTelemetry OTLP and Sentry

## Quick Start

```rust
use nebula_log::prelude::*;

fn main() -> LogResult<()> {
    let _guard = nebula_log::auto_init()?;
    info!(service = "api", "server started");
    Ok(())
}
```

`LoggerGuard` must stay alive for the process lifetime (or until you intentionally shut logging down).

## Explicit Configuration

```rust
use nebula_log::{Config, Format, WriterConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut cfg = Config::production();
    cfg.format = Format::Json;
    cfg.writer = WriterConfig::Stderr;
    cfg.fields.service = Some("nebula-api".to_string());
    cfg.fields.env = Some("prod".to_string());

    let _guard = nebula_log::init_with(cfg)?;
    Ok(())
}
```

## Feature Flags

- `default`: `ansi`, `async`
- `file`: file writer + rolling support
- `log-compat`: bridge `log` crate events into `tracing`
- `observability`: metrics helpers + hook APIs
- `telemetry`: OpenTelemetry OTLP tracing
- `sentry`: Sentry integration
- `full`: enables all major capabilities

## Telemetry and Sentry

- OTLP endpoint is read from config telemetry section or `OTEL_EXPORTER_OTLP_ENDPOINT`.
- Sentry is enabled when `sentry` feature is active and `SENTRY_DSN` is set.
- Useful env vars:
  - `SENTRY_DSN`
  - `SENTRY_ENV`
  - `SENTRY_RELEASE`
  - `SENTRY_TRACES_SAMPLE_RATE`

## Environment Variables

- `NEBULA_LOG` or `RUST_LOG`: log level/filter
- `NEBULA_LOG_FORMAT`: `pretty|compact|json|logfmt`
- `NEBULA_LOG_TIME`, `NEBULA_LOG_SOURCE`, `NEBULA_LOG_COLORS`
- `NEBULA_SERVICE`, `NEBULA_ENV`, `NEBULA_VERSION`, `NEBULA_INSTANCE`, `NEBULA_REGION`

## Development Checks

```bash
cargo test -p nebula-log
cargo clippy -p nebula-log --all-targets --all-features --locked -- -D warnings
```

## Internal Documentation

For architecture decisions, API contracts, reliability notes, and roadmap:
- [docs/README.md](./docs/README.md)
