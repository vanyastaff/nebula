# Test Strategy

## Test Pyramid

- **Unit:** EventBus::new, emit, subscribe, stats; BackPressurePolicy behavior; EventSubscriber recv/try_recv; Lagged handling.
- **Integration:** Telemetry with eventbus EventBus; resource with eventbus EventBus; engine emits; runtime emits; subscriber receives.
- **Contract:** EventBus API stability; BackPressurePolicy semantics; EventSubscriber recv returns Option<E>.
- **End-to-end:** Execution flow emits events; metrics subscriber updates counters; no panics.

## Critical Invariants

- emit with zero subscribers does not panic.
- Multiple subscribers each receive a copy of every event (when not lagging).
- Subscriber recv returns events in order (modulo Lagged skip).
- EventBusStats.emitted increments on successful send; EventBusStats.dropped increments when event dropped.
- BackPressurePolicy::Block emit_async blocks up to timeout when no subscribers; then drops.
- Event type E must be Clone + Send.

## Scenario Matrix

| Scenario | Coverage |
|----------|----------|
| Happy path | Emit → subscribe → recv → event received |
| No subscribers | Emit → no panic; dropped counted |
| Multiple subscribers | Emit → all receive |
| Lagged subscriber | Emit faster than recv → Lagged → skip to latest |
| Buffer full (DropOldest) | Overflow → oldest overwritten; Lagged for slow subscriber |
| Buffer full (DropNewest) | Overflow → new event dropped (when applicable) |
| Buffer full (Block) | emit_async → wait up to timeout → drop if no space |
| Sender dropped | recv returns None |
| Cancellation | Subscriber task cancelled; no leak |

## Tooling

- **Property testing:** proptest for event payloads (Clone, roundtrip if Serialize).
- **Fuzzing:** Optional; event struct fuzz for Clone.
- **Benchmarks:** emit latency; recv throughput; memory under sustained emit.
- **CI quality gates:** `cargo test -p nebula-eventbus`; `cargo test -p nebula-telemetry`; `cargo test -p nebula-resource`.

## Exit Criteria

- **Coverage goals:** EventBus, EventSubscriber, BackPressurePolicy, stats; migration tests for telemetry/resource.
- **Flaky test budget:** Zero.
- **Performance regression:** emit < 1µs; no regression in engine/runtime tests.
