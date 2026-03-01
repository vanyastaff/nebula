# nebula-eventbus Constitution

> **Version**: 1.0.0 | **Created**: 2026-03-01

---

## Platform Role

Engine, runtime, resource, credential, and telemetry need to notify each other and external subscribers without tight coupling. Events (execution started, node completed, resource acquired, credential rotated) should be broadcast so that metrics, logging, and API can consume them without the producer depending on consumers.

**nebula-eventbus is the generic event distribution layer.**

It answers: *How do crates emit and subscribe to typed events (E) with backpressure policy and no blocking of the emit path?*

```
Producer (engine, runtime, resource, etc.) → EventBus::send(event)
    ↓
Broadcast to subscribers (telemetry, log, metrics, API SSE)
    ↓
BackPressurePolicy: drop lagging subscribers or block (configurable)
    ↓
Subscribers recv() async; emit path is non-blocking (fire-and-forget)
```

This is the eventbus contract: generic over event type E; sync or async send; subscribers are best-effort; observability failures do not block producers.

---

## User Stories

### Story 1 — Engine Emits Execution Events (P1)

Engine starts and completes executions and nodes. It sends ExecutionEvent (Started, NodeCompleted, NodeFailed, Completed) to the event bus. Telemetry and metrics subscribe; they must not block the engine.

**Acceptance**:
- EventBus::send(event) is non-blocking (or documented blocking policy)
- Subscribers receive events via async recv or callback; slow subscriber is dropped or backpressure applied per policy
- Event type E is generic; telemetry owns ExecutionEvent schema

### Story 2 — Resource Manager Emits Pool Events (P1)

Resource crate emits ResourceEvent (acquired, released, quarantined, health check). Metrics and API subscribe for dashboard and alerting.

**Acceptance**:
- Same EventBus pattern: send non-blocking; subscribe best-effort
- ResourceEvent schema owned by resource crate; eventbus is transport only
- EventBusStats or similar for observability (dropped events, subscriber count)

### Story 3 — Subscriber Does Not Block Producer (P2)

A slow or failing subscriber (e.g. metrics endpoint down) must not block or fail the producer. BackPressurePolicy controls whether to drop subscriber or apply backpressure.

**Acceptance**:
- Document BackPressurePolicy: Drop vs Block; default Drop for observability events
- Producer never panics or returns Err due to subscriber failure (unless policy is Block and buffer full)
- Fire-and-forget semantics for default policy

---

## Core Principles

### I. Event Type Is Generic

**EventBus<E> is parameterized by event type. Producers and subscribers agree on E; eventbus does not define domain events.**

**Rationale**: ExecutionEvent belongs to telemetry/engine; ResourceEvent to resource. Eventbus is transport, not schema owner.

**Rules**:
- No ExecutionEvent or ResourceEvent types in eventbus crate
- Generic EventBus<E: Clone + Send> or equivalent
- Subscriber receives E

### II. Emit Path Is Non-Blocking by Default

**Send must not block on subscriber processing. Lagging subscribers are dropped or buffered per policy.**

**Rationale**: Producers (engine, runtime) must never be slowed by observability. Losing some events is acceptable for metrics/logging.

**Rules**:
- Default BackPressurePolicy: drop oldest or drop subscriber when buffer full
- Optional Block policy for critical paths; documented
- No synchronous "wait for all subscribers" in default path

### III. Best-Effort Delivery

**Delivery to subscribers is best-effort. No guaranteed delivery or ordering across subscribers.**

**Rationale**: Simplifies implementation and keeps emit path fast. Critical consistency is not achieved via eventbus.

**Rules**:
- Document delivery semantics (per-subscriber order, no cross-subscriber guarantee)
- EventBusStats for monitoring (sent, dropped, subscriber lag)

### IV. No Business Logic in Eventbus

**Eventbus transports events. It does not interpret, filter, or transform domain events.**

**Rationale**: Filtering and interpretation belong to subscribers or domain crates. Eventbus stays small and stable.

**Rules**:
- No dependency on engine, resource, credential, or telemetry domain types
- Only generic E and transport/backpressure policy

---

## Production Vision

### The eventbus in an n8n-class fleet

In production, each process has one or more EventBus instances (e.g. EventBus<ExecutionEvent>, EventBus<ResourceEvent>). Engine and runtime send execution events; resource sends resource events. Telemetry, metrics, log, and API subscribe. Events are broadcast; slow subscribers are dropped so that engine and runtime never block. Optional EventBusStats exposed for monitoring.

```
EventBus<ExecutionEvent>
    ├── send(ExecutionEvent) — non-blocking
    ├── subscribe() → Receiver<ExecutionEvent> or callback
    ├── BackPressurePolicy: Drop | Block (per deployment)
    └── EventBusStats: sent_count, dropped_count, subscriber_count
```

Scoped subscriptions (e.g. by execution_id or workflow_id) can be implemented as filter in subscriber or as future eventbus feature. Event schema evolution is owned by domain crates; eventbus remains transport.

### From the archives: extraction and consumers

The archive and INTERACTIONS.md describe eventbus as extracted from telemetry/resource; after extraction, nebula-telemetry and nebula-resource use EventBus<ExecutionEvent> and EventBus<ResourceEvent>. Contract: sync emit, fire-and-forget, event schema owned by telemetry/resource. Production vision aligns: eventbus is generic transport; domain crates own event types and semantics.

### Key gaps from current state to prod

| Gap | Priority | Notes |
|-----|----------|-------|
| Formal BackPressurePolicy API and docs | High | Document Drop vs Block; buffer sizes |
| EventBusStats for observability | Medium | Sent, dropped, subscriber count |
| Scoped subscriptions (filter by scope/execution_id) | Low | Optional; can be in subscriber |
| Persistence or replay | Low | Not in scope for first production; eventbus is in-memory |

---

## Key Decisions

### D-001: Generic EventBus<E>, Not Single Event Type

**Decision**: EventBus is generic over event type. Telemetry and resource each have their own event type.

**Rationale**: Avoids eventbus depending on all domain crates. Each domain owns its event schema.

**Rejected**: Single enum of all events — would create central dependency and versioning bottleneck.

### D-002: Non-Blocking Send by Default

**Decision**: send() does not wait for subscribers to process. Backpressure policy can drop or buffer.

**Rationale**: Producers must not block on observability. Drop is acceptable for metrics/logging.

**Rejected**: Synchronous delivery — would couple producer latency to slowest subscriber.

### D-003: Best-Effort Delivery

**Decision**: No guaranteed delivery or global ordering across subscribers.

**Rationale**: Keeps implementation simple and emit path fast. Critical paths use other mechanisms.

**Rejected**: At-least-once or ordered delivery — would require persistence and ack, out of scope.

---

## Open Proposals

### P-001: Scoped Subscription API

**Problem**: Subscribers may want only events for a given execution or workflow.

**Proposal**: subscribe_with_filter(predicate) or subscribe_scoped(scope) that filters events before delivery.

**Impact**: Additive; may require Clone + filter in eventbus or in subscriber wrapper.

### P-002: EventBusStats Standardization

**Problem**: Operators need to see bus health (drops, lag).

**Proposal**: Standard EventBusStats (sent, dropped, subscriber_count, optional buffer_len) and optional metrics export.

**Impact**: Additive; all backends implement same stats shape.

---

## Non-Negotiables

1. **Generic EventBus<E>** — no domain event types in eventbus crate.
2. **Emit path non-blocking by default** — producers never block on subscriber speed.
3. **Best-effort delivery** — no guarantee of delivery or global order.
4. **No business logic in eventbus** — transport and backpressure only.
5. **BackPressurePolicy documented** — Drop vs Block and buffer semantics.
6. **Breaking send/subscribe contract = major + MIGRATION.md** — all producers and subscribers depend on it.

---

## Governance

- **PATCH**: Bug fixes, docs. No change to send/subscribe semantics.
- **MINOR**: Additive (stats, optional filter). No removal.
- **MAJOR**: Breaking changes to send/subscribe or policy. Requires MIGRATION.md.
