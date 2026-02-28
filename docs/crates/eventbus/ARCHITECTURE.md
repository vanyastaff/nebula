# Architecture

## Problem Statement

- **Business problem:** Workflow engine components (execution, resource, log, metrics) need loose coupling via asynchronous event notifications. Emitters must not block; subscribers must receive events in order; slow subscribers must not stall the system.
- **Technical problem:** Provide a unified pub/sub abstraction that consolidates duplicated EventBus logic from `nebula-telemetry` and `nebula-resource`, adds scoped subscriptions and filtering, and supports multiple event domains without unbounded memory growth.

## Current Architecture

### Module Map (Current — EventBus in Other Crates)

| Location | Responsibility |
|----------|----------------|
| `nebula-telemetry::event` | EventBus, ExecutionEvent, EventSubscriber |
| `nebula-resource::events` | EventBus, ResourceEvent, BackPressurePolicy, EventBusStats |
| `nebula-engine` | Emits ExecutionEvent via telemetry EventBus |
| `nebula-runtime` | Emits ExecutionEvent via telemetry EventBus |
| `nebula-resource::manager` | Emits ResourceEvent via resource EventBus |

### Data/Control Flow

1. **Telemetry path:** Engine/Runtime → `EventBus::emit(ExecutionEvent)` → broadcast channel → subscribers (metrics, optional logger)
2. **Resource path:** Manager/Pool → `EventBus::emit(ResourceEvent)` → broadcast channel → subscribers (metrics collector, health monitor)

### Known Bottlenecks

- **Duplication:** Two separate EventBus implementations with different policies and stats
- **No scoping:** All subscribers receive all events; filtering done ad-hoc in handlers
- **No unified API:** Different `subscribe()` return types; resource has BackPressurePolicy, telemetry does not
- **Event ownership:** Each crate owns its event enum; no shared event taxonomy

## Target Architecture

### Target Module Map (Planned)

```
nebula-eventbus/
├── core/           — EventBus<E>, BackPressurePolicy, EventBusStats
├── subscriber/     — EventSubscriber<E>, SubscriptionHandle
├── scope/          — SubscriptionScope, ScopedSubscription (Phase 2)
├── filter/         — EventFilter, FilteredSubscription (Phase 2)
└── re-export/      — TypedEventBus for ExecutionEvent, ResourceEvent (optional)
```

### Public Contract Boundaries

- `EventBus<E: Clone + Send>` — generic broadcast bus; `emit()`, `subscribe()`, `stats()`
- `BackPressurePolicy` — DropOldest (default), DropNewest, Block { timeout }
- `EventSubscriber<E>` — async `recv()`, sync `try_recv()`; handles Lagged/Closed
- Event schemas remain in domain crates (telemetry, resource); eventbus is transport-only

### Internal Invariants

- Events emitted after operation completes (not before)
- Emit never blocks caller (fire-and-forget for sync path)
- Lagging subscribers receive RecvError::Lagged; skip to latest
- No subscribers → event dropped silently (no error)

## Design Reasoning

### Key Trade-off 1: Generic vs typed buses

- **Option A:** Single `EventBus<dyn Event>` with type erasure — complex, runtime dispatch
- **Option B:** Generic `EventBus<E>` per event type — simple, compile-time; each domain has its own bus
- **Decision:** Option B; domain crates construct `EventBus<ExecutionEvent>`, `EventBus<ResourceEvent>`; eventbus provides the generic implementation

### Key Trade-off 2: Scoped subscriptions

- **Current:** All-or-nothing; subscribers filter in handler
- **Target:** `subscribe_scoped(scope, filter)` — bus filters before delivery; reduces handler complexity
- **Consequence:** Requires event metadata (workflow_id, execution_id, resource_id); Phase 2

### Rejected Alternatives

- **MPSC per subscriber:** Does not scale; broadcast is correct for fan-out
- **Blocking emit:** Would stall execution; fire-and-forget required
- **Unbounded channel:** Memory growth under backpressure; bounded with DropOldest/DropNewest

## Comparative Analysis

Sources: n8n, Node-RED, Temporal, Prefect, tokio broadcast.

| Pattern | Verdict | Rationale |
|---------|---------|-----------|
| tokio::sync::broadcast | **Adopt** | Battle-tested; bounded; Lagged semantics; zero-copy clone |
| Fire-and-forget emit | **Adopt** | n8n, Temporal; events are projections, not source of truth |
| Scoped subscriptions | **Adopt** | Reduces handler logic; workflow/execution isolation |
| Event filtering in bus | **Adopt** | Phase 2; avoids unnecessary handler invocations |
| Distributed event bus | **Defer** | Phase 3; single-node first; Redis/NATS later |
| At-least-once delivery | **Reject** | Events are best-effort projections; no persistence in bus |

## Breaking Changes (if any)

- None until crate exists; extraction from telemetry/resource will require migration (see MIGRATION.md).

## Open Questions

- Q1: Should eventbus own event type definitions or only transport? (Leaning: transport-only; domain crates own schemas)
- Q2: Single EventBus instance with multi-type vs multiple EventBus<E> instances? (Leaning: multiple instances; simpler, no type erasure)
