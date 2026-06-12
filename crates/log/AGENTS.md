# nebula-log — Agent orientation
> Agent quick-map for `crates/log/`. Full design: `README.md`. Repo-wide rules: root `AGENTS.md`.

**Purpose:** Single `tracing` subscriber-init pipeline for all Nebula binaries — one `auto_init`/`init_with` call wires format, writers, structured fields, runtime reload, and optional OTLP/Sentry.
**Layer:** Cross-cutting — no upward deps; importable from any tier (root AGENTS.md -> Layered Dependency Map).

## Commands
- `cargo check -p nebula-log`
- `cargo nextest run -p nebula-log`  ·  doctests: `cargo test -p nebula-log --doc`
- Feature-gated paths: `cargo check -p nebula-log --features full` (or `file` / `telemetry` / `sentry` / `log-compat`); bench: `task bench:crate CRATE=nebula-log` (`benches/log_hot_path.rs`)

## Key files
- `src/lib.rs` — public API + `auto_init`/`init`/`init_with`/`init_test`; idempotent-init logic (#379 no-op guard when a dispatcher already set)
- `src/builder/mod.rs` — `LoggerBuilder`, `LoggerGuard`, `ReloadHandle`; `build_startup` resolution (`explicit > env > preset`)
- `src/config/` — `Config`, `Format`, `WriterConfig`, presets (`development`/`production`/`test`), env resolution, `Fields`
- `src/observability/` — typed `ObservabilityEvent`/hooks/`OperationTracker`, semantic spans (canon §4.6/§12.5 boundary)
- `src/telemetry/` — OTLP (`otel.rs`) + Sentry (`sentry.rs`), gated on `telemetry`/`sentry`
- `src/writer.rs`, `src/format.rs`, `src/timing.rs` — writer backends, formatters, `Timer`/`Timed`

## Conventions & never-do
- This crate sets up the subscriber only; it does NOT redact secrets — callers must pass redacted forms to `tracing::*!` (canon §12.5). Never log raw credential/token values from here.
- Not a metrics system (`nebula-metrics`) and not an event bus (`nebula-eventbus`) — don't add either here.
- `LoggerGuard` is RAII; it must stay alive for the process lifetime. Duplicate init returns a no-op guard / `LogError::AlreadyInitialized` — keep init idempotent, never panic.
- `#![forbid(unsafe_code)]` and `#![warn(missing_docs)]` — every public item needs docs; sentry uses `rustls` only (native-tls banned in `deny.toml`).
- Library code uses typed `thiserror`/`LogError`; no panicking unwrap/expect/panic in lib code.

## See also
- `README.md` — full design · `docs/README.md` (extended internals) · canon `docs/PRODUCT_CANON.md` §4.6/§12.5, `docs/OBSERVABILITY.md` · sibling `nebula-metrics`
