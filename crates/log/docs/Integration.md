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

## Telemetry/Sentry integration

- Enable feature flags explicitly in Cargo:
  - `telemetry` for OTLP
  - `sentry` for Sentry
- Validate env variables at startup in deployment manifests.

## Anti-patterns

- Multiple initialization attempts from multiple crates.
- Mixing unstructured free-text logs where structured fields are expected.
- Relying on default env parsing in critical production paths without validation.
