# nebula-log

`nebula-log` is the logging and observability foundation for Nebula.

It provides:
- tracing initialization (`auto_init`, `init`, `init_with`)
- configurable formatting/writers (`compact`, `pretty`, `json`, `logfmt`)
- async-safe context propagation
- operation timing primitives and macros
- pluggable observability event hooks
- optional metrics/telemetry integrations

## Role in Platform

For a Rust n8n-like automation platform, this crate is the cross-cutting observability layer used by `engine`, `runtime`, `api`, `worker`, plugins, and SDK paths.

## Feature Flags

- `default`: `ansi`, `async`
- `file`: file writer + rolling support
- `log-compat`: bridge `log` ecosystem into tracing
- `observability`: metrics + hooks
- `telemetry`: OpenTelemetry pipeline
- `sentry`: Sentry integration
- `full`: enables all major capabilities

## Main API

- Builder/config:
  - `LoggerBuilder`
  - `Config`, `Format`, `Level`, `WriterConfig`, `Rolling`
- Initialization:
  - `auto_init()`
  - `init()`
  - `init_with(config)`
- Context/timing:
  - `Context`
  - `Timer`, `TimerGuard`, `Timed`
  - macros: `timed!`, `async_timed!`, `measure!`, `with_context!`
- Observability:
  - `ObservabilityEvent`, `ObservabilityHook`
  - `register_hook`, `emit_event`, `shutdown_hooks`
  - `OperationTracker` and operation lifecycle events

## Document Set

- [ARCHITECTURE.md](ARCHITECTURE.md)
- [API.md](API.md)
- [DECISIONS.md](DECISIONS.md)
- [ROADMAP.md](ROADMAP.md)
- [PROPOSALS.md](PROPOSALS.md)
