# nebula-telemetry
Execution event bus, TelemetryService trait, W3C trace context, and buffered call recording.

## Module Structure
- `trace` — W3C TraceContext, TraceId, SpanId (distributed tracing identity)
- `recorder` — Recorder trait, recording types (ResourceUsageRecord, CallRecord), BufferedRecorder, RecordSink
- `event` — EventBus wrapper, ExecutionEvent, eventbus re-exports (EventFilter, SubscriptionScope, etc.)
- `metrics` — Counter, Gauge, Histogram, MetricsRegistry (in-memory primitives)
- `labels` — LabelInterner, LabelSet, MetricKey (lasso-backed string interning)
- `service` — TelemetryService trait, NoopTelemetry, ProductionTelemetry
- `error` — TelemetryError (RecorderClosed, Io)

## Invariants
- Events are **projections**, not the source of truth. The execution store (nebula-storage) is the source of truth.
- `NoopTelemetry` must be used in unit tests. `ProductionTelemetry` requires a running Tokio runtime and event bus.
- `EventBus` and `MetricsRegistry` are cheaply cloneable (Arc-backed internally). Callers should `.clone()` from the TelemetryService reference — no `_arc` methods.

## Key Decisions
- `TelemetryService` trait has 3 methods: `event_bus()`, `metrics()`, `execution_recorder()`. Inject via DI.
- `ExecutionEvent` = typed execution lifecycle events (started, completed, failed, node transitions).
- `TraceContext` = W3C trace context (trace ID + span ID + sampling flag) for distributed tracing.
- `BufferedRecorder` = background-buffered resource call recording (logs `CallRecord` entries).
- `LabelInterner` / `LabelSet` = `lasso`-backed string interning for metric label keys/values (zero-copy dimensions).
- In-memory primitives (`Counter`, `Gauge`, `Histogram`, `MetricsRegistry`) live here; nebula-metrics adds naming + export.

## Traps
- `nebula_telemetry::EventBus` is a wrapper around `nebula_eventbus::EventBus<ExecutionEvent>`. Don't create a raw `nebula_eventbus::EventBus<ExecutionEvent>` directly — use `nebula_telemetry::EventBus`.
- Metric names must use `nebula_` prefix — enforced by convention, not by code.
- Eventbus types (EventFilter, SubscriptionScope, etc.) are re-exported from `nebula_telemetry::event`, not the crate root.

## Relations
- Wraps nebula-eventbus. Used by nebula-metrics (re-exports Counter/Gauge/Histogram), nebula-resource (re-exports CallRecord types).

<!-- reviewed: 2026-03-30 — derive Classify migration -->
<!-- reviewed: 2026-04-02 — pre-existing modifications, no architectural changes this session -->
