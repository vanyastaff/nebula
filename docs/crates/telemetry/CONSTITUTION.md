# nebula-telemetry Constitution

> **Version**: 1.0.0 | **Created**: 2026-03-01

---

## Platform Role

Engine and runtime need to emit execution lifecycle events (Started, NodeCompleted, NodeFailed, Completed) and metrics (counters, gauges, histograms) for observability. A single telemetry crate provides EventBus for execution events, in-memory metrics primitives, and a TelemetryService trait so that production can plug in Prometheus/OTLP without changing engine code.

**nebula-telemetry is the event bus, metrics, and telemetry abstraction for the Nebula workflow engine.**

It answers: *How do engine and runtime emit events and metrics without depending on a specific exporter, and how do subscribers consume them without blocking execution?*

```
Engine/Runtime → EventBus::send(ExecutionEvent) — fire-and-forget
Engine/Runtime → Counter/Gauge/Histogram (in-memory or via TelemetryService)
    ↓
Subscribers (log, metrics, API SSE) receive events; exporters (future: Prometheus/OTLP) read metrics
```

This is the telemetry contract: events are projections (not source of truth); metrics names are stable; emit path is non-blocking.

---

## User Stories

### Story 1 — Engine Emits Execution Lifecycle Events (P1)

Engine emits ExecutionEvent (Started, NodeCompleted, NodeFailed, Completed). Telemetry crate provides EventBus<ExecutionEvent> (or equivalent). Subscribers (log, metrics, API) receive events; slow subscriber does not block engine.

**Acceptance**:
- EventBus send is fire-and-forget
- ExecutionEvent schema is owned by telemetry (or eventbus + telemetry); versioned
- Event schema additive-only in minor

### Story 2 — Runtime Records Node Metrics (P1)

Runtime increments counter on node start/complete/fail and records duration in histogram. Metrics are in-memory or forwarded via TelemetryService. No external exporter required for MVP.

**Acceptance**:
- Counter, Gauge, Histogram primitives; names stable
- TelemetryService trait allows noop and future Prometheus/OTLP impl
- High-cardinality histograms bounded (or document memory risk)

### Story 3 — Production Exports to Prometheus/OTLP (P2)

Operator needs Prometheus scrape or OTLP push for dashboards. TelemetryService implementation exports metrics; event bus can have optional persistence or forward to log/metrics pipeline.

**Acceptance**:
- Exporter is pluggable; core path has zero external deps
- Event schema versioning for compatibility
- Document export path and cardinality limits

---

## Core Principles

### I. Events Are Projections, Not Source of Truth

**Execution events are for observability. Execution state and result are authoritative in engine/storage.**

**Rationale**: Subscribers may drop or lag. No critical path should depend on event delivery for correctness.

**Rules**:
- Fire-and-forget emit; no synchronous "wait for subscribers"
- Document that events are best-effort
- Source of truth for execution state is engine/storage

### II. Emit Path Is Non-Blocking

**Sending events and recording metrics must not block or fail the execution path.**

**Rationale**: Observability failures must not become workflow failures.

**Rules**:
- EventBus send is non-blocking
- Metrics record is non-blocking (or bounded queue)
- TelemetryService implementations must not panic or block

### III. Metrics Names and Event Schema Are Stable

**Metric names and ExecutionEvent variants are versioned. Minor = additive only.**

**Rationale**: Dashboards and alerts depend on names and schema. Breaking them breaks production.

**Rules**:
- Document metric names and labels
- Event schema additive in minor; breaking in major with MIGRATION.md
- TelemetryService trait backward compatible

### IV. No Business Logic in Telemetry Crate

**Telemetry transports events and metrics. It does not interpret workflow or execution logic.**

**Rationale**: Engine and runtime own semantics; telemetry owns transport and primitives.

**Rules**:
- ExecutionEvent is data; interpretation is in subscribers
- No dependency on engine for event content semantics beyond schema
- Optional: eventbus crate holds generic bus; telemetry holds ExecutionEvent and metrics

### V. Pluggable Export, Zero External Deps for Core

**Core path (in-memory metrics, event bus) has no required external deps. Prometheus/OTLP are optional or behind feature.**

**Rationale**: Minimal builds and tests do not need exporters. Production enables export.

**Rejected**: Mandatory Prometheus dependency — would block minimal deployments.

---

## Production Vision

### The telemetry layer in an n8n-class fleet

In production, engine and runtime emit to EventBus and metrics. Subscribers (log, metrics service) consume events; Prometheus scrapes or OTLP pushes metrics. Histogram is bounded to avoid high-cardinality memory blow-up. Event schema is versioned; TelemetryService has production implementation.

```
EventBus<ExecutionEvent> — engine/runtime emit; log/metrics/API subscribe
Metrics: Counter, Gauge, Histogram — in-memory or TelemetryService
    → Prometheus exporter / OTLP (optional)
```

From the archives: phase 5 production and telemetry role. Production vision: Prometheus/OTLP exporter, bounded histogram, event schema versioning, fire-and-forget semantics.

### Key gaps from current state to prod

| Gap | Priority | Notes |
|-----|----------|-------|
| Prometheus/OTLP exporter | High | Production dashboards |
| Bounded histogram | High | Avoid high-cardinality memory |
| Event schema versioning | Medium | Additive-only in minor |
| EventBus extraction to nebula-eventbus | Medium | Already planned; telemetry keeps ExecutionEvent schema |

---

## Key Decisions

### D-001: Fire-and-Forget Events

**Decision**: EventBus send does not wait for subscribers.

**Rationale**: Engine must not block on observability. Drop or buffer when slow.

**Rejected**: Synchronous delivery — would couple latency to slowest subscriber.

### D-002: TelemetryService Trait for Pluggability

**Decision**: Metrics can go through TelemetryService trait; NoopTelemetry for tests.

**Rationale**: Production plugs in real exporter without changing engine code.

**Rejected**: Hardcoded Prometheus — would block noop and other backends.

### D-003: ExecutionEvent Owned by Telemetry (or Shared with Eventbus)

**Decision**: ExecutionEvent schema is defined in telemetry (or eventbus with telemetry as consumer). Engine/runtime depend on telemetry/eventbus for type.

**Rationale**: Single source of truth for event shape. Versioning in one place.

**Rejected**: ExecutionEvent in engine — would push eventbus to depend on engine.

---

## Non-Negotiables

1. **Events are projections** — not source of truth; best-effort delivery.
2. **Emit path non-blocking** — no block or fail on send/record.
3. **Metrics names and event schema stable** — additive in minor; breaking in major.
4. **No business logic in telemetry** — transport and primitives only.
5. **Core path zero required external deps** — exporters optional.
6. **Breaking event or metrics contract = major + MIGRATION.md** — dashboards and pipelines depend on it.

---

## Governance

- **PATCH**: Bug fixes, docs. No schema or name change.
- **MINOR**: Additive (new event fields, new metrics). No removal.
- **MAJOR**: Breaking event or metrics. Requires MIGRATION.md.
