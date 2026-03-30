# nebula-log
Structured logging foundation for Nebula, built on `tracing` — single pipeline for dev and production.

## Invariants
- `LoggerGuard` must be held for the process lifetime. Dropping it shuts down the logger (all subsequent log calls are silently dropped).
- Only one logger can be initialized per process. Subsequent init calls after a dispatcher is set are ignored (returns a noop guard).

## Key Decisions
- `auto_init()` for zero-config dev/test (reads `NEBULA_LOG`/`RUST_LOG`).
- `init_with(Config)` for production (deterministic, no env-var surprises).
- `init_test()` is idempotent — safe to call in every test; returns noop guard if already initialized.
- Runtime log-level reload via `ReloadHandle` when `reloadable: true` in config.

## Traps
- Don't call `init_with()` in tests — use `init_test()`. Duplicate init causes a panic in tracing.
- File rolling requires the `file` feature flag. Not enabled by default.
- `LoggerGuard::noop()` is returned in tests to avoid double-init — it is not an error, it's intentional.
- Telemetry (OpenTelemetry OTLP) requires the `telemetry` feature and `OTEL_EXPORTER_OTLP_ENDPOINT` env var.

## Relations
- No nebula deps. Used by almost every crate. Wraps `tracing` macros — importing crates use `nebula_log::{info, debug, ...}` macros, not `tracing::` directly.

<!-- reviewed: 2026-03-30 — derive Classify migration -->
