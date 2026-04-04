# nebula-telemetry
Execution event bus, TelemetryService trait, W3C trace context, and metrics primitives.

## Module Structure
- `trace` — W3C TraceContext, TraceId, SpanId (distributed tracing identity)
- `event` — EventBus wrapper, ExecutionEvent, eventbus re-exports (EventFilter, SubscriptionScope, etc.)
- `metrics` — Counter, Gauge, Histogram, MetricsRegistry (in-memory primitives)
- `labels` — LabelInterner, LabelSet, MetricKey (lasso-backed string interning)
- `service` — TelemetryService trait, NoopTelemetry, ProductionTelemetry
- `error` — TelemetryError (Io)

## Invariants
- Events are **projections**, not the source of truth. The execution store (nebula-storage) is the source of truth.
- `NoopTelemetry` must be used in unit tests. `ProductionTelemetry` requires a running Tokio runtime and event bus.
- `EventBus` and `MetricsRegistry` are cheaply cloneable (Arc-backed internally). Callers should `.clone()` from the TelemetryService reference — no `_arc` methods.

## Key Decisions
- `TelemetryService` trait has 2 methods: `event_bus()`, `metrics()`. Inject via DI.
- `ExecutionEvent` = typed execution lifecycle events (started, completed, failed, node transitions).
- `TraceContext` = W3C trace context (trace ID + span ID + sampling flag) for distributed tracing.
- `LabelInterner` / `LabelSet` = `lasso`-backed string interning for metric label keys/values (zero-copy dimensions).
- In-memory primitives (`Counter`, `Gauge`, `Histogram`, `MetricsRegistry`) live here; nebula-metrics adds naming + export.
- Recorder module removed (2026-04-04): had zero external consumers. If resource call recording is needed later, design it at the resource layer.

## Traps
- `nebula_telemetry::EventBus` is a wrapper around `nebula_eventbus::EventBus<ExecutionEvent>`. Don't create a raw `nebula_eventbus::EventBus<ExecutionEvent>` directly — use `nebula_telemetry::EventBus`.
- Metric names must use `nebula_` prefix — enforced by convention, not by code.
- Eventbus types (EventFilter, SubscriptionScope, etc.) are re-exported from `nebula_telemetry::event`, not the crate root.

## Relations
- Wraps nebula-eventbus. Used by nebula-metrics (re-exports Counter/Gauge/Histogram).

<!-- reviewed: 2026-04-04 — removed recorder module, async-trait dep, nebula-core dep -->
