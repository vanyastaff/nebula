# Tasks: nebula-resilience

**Roadmap**: [ROADMAP.md](ROADMAP.md) | **Plan**: [PLAN.md](PLAN.md)

## Legend

- `[P]` — Can run in parallel with other `[P]` tasks in same phase
- `RSL-TXXX` — Task ID
- `→` — Depends on previous task

---

## Phase 1: API Contract Consolidation (Done)

**Goal**: Reconcile typed/untyped manager APIs and establish clear adoption path.

- [x] RSL-T001 [P] Reconcile typed/untyped manager APIs and document preferred adoption path
- [x] RSL-T002 [P] Align examples with current production guidance
- [x] RSL-T003 Clarify stability boundaries for advanced type-system APIs (→ T001)

**Checkpoint**: Unified API documented; examples aligned; stability boundaries clear.

---

## Phase 2: Performance and Scalability

**Goal**: Optimize resilience patterns for high-cardinality, high-throughput scenarios.

- [x] RSL-T004 [P] Benchmark manager hot paths with high service cardinality (`benches/manager.rs`)
- [x] RSL-T005 [P] Benchmark circuit breaker under concurrent load (`benches/circuit_breaker.rs`)
- [x] RSL-T006 [P] Benchmark rate limiter under concurrent load (`benches/rate_limiter.rs`)
- [x] RSL-T007 Optimize circuit breaker contention for high-concurrency scenarios (→ T005)
- [x] RSL-T008 Optimize rate limiter contention for high-concurrency scenarios (→ T006)
- [x] RSL-T009 Profile layer composition overhead in deep chains (→ T004)
- [x] RSL-T010 Document performance budget and thresholds (→ T004, T005, T006)
- [x] RSL-T026 [P] Add reproducible A/B benchmark harness and performance snapshot doc

**Checkpoint**: Benchmarks within budget; contention scenarios optimized; layer overhead profiled.

---

## Phase 3: Policy and Config Hardening

**Goal**: Strengthen policy validation and dynamic configuration behavior.

- [x] RSL-T011 [P] Implement validation for conflicting policy combinations (`src/policy/`)
- [x] RSL-T012 [P] Design and document policy migration/versioning strategy (`docs/crates/resilience/MIGRATION.md`)
- [x] RSL-T013 Tighten dynamic config reload behavior and document semantics (→ T011)
- [x] RSL-T014 Add tests for conflicting policy detection (→ T011)
- [x] RSL-T015 Add tests for policy reload determinism (→ T013)

**Checkpoint**: No conflicting policies accepted; migration strategy documented; reload behavior deterministic.

---

## Phase 4: Reliability and Safety

**Goal**: Expand fault-injection testing and formalize failure defaults.

- [x] RSL-T016 [P] Implement fault-injection tests for retry+breaker interplay
- [x] RSL-T017 [P] Implement fault-injection tests for retry+timeout interplay
- [x] RSL-T018 [P] Implement fault-injection tests for breaker+timeout interplay
- [x] RSL-T019 Implement fault-injection test for retry+breaker+timeout combined (→ T016, T017, T018)
- [x] RSL-T020 Validate observability behavior in failure storms (→ T019)
- [x] RSL-T021 Formalize and document fail-open/fail-closed defaults per pattern (→ T019)

**Checkpoint**: Fault-injection tests cover all pattern combinations; defaults documented.

---

## Phase 5: Toolchain and Compatibility

**Goal**: Ensure forward compatibility and define serialization guarantees.

- [ ] RSL-T022 [P] Verify workspace builds cleanly on Rust 1.93+
- [ ] RSL-T023 [P] Define compatibility guarantees for policy serialization format
- [ ] RSL-T024 [P] Define compatibility guarantees for metrics schema
- [ ] RSL-T025 Document compatibility guarantees and migration path (→ T022, T023, T024)

**Checkpoint**: Compatibility guarantees documented; no MSRV regression.

---

## Dependencies & Execution Order

- Phase 1 → Phase 2 → Phase 3 → Phase 4 → Phase 5
- [P] tasks within a phase can run in parallel

## Verification (after all phases)

- [ ] `cargo check -p nebula-resilience --all-features`
- [ ] `cargo test -p nebula-resilience`
- [ ] `cargo clippy -p nebula-resilience -- -D warnings`
- [ ] `cargo doc --no-deps -p nebula-resilience`
