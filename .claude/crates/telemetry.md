# nebula-telemetry
Execution event bus, TelemetryService trait, W3C trace context, and buffered call recording.

## Invariants
- Events are **projections**, not the source of truth. The execution store (nebula-storage) is the source of truth.
- `NoopTelemetry` must be used in unit tests. `ProductionTelemetry` requires a running Tokio runtime and event bus.

## Key Decisions
- `TelemetryService` trait is the abstraction — inject it via DI, never construct `ProductionTelemetry` directly in tests.
- `ExecutionEvent` = typed execution lifecycle events (started, completed, failed, node transitions).
- `TraceContext` = W3C trace context (trace ID + span ID + sampling flag) for distributed tracing.
- `BufferedRecorder` = background-buffered resource call recording (logs `CallRecord` entries).
- `LabelInterner` / `LabelSet` = `lasso`-backed string interning for metric label keys/values (zero-copy dimensions).
- In-memory primitives (`Counter`, `Gauge`, `Histogram`, `MetricsRegistry`) live here; nebula-metrics adds naming + export.

## Traps
- `nebula_telemetry::EventBus` is a re-export of `nebula_eventbus::EventBus` wrapped with `ExecutionEvent`. Don't create a raw `nebula_eventbus::EventBus<ExecutionEvent>` directly — use `nebula_telemetry::EventBus`.
- Metric names must use `nebula_` prefix — enforced by convention, not by code.

## Relations
- Wraps nebula-eventbus. Used by nebula-metrics (re-exports Counter/Gauge/Histogram), nebula-resource (re-exports CallRecord types).
