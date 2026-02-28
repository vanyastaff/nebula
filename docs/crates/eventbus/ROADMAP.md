# Roadmap

## Phase 1: Extract and Consolidate

**Deliverables:**
- Create `nebula-eventbus` crate with generic `EventBus<E>`
- Implement `BackPressurePolicy` (DropOldest, DropNewest, Block)
- Implement `EventSubscriber<E>`, `EventBusStats`
- Migrate `nebula-telemetry` to use eventbus (replace internal EventBus)
- Migrate `nebula-resource` to use eventbus (replace internal EventBus)
- All existing tests pass

**Risks:**
- Migration breaks engine/runtime integration
- Performance regression vs current implementations

**Exit criteria:**
- `cargo test -p nebula-telemetry` passes
- `cargo test -p nebula-resource` passes
- `cargo test -p nebula-engine` passes
- No duplicate EventBus code in telemetry/resource

---

## Phase 2: Scoped Subscriptions and Filtering

**Deliverables:**
- `SubscriptionScope` enum (Workflow, Execution, Resource, Global)
- `EventFilter` (EventType, PayloadMatch, Custom)
- `EventBus::subscribe_scoped(scope, filter) -> ScopedSubscription`
- Event metadata traits for scope extraction (workflow_id, execution_id, etc.)
- Documentation and examples

**Risks:**
- Event schemas need metadata; breaking change for domain crates
- Filter complexity; performance of filter evaluation

**Exit criteria:**
- Scoped subscription delivers only matching events
- Filter reduces handler invocations measurably
- Backward compatible: unscoped subscribe still works

---

## Phase 3: Scale and Observability

**Deliverables:**
- EventBusStats integration with metrics (emitted, dropped, subscribers)
- Optional: multiple EventBus instances per process (e.g. per-tenant isolation)
- Benchmarks: emit latency, subscriber throughput, memory under load
- Capacity planning documentation

**Risks:**
- Multi-bus overhead; memory growth
- Stats overhead in hot path

**Exit criteria:**
- Benchmarks show emit < 1µs (sync)
- Stats accurate under load
- Memory bounded under sustained emit

---

## Phase 4: Ecosystem and DX

**Deliverables:**
- Distributed event transport (Redis/NATS) — optional; single-node default
- Event schema versioning guidance
- Integration examples: log, metrics, dashboard
- Developer documentation for adding new event types

**Risks:**
- Distributed adds complexity; may not be needed for single-node
- Schema evolution across domains

**Exit criteria:**
- Clear migration path for new event domains
- Documentation complete
- Optional distributed backend (if adopted)

---

## Metrics of Readiness

| Metric | Target |
|--------|--------|
| **Correctness** | All tests pass; no regressions |
| **Latency** | emit < 1µs (sync); recv < 10µs |
| **Throughput** | 100k+ events/sec single bus |
| **Stability** | No panics; Lagged handled gracefully |
| **Operability** | EventBusStats; integration with metrics |
