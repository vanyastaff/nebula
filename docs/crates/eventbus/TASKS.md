# Tasks: nebula-eventbus

**ROADMAP**: [ROADMAP.md](ROADMAP.md) | **PLAN**: [PLAN.md](PLAN.md)

## Format: `[ID] [P?] Description`

- **[P]**: Can run in parallel with other [P] tasks in same phase
- IDs use prefix `EVB`

---

## Phase 1: Extract and Consolidate ⬜

**Goal**: Create crate; extract EventBus from telemetry/resource; migrate all consumers.

- [x] EVB-T001 Create `crates/eventbus` crate with `Cargo.toml` and workspace registration
- [x] EVB-T002 Implement generic `EventBus<E>` with tokio broadcast/mpsc in `src/lib.rs`
- [x] EVB-T003 [P] Implement `BackPressurePolicy` enum: DropOldest, DropNewest, Block
- [x] EVB-T004 [P] Implement `EventSubscriber<E>` — subscription handle with receive and close
- [x] EVB-T005 [P] Implement `EventBusStats` — emitted, dropped, active subscribers counts
- [x] EVB-T006 Migrate `nebula-telemetry` to use `nebula-eventbus` — remove internal EventBus
- [x] EVB-T007 Migrate `nebula-resource` to use `nebula-eventbus` — remove internal EventBus
- [x] EVB-T008 Verify `cargo test -p nebula-telemetry` still passes after migration
- [x] EVB-T009 [P] Verify `cargo test -p nebula-resource` still passes
- [x] EVB-T010 [P] Verify `cargo test -p nebula-engine` still passes

**Checkpoint**: No duplicate EventBus code; all consumer tests pass; crate published in workspace.

---

## Phase 2: Scoped Subscriptions and Filtering ⬜

**Goal**: SubscriptionScope, EventFilter, subscribe_scoped; backward compatible.

- [x] EVB-T011 Define `SubscriptionScope` in `src/scope.rs`: Workflow, Execution, Resource, Global
- [x] EVB-T012 [P] Define `EventFilter` in `src/filter.rs`: EventType, PayloadMatch, Custom
- [x] EVB-T013 Implement `EventBus::subscribe_scoped(scope, filter) -> ScopedSubscription`
- [x] EVB-T014 Add event metadata trait in `src/metadata.rs` — extract workflow_id, execution_id from events
- [x] EVB-T015 [P] Verify backward compatibility: unscoped `subscribe()` still works unchanged
- [x] EVB-T016 Write tests: scoped subscription receives only matching events

**Checkpoint**: Scoped subscriptions deliver only matching events; unscoped subscribe unchanged.

---

## Phase 3: Scale and Observability ⬜

**Goal**: EventBusStats + metrics integration; benchmarks; optional multi-bus per process.

- [x] EVB-T017 Integrate `EventBusStats` with `nebula-metrics` — emit `nebula_eventbus_*` metrics
- [x] EVB-T018 [P] Add criterion benchmarks for emit latency in `benches/emit.rs`
- [x] EVB-T019 [P] Add benchmarks for subscriber throughput under load
- [x] EVB-T020 Implement optional multiple `EventBus` instances per process (per-tenant isolation)
- [x] EVB-T021 Verify memory bounded under sustained emit — add memory usage test

**Checkpoint**: Emit < 1µs benchmark target verified; stats accurate; memory bounded.

---

## Phase 4: Ecosystem and DX ⬜

**Goal**: Optional distributed transport; schema versioning; integration examples.

- [ ] EVB-T022 Implement optional Redis event transport behind `redis-transport` feature flag
- [ ] EVB-T023 [P] Implement optional NATS event transport behind `nats-transport` feature flag
- [ ] EVB-T024 [P] Write event schema versioning guidance in README
- [ ] EVB-T025 [P] Write integration examples: log events, metrics, dashboard in `docs/examples/`
- [ ] EVB-T026 Document migration path for adding new event types

**Checkpoint**: Optional distributed backend available; docs complete; migration path documented.

---

## Dependencies & Execution Order

- Phase 1 → Phase 2 → Phase 3 → Phase 4 (sequential)
- [P] tasks within phases can run in parallel
- Phase 1 is a prerequisite for telemetry and resource crates to use it

## Verification (after all phases)

- [x] `cargo check -p nebula-eventbus --all-features`
- [x] `cargo test -p nebula-eventbus`
- [x] `cargo clippy -p nebula-eventbus -- -D warnings`
- [x] `cargo doc --no-deps -p nebula-eventbus`
- [x] emit latency < 1µs in benchmark
