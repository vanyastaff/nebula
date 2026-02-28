# Interactions

## Ecosystem Map (Current + Planned)

`nebula-log` is a cross-cutting infra crate. It has no domain dependencies; all workflow/engine/runtime crates depend on it.

## Existing Crates

- **core:** Uses log for diagnostics; may provide typed IDs for observability contexts (P-003)
- **action:** Emits structured logs/traces; log is optional, not part of action trait contract
- **config:** Uses log for startup/config diagnostics
- **credential:** Uses log for credential lifecycle events
- **expression:** Uses log for evaluation diagnostics
- **memory:** Optional `logging` feature; uses log when enabled
- **resilience:** Uses log for circuit breaker, retry, rate-limit events
- **validator, storage, resource, system:** May use log when present in workspace

## Planned Crates

- **engine / runtime / worker:** Will consume log for execution spans, node lifecycle, workflow events
- **api / cli / ui:** Will use log for request/response and user action traces
- **metrics / telemetry:** May be separate crates or remain feature-gated in log

## Downstream Consumers

- **core, action, config, credential, expression, memory, resilience:** Expect `auto_init`/`init_with`, tracing macros, optional `ObservabilityHook` integration
- **engine/runtime (future):** Expect `OperationTracker`, `ExecutionContext`, `NodeContext` for workflow observability

## Upstream Dependencies

- **tracing, tracing-subscriber:** Hard contract; init and span/event emission
- **tokio (optional):** Task-local context propagation
- **opentelemetry, sentry (optional):** Telemetry export; fallback: no-op when disabled

## Interaction Matrix

| This crate <-> Other | Direction | Contract | Sync/Async | Failure handling | Notes |
|----------------------|-----------|----------|------------|------------------|-------|
| core | out | macros, optional typed IDs for contexts | sync | N/A | |
| action | out | macros, optional hooks | sync | log never fails action | action treats log as optional |
| config | out | init, Config | sync | LogError on init fail | config may init log early |
| credential | out | macros, spans | sync | N/A | |
| expression | out | macros | sync | N/A | |
| memory | out | optional feature | sync | N/A | |
| resilience | out | macros, spans | sync | N/A | |

## Runtime Sequence

1. Application calls `auto_init()` or `init_with(config)`
2. `LoggerBuilder` builds tracing subscriber with fmt/filter/telemetry layers
3. `LoggerGuard` is returned; subscriber is set as global default
4. Crates emit `info!`, `span!`, `emit_event` etc.
5. On shutdown, `shutdown_hooks()` flushes observability hooks

## Cross-Crate Ownership

- **log owns:** tracing init, format, writers, observability registry, hook dispatch
- **Consumers own:** when to init, what to log, custom hooks
- **No shared persistence:** log writes to configured writers; no cross-crate state beyond global subscriber

## Failure Propagation

- Init failures return `LogError`; caller decides retry or exit
- Hook panics are caught; event emission continues
- Writer I/O errors surface as `LogError::Io` during init; runtime write failures depend on writer implementation

## Versioning and Compatibility

- **Compatibility promise:** Patch/minor releases preserve config schema and public API
- **Breaking-change protocol:** Deprecation first, then removal in next major
- **Deprecation window:** Minimum 6 months (see MIGRATION.md)

## Contract Tests Needed

- Config serialization/deserialization snapshot tests
- Init with each preset (dev, prod, env) succeeds
- Hook panic does not abort event emission
- Context propagation across `.await` in async tests
