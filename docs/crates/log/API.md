# API Reference (Human-Oriented)

## Initialization

- `auto_init() -> LogResult<LoggerGuard>`
  - chooses env/dev/prod config automatically
- `init() -> LogResult<LoggerGuard>`
  - default compact+info setup
- `init_with(config: Config) -> LogResult<LoggerGuard>`
  - full custom setup

## Config Surface

- `Config`
  - `level: String`
  - `format: Format` (`Pretty | Compact | Json | Logfmt`)
  - `writer: WriterConfig`
  - `display: DisplayConfig`
  - `fields: Fields`
  - `reloadable: bool`
  - optional telemetry config (feature-gated)
- `WriterConfig`
  - `Stderr`, `Stdout`, `File{...}` (feature `file`), `Multi(Vec<WriterConfig>)`
- `Rolling`
  - `Never | Hourly | Daily | Size(u64)` (`Size` currently not implemented)

## Errors

- `LogError`
  - `Config`, `Filter`, `Io`, `Telemetry`, `Internal`
- `LogResult<T>`
- `LogResultExt`, `LogIoResultExt`

## Context and Timing

- `Context` (request/user/session + custom fields)
- `Timer`, `TimerGuard`, `Timed` / `TimedFuture`
- Macros:
  - `timed!`
  - `async_timed!`
  - `timed_span!`
  - `measure!`
  - `with_context!`
  - `log_error!`

## Observability

- Traits:
  - `ObservabilityEvent`
  - `ObservabilityHook`
  - `ResourceAwareHook`
- Registry:
  - `register_hook`
  - `emit_event`
  - `shutdown_hooks`
- Built-ins:
  - `LoggingHook`
  - `MetricsHook` (feature `observability`)
  - `OperationStarted`, `OperationCompleted`, `OperationFailed`, `OperationTracker`
- Context model:
  - `GlobalContext`
  - `ExecutionContext`
  - `NodeContext`
  - `ResourceMap`
  - `current_contexts()`

## Prelude

`nebula_log::prelude::*` re-exports common logging macros/types and observability primitives for fast integration in other crates.
