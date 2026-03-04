# Implementation Plan: nebula-eventbus

**Crate**: `nebula-eventbus` | **Path**: `crates/eventbus` | **ROADMAP**: [ROADMAP.md](ROADMAP.md)

## Summary

The eventbus crate provides a generic `EventBus<E>` for intra-process event delivery with backpressure policies (DropOldest, DropNewest, Block), scoped subscriptions, and stats. It will consolidate duplicated EventBus implementations in `nebula-telemetry` and `nebula-resource`. Current focus is Phase 1: creating the crate, extracting and consolidating implementations, and migrating consumers.

## Technical Context

**Language/Edition**: Rust 2024 (MSRV 1.93)
**Async Runtime**: Tokio (broadcast/mpsc channels)
**Key Dependencies**: `nebula-core`, `tokio`
**Testing**: `cargo test -p nebula-eventbus`

## Current Status

| Phase | Status | Summary |
|-------|--------|---------|
| Phase 1: Extract and Consolidate | ⬜ Planned | Create crate, extract from telemetry/resource, migrate consumers |
| Phase 2: Scoped Subscriptions and Filtering | ⬜ Planned | SubscriptionScope, EventFilter, subscribe_scoped |
| Phase 3: Scale and Observability | ⬜ Planned | EventBusStats + metrics, benchmarks, multi-bus isolation |
| Phase 4: Ecosystem and DX | ⬜ Planned | Distributed transport (optional), schema versioning, docs |

## Phase Details

### Phase 1: Extract and Consolidate

**Goal**: Create `nebula-eventbus` crate with generic `EventBus<E>`; migrate telemetry and resource.

**Deliverables**:
- `nebula-eventbus` crate with `EventBus<E>`, `BackPressurePolicy`, `EventSubscriber<E>`, `EventBusStats`
- `nebula-telemetry` migrated to use eventbus
- `nebula-resource` migrated to use eventbus
- All existing tests pass

**Exit Criteria**:
- `cargo test -p nebula-telemetry`, `nebula-resource`, `nebula-engine` all pass
- No duplicate EventBus code remaining

**Risks**:
- Migration breaks engine/runtime integration
- Performance regression vs current implementations

### Phase 2: Scoped Subscriptions and Filtering

**Goal**: Scoped subscriptions deliver only matching events; EventFilter reduces handler invocations.

**Deliverables**:
- `SubscriptionScope` enum: Workflow, Execution, Resource, Global
- `EventFilter`: EventType, PayloadMatch, Custom
- `EventBus::subscribe_scoped(scope, filter)`
- Backward compatible: unscoped subscribe still works

**Exit Criteria**:
- Scoped subscription delivers only matching events
- Filter measurably reduces handler invocations

### Phase 3: Scale and Observability

**Goal**: EventBusStats integration with metrics; benchmarks; multi-bus isolation option.

**Deliverables**:
- EventBusStats with metrics integration (emitted, dropped, subscribers)
- Optional multiple EventBus instances per process (per-tenant isolation)
- Benchmarks: emit latency, subscriber throughput, memory under load

**Exit Criteria**:
- Benchmarks show emit < 1µs (sync)
- Memory bounded under sustained emit

### Phase 4: Ecosystem and DX

**Goal**: Distributed transport (optional); schema versioning guidance; integration docs.

**Deliverables**:
- Optional Redis/NATS distributed transport (behind feature flag)
- Event schema versioning guidance
- Integration examples: log, metrics, dashboard

**Exit Criteria**:
- Clear migration path for new event domains; docs complete

## Inter-Crate Dependencies

- **Depends on**: `nebula-core`, `tokio`
- **Depended by**: `nebula-telemetry`, `nebula-resource`, `nebula-engine`, `nebula-execution`

## Verification

- [ ] `cargo check -p nebula-eventbus`
- [ ] `cargo test -p nebula-eventbus`
- [ ] `cargo clippy -p nebula-eventbus -- -D warnings`
- [ ] `cargo doc --no-deps -p nebula-eventbus`
