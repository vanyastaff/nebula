# Tasks: nebula-resource

**ROADMAP**: [ROADMAP.md](ROADMAP.md) | **PLAN**: [PLAN.md](PLAN.md)

## Format: `[ID] [P?] Description`

- **[P]**: Can run in parallel with other [P] tasks in same phase
- IDs use prefix `RSC`

---

## Phase 1: Contract and Safety Baseline ✅

**Goal**: Finalize cross-crate contracts; formalize error taxonomy; lock scope invariants.

- [x] RSC-T001 Finalize INTERACTIONS.md — map every integration point with action/runtime/credential to actual code
- [x] RSC-T002 [P] Update API.md — annotate every public item with stability guarantee
- [x] RSC-T003 [P] Update MIGRATION.md — document breaking change policy and migration path for consumers
- [x] RSC-T004 Define error handling taxonomy in `src/error.rs`: retryable, fatal, validation categories
- [x] RSC-T005 [P] Lock scope invariants — document deny-by-default behavior in ARCHITECTURE.md
- [x] RSC-T006 [P] Add integration contract tests with nebula-action in `tests/action_integration.rs`
- [x] RSC-T007 Add integration contract tests with nebula-runtime in `tests/runtime_integration.rs`

**Checkpoint**: Contract docs match implementation; error taxonomy documented; scope invariants locked and tested.

---

## Phase 2: Runtime Hardening ✅

**Goal**: Deterministic shutdown tests; health-to-quarantine observability; reload guardrails.

- [x] RSC-T008 Write stress tests for pool shutdown under in-flight load in `tests/shutdown_stress.rs`
- [x] RSC-T009 [P] Add health-to-quarantine propagation observability — structured events for each transition
- [x] RSC-T010 Implement operational guardrails for invalid config reload attempts (reject or warn)
- [x] RSC-T011 [P] Write CI race scenario tests — verify no leaked permits/instances under concurrent ops
- [x] RSC-T012 Write pool swap integration test — old pool drains, new pool activates cleanly

**Checkpoint**: Deterministic shutdown in stress tests; no leaked permits in CI race scenarios.

---

## Phase 3: Scale and Performance ✅

**Goal**: Criterion benchmarks; backpressure policy profiles; high-cardinality metrics hygiene.

- [x] RSC-T013 Add criterion benchmarks for acquire latency in `benches/acquire.rs`
- [x] RSC-T014 [P] Benchmark pool contention under concurrent load
- [x] RSC-T015 Implement backpressure policy: `fail-fast` (immediate error on pool exhaustion)
- [x] RSC-T016 [P] Implement backpressure policy: `bounded-wait` (queue with timeout)
- [x] RSC-T017 [P] Implement backpressure policy: `adaptive` (dynamic based on system pressure)
- [x] RSC-T018 Improve high-cardinality metrics hygiene — cap metric label cardinality for multi-tenant

**Checkpoint**: p95 acquire latency within SLO; backpressure policies selectable; no perf regression.

---

## Phase 4: Ecosystem and DX 🔄

**Goal**: Adapter crate guidance; typed key migration path; integration cookbook.

- [x] RSC-T019 Write adapter crate guide: how to implement a resource driver (postgres, redis) in `docs/adapters.md`
- [ ] RSC-T020 [P] Implement reference adapter: `resource-postgres` in `crates/resource-postgres`
- [x] RSC-T021 Document typed key migration path for consumers
- [x] RSC-T022 Write cookbook: runtime/action integration patterns using resource in `docs/cookbook.md`

**Checkpoint**: Reference adapter works end-to-end; cookbook demonstrates runtime/action/resource composition.

---

## Dependencies & Execution Order

- Phase 1 → Phase 2 → Phase 3 → Phase 4 → Phase 5
- [P] tasks within phases can run in parallel
- Phase 1 should coordinate with `nebula-credential` Phase 1 (credential–resource integration)

---

## Phase 5: Pool Safety Hardening ✅

**Goal**: Apply safety primitives for correct cooperative shutdown and RAII observability.

- [x] RSC-T023 Implement `Poison<T>`, `PoisonGuard`, `PoisonError` in `crates/resource/src/poison.rs`; wrap `PoolState` in `Mutex<Poison<PoolState>>`; map `PoisonError` to `Error::Internal`
- [x] RSC-T024 Implement `Gate`/`GateGuard` cooperative shutdown barrier in `nebula-resilience::gate`; export in prelude
- [x] RSC-T025 Wire `Gate` into `PoolInner`: maintenance task holds `GateGuard`; `shutdown()` calls `gate.close().await` before `semaphore.close()`
- [x] RSC-T026 Replace manual `fetch_add`/`fetch_sub` pairs in `acquire_inner` with `CounterGuard` RAII wrapper
- [x] RSC-T027 Add `NEBULA_RESOURCE_CIRCUIT_BREAKER_OPENED_TOTAL` / `NEBULA_RESOURCE_CIRCUIT_BREAKER_CLOSED_TOTAL` constants to `nebula-metrics`
- [x] RSC-T028 Emit dedicated CB open/close counters with `{resource_id, operation}` label in `MetricsCollector`

**Checkpoint**: All 6 gate unit tests + 1 doctest pass; `cargo check --workspace --all-targets` clean; hardening checklist rows show Implemented.

## Verification (after all phases)

- [x] `cargo check -p nebula-resource`
- [x] `cargo test -p nebula-resource`
- [x] `cargo clippy -p nebula-resource -- -D warnings`
- [x] `cargo doc --no-deps -p nebula-resource`
- [x] `cargo bench -p nebula-resource`
