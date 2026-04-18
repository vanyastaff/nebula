---
name: nebula-log
role: Structured Tracing Initialization (single logging pipeline, multi-format, multi-backend)
status: stable
last-reviewed: 2026-04-17
canon-invariants: [L2-12.5]
related: [nebula-telemetry, nebula-metrics]
---

# nebula-log

## Purpose

A workflow engine that processes credentials and sensitive data must ensure that no secret material
appears in log output, that structured logs carry consistent service/env/version fields, and that
the same configuration works in development (pretty/colored) and production (JSON/logfmt, rolling
files, OTLP). Without a shared initialization crate, each binary invents its own `tracing`
subscriber setup — different fields, different rotation policies, inconsistent Sentry/OTLP wiring.
`nebula-log` provides a single logging pipeline for all Nebula processes: one call to `auto_init`
or `init_with`, and the subscriber is configured, including the runtime reload handle if needed.

## Role

**Structured Tracing Initialization** — the single entry point for `tracing` subscriber setup
across all Nebula binaries and integration tests. Cross-cutting infrastructure (no upward
dependencies). The canon observability contract (§4.6, §12.5) is enforced at the logging
boundary: no secrets in log output, structured events with typed event kinds.

## Public API

- `auto_init() -> LogResult<LoggerGuard>` — zero-config startup (reads env, falls back to preset).
- `init() -> LogResult<LoggerGuard>` — default config.
- `init_with(Config) -> LogResult<LoggerGuard>` — fully explicit, deterministic production setup.
- `Config` — full configuration struct: `format`, `writer`, `fields`, `level`, `reloadable`, `telemetry`, etc.
- `Config::development()`, `Config::production()` — preset constructors.
- `Format` — `Pretty` / `Compact` / `Json` / `Logfmt`.
- `WriterConfig` — `Stderr` / `Stdout` / `File(path)` / `Fanout(Vec<WriterConfig>)`.
- `LoggerGuard` — RAII handle; must stay alive for the process lifetime.
- `ReloadHandle` — runtime log-level reload (when `reloadable: true`).
- `LogResult<T>` — `Result<T, LogError>` alias.
- `prelude` — convenience re-exports of `tracing` macros.

## Contract

- **[L2-§12.5]** Every `tracing::*!` macro call that touches credential or token arguments must use redacted forms. `nebula-log` provides the subscriber setup; individual call sites in other crates must not pass raw secret values to structured fields. Seam: `tracing` spans in credential-handling code paths. Test coverage: see `docs/MATURITY.md`.
- **[L1-§4.6]** Observability is a first-class contract — structured logs with consistent fields (service, env, version, instance, region) are not polish but a product invariant.

## Non-goals

- Not a metrics system — metric counters, gauges, and histograms live in `nebula-telemetry` / `nebula-metrics`.
- Not an event bus — domain event distribution lives in `nebula-eventbus`.
- Not responsible for secret redaction itself — callers must use redacted wrappers before passing values to `tracing` macros.

## Maturity

See `docs/MATURITY.md` row for `nebula-log`.

- API stability: `stable` — `auto_init`, `init_with`, `Config`, and `LoggerGuard` are in active use; runtime reload and file rolling are stable.
- OTLP (`telemetry` feature) and Sentry (`sentry` feature) integrations are functional but depend on external services; treat as `partial` until tested end-to-end in CI.

## Related

- Canon: `docs/PRODUCT_CANON.md` §4.6 (Observability pillar), §12.5 (secrets and auth), `docs/OBSERVABILITY.md`.
- Siblings: `nebula-telemetry` (metric primitives), `nebula-metrics` (metric export).

## Appendix: Feature flags

| Feature | What it enables |
|---|---|
| `default` | `ansi`, `async` |
| `file` | File writer + rolling support |
| `log-compat` | Bridge `log` crate events into `tracing` |
| `observability` | Metrics helpers + hook APIs |
| `telemetry` | OpenTelemetry OTLP tracing |
| `sentry` | Sentry integration |
| `full` | All major capabilities |

## Appendix: Environment variables

| Variable | Purpose |
|---|---|
| `NEBULA_LOG` / `RUST_LOG` | Log level / filter |
| `NEBULA_LOG_FORMAT` | `pretty\|compact\|json\|logfmt` |
| `NEBULA_LOG_TIME`, `NEBULA_LOG_SOURCE`, `NEBULA_LOG_COLORS` | Display options |
| `NEBULA_SERVICE`, `NEBULA_ENV`, `NEBULA_VERSION`, `NEBULA_INSTANCE`, `NEBULA_REGION` | Structured field defaults |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | OTLP endpoint (telemetry feature) |
| `SENTRY_DSN`, `SENTRY_ENV`, `SENTRY_RELEASE`, `SENTRY_TRACES_SAMPLE_RATE` | Sentry (sentry feature) |

Extended documentation: `crates/log/docs/README.md`.
