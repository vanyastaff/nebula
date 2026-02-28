# Interactions

## Ecosystem Map (Current + Planned)

### Existing Crates (Current — EventBus in telemetry/resource)

| Crate | Relationship | Description |
|-------|-------------|-------------|
| `nebula-telemetry` | Contains EventBus | ExecutionEvent, EventBus, EventSubscriber |
| `nebula-resource` | Contains EventBus | ResourceEvent, BackPressurePolicy, EventBusStats |
| `nebula-engine` | Downstream | Emits ExecutionEvent via telemetry EventBus |
| `nebula-runtime` | Downstream | Emits ExecutionEvent via telemetry EventBus |
| `nebula-log` | Sibling | Logging; may subscribe to events (future) |
| `nebula-core` | Upstream | Identifiers (ExecutionId, NodeId, WorkflowId) |

### Planned Crates (After eventbus extraction)

| Crate | Relationship | Description |
|-------|-------------|-------------|
| `nebula-eventbus` | New | Generic EventBus<E>, BackPressurePolicy, scoped subscriptions |
| `nebula-telemetry` | Downstream | Uses EventBus<ExecutionEvent> from eventbus; owns ExecutionEvent schema |
| `nebula-resource` | Downstream | Uses EventBus<ResourceEvent> from eventbus; owns ResourceEvent schema |
| `nebula-log` | Downstream | Subscribes to ExecutionEvent for structured logging |
| `nebula-metrics` | Downstream | Subscribes to events for counters/histograms |

## Downstream Consumers

### nebula-telemetry

- **Expectations:** `EventBus<ExecutionEvent>` from eventbus; re-exports or wraps for engine/runtime
- **Contract:** Sync emit; fire-and-forget; event schema owned by telemetry

### nebula-resource

- **Expectations:** `EventBus<ResourceEvent>` from eventbus; BackPressurePolicy support
- **Contract:** Sync and async emit; EventBusStats for observability

### nebula-log (planned)

- **Expectations:** Subscribe to ExecutionEvent; map to log entries
- **Contract:** Async recv loop; never block emit path

### nebula-metrics / telemetry metrics

- **Expectations:** Subscribe to NodeCompleted, NodeFailed, etc.; update histograms/counters
- **Contract:** Fire-and-forget; best-effort delivery

## Upstream Dependencies

| Crate | Why needed | Hard contract | Fallback |
|-------|------------|---------------|----------|
| `tokio` | broadcast channel | `broadcast::Sender` | — |
| `nebula-core` | Optional; Scope, Id types for scoped subscriptions | — | Phase 2 |

## Interaction Matrix

| This crate <-> Other | Direction | Contract | Sync/Async | Failure handling | Notes |
|----------------------|-----------|----------|------------|------------------|-------|
| eventbus -> telemetry | out | EventBus<E> type | sync | emit never fails | Telemetry owns ExecutionEvent |
| eventbus -> resource | out | EventBus<E>, BackPressurePolicy | sync/async | emit best-effort | Resource owns ResourceEvent |
| telemetry -> eventbus | in | EventBus<ExecutionEvent> | sync | — | Telemetry constructs bus |
| resource -> eventbus | in | EventBus<ResourceEvent> | sync/async | — | Resource constructs bus |
| engine -> telemetry | in | emit(ExecutionEvent) | sync | best-effort | Via telemetry's bus |
| runtime -> telemetry | in | emit(ExecutionEvent) | sync | best-effort | Via telemetry's bus |
| log -> eventbus | in | subscribe, recv | async | Lagged handled | Phase 2 |
| metrics -> eventbus | in | subscribe, recv | async | Lagged handled | Via telemetry or direct |

## Runtime Sequence

1. Application constructs `EventBus<ExecutionEvent>` and `EventBus<ResourceEvent>` (or domain crates do).
2. Telemetry/resource inject buses into engine, runtime, manager.
3. On execution start: engine emits `Started` via bus.
4. On node execution: runtime emits `NodeStarted`, `NodeCompleted`/`NodeFailed`.
5. On resource lifecycle: manager emits `Acquired`, `Released`, `HealthChanged`, etc.
6. Subscribers (metrics, log) receive events in background tasks; process and update state.

## Cross-Crate Ownership

| Responsibility | Owner |
|----------------|-------|
| EventBus transport implementation | `nebula-eventbus` |
| ExecutionEvent schema | `nebula-telemetry` |
| ResourceEvent schema | `nebula-resource` |
| BackPressurePolicy, EventBusStats | `nebula-eventbus` |
| Emit timing and content | `nebula-engine`, `nebula-runtime`, `nebula-resource` |
| Source of truth for execution state | `ports::ExecutionRepo` (not eventbus) |

## Failure Propagation

- **How failures bubble up:** Emit does not return `Result`; infallible from caller perspective.
- **Where retries apply:** N/A; no I/O.
- **Where retries forbidden:** N/A.
- **Lagged subscribers:** Skip to latest; no retry of missed events (events are projections).

## Versioning and Compatibility

- **Compatibility promise:** EventBus API additive-only; BackPressurePolicy variants additive; event schemas owned by domain crates.
- **Breaking-change protocol:** Major version bump; migration guide in MIGRATION.md.
- **Deprecation window:** Minimum 2 minor releases.

## Contract Tests Needed

- [ ] EventBus emit with zero subscribers does not panic
- [ ] Multiple subscribers each receive copy of event
- [ ] Subscriber recv/try_recv returns events in order (when not lagging)
- [ ] BackPressurePolicy::Block emit_async respects timeout
- [ ] EventBusStats emitted/dropped/subscribers accurate
- [ ] Telemetry and resource can migrate to eventbus without API break
