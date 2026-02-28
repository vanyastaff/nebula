# API Surface Contract: nebula-log

**Branch**: `001-log-crate-spec` | **Date**: 2026-02-28

## Public Types (Stable)

### Config

- `Config` — main config struct
- `Config::schema_version` — schema compatibility marker (current: `1`)
- `Format` — Human | Json | Logfmt
- `Level` — Trace | Debug | Info | Warn | Error
- `WriterConfig` — Stderr | Stdout | File { ... } | Multi { policy, writers }
- `DestinationFailurePolicy` — FailFast | BestEffort | PrimaryWithFallback *(new)*
- `Rolling` — Never | Hourly | Daily | Size(u64)
- `DisplayConfig` — display options
- `Fields` — global enrichment

### Builder

- `LoggerBuilder` — builder for subscriber
- `LoggerGuard` — guard holding subscriber + guards

### Observability

- `ObservabilityEvent` — trait for events
- `ObservabilityHook` — trait for hooks
- `HookPolicy` — Inline | Bounded { timeout_ms, queue_capacity } *(new)*
- `emit_event`, `register_hook`, `shutdown_hooks`

### Init

- `Config::from_env()` — env-derived config
- `Config::development()`, `Config::production()`, `Config::test()` — presets
- `init_logging(config)` — initialize from config

## Compatibility Policy

- Minor: additive only; no removals
- Deprecation: 12 months before removal
- Major: breaking changes documented with migration guide
