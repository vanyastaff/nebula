# Architecture

## Problem Statement

- **Business problem:** Workflow engine and runtime need observability for execution lifecycle, node-level metrics, and failure tracking without blocking the hot path.
- **Technical problem:** Provide a lightweight, pluggable telemetry subsystem that supports event fan-out and metrics recording with minimal overhead and no external dependencies for MVP.

## Current Architecture

### Module Map

```
nebula-telemetry/
├── event.rs    — EventBus, EventSubscriber, ExecutionEvent
├── metrics.rs  — Counter, Gauge, Histogram, MetricsRegistry, NoopMetricsRegistry
├── service.rs  — TelemetryService trait, NoopTelemetry
└── lib.rs      — Re-exports
```

### Data/Control Flow

1. **Event flow:** Engine/runtime call `event_bus.emit(ExecutionEvent)` → broadcast channel → all subscribers receive copy; if no subscribers, event is dropped (fire-and-forget).
2. **Metrics flow:** Engine/runtime call `metrics.counter("name").inc()` etc. → atomic updates in registry; values stored in-memory.
3. **Service facade:** `TelemetryService` trait exposes `event_bus()` and `metrics()`; `NoopTelemetry` provides both with no external side effects.

### Known Bottlenecks

- **Histogram:** Stores every observation in a `Vec<f64>`; unbounded growth under high throughput.
- **Broadcast:** Lagging subscribers cause `RecvError::Lagged`; subscriber must catch up or skip; no backpressure to emitter.
- **No export:** Metrics never leave process; no Prometheus scrape or OTLP push.

## Target Architecture

### Target Module Map

- Add `export/` module (optional feature) for Prometheus/OTLP.
- Add `histogram_bounded` or replace with bucketed implementation.
- Keep `event`, `metrics`, `service` as core; export as additive.

### Public Contract Boundaries

- `TelemetryService`: trait; consumers depend on `Arc<dyn TelemetryService>`.
- `ExecutionEvent`: enum; schema is additive-only (new variants allowed).
- `MetricsRegistry`: creates named metrics; names are integration contract.

### Internal Invariants

- Events are projections; `ExecutionRepo` (ports) is source of truth.
- Emit never blocks; subscribers must not block hot path.
- Metrics are best-effort; recording failures must not affect execution.

## Design Reasoning

### Key Trade-off 1: Fire-and-forget vs reliable delivery

- **Chosen:** Fire-and-forget (broadcast, drop when no subscribers).
- **Rationale:** Telemetry must not block execution; events are for dashboards/audit, not critical path.
- **Consequence:** Subscribers can miss events; use ExecutionRepo for authoritative state.

### Key Trade-off 2: In-memory metrics vs external exporter

- **Chosen:** In-memory first; exporter as Phase 2.
- **Rationale:** MVP needs zero external deps; desktop/CLI use case.
- **Consequence:** Production deployments need exporter implementation.

### Rejected Alternatives

- **Reliable event queue (e.g. Kafka):** Overkill for MVP; adds infra dependency.
- **Prometheus metrics crate directly:** Would force Prometheus as hard dep; we want pluggable backends.

## Comparative Analysis

Sources: n8n, Node-RED, Activepieces, Temporal/Prefect/Airflow.

| Pattern | Verdict | Rationale |
|---------|---------|-----------|
| Execution lifecycle events | **Adopt** | n8n, Temporal use similar event models; standard for workflow observability |
| In-memory metrics with optional export | **Adopt** | Matches Node-RED, Activepieces; export via adapter |
| OpenTelemetry spans in engine | **Defer** | nebula-log already has OTel; telemetry crate focuses on events/metrics |
| Prometheus-native metrics | **Defer** | Add as exporter implementation, not core API |

## Breaking Changes (if any)

- None planned for current API.
- Future: Histogram replacement may require migration if we introduce bounded/bucketed API.

## Open Questions

- Q1: Should EventBus support backpressure (e.g. when all subscribers lag)?
- Q2: Standard metric names convention (e.g. `nebula_*` prefix, labels)?
