# Architecture

## Positioning

`nebula-log` is a shared infra crate. It should be depended on broadly, but keep domain-agnostic boundaries.

Dependency direction:
- domain/runtime crates -> `nebula-log`
- `nebula-log` should not depend on workflow domain logic

## Internal Modules

- `core/`
  - `LogError`, `LogResult`, result extension traits
- `config/`
  - config schema (`Config`, `Format`, `Level`, writer/display/fields presets)
- `builder/`
  - subscriber construction, filter reload support, telemetry attachment
- `writer.rs`
  - writer instantiation (stderr/stdout/file/multi)
- `layer/`
  - context and field layers injected into tracing pipeline
- `timing.rs` + `macros.rs`
  - low-overhead timing helpers for sync/async paths
- `observability/`
  - event model + hook registry + context/resource-aware hooks
- `metrics/` (feature-gated)
  - metrics facade and timing helpers
- `telemetry/` (feature-gated)
  - OpenTelemetry/Sentry integration

## Runtime Properties

- no `unsafe` (`#![forbid(unsafe_code)]`)
- fast read path for hook emission (`ArcSwap` snapshot reads)
- task-local context propagation with `async` feature
- panic isolation for hooks (`catch_unwind` on hook callbacks)

## Known Design Constraints

- `WriterConfig::Multi` currently falls back to first writer only.
- size-based rolling is declared but not implemented.
- observability registry is global; tests need serialization around shared state.
