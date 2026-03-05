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

- [x] RSL-T022 [P] Verify workspace builds cleanly on Rust 1.93+
- [x] RSL-T023 [P] Define compatibility guarantees for policy serialization format
- [x] RSL-T024 [P] Define compatibility guarantees for metrics schema
- [x] RSL-T025 Document compatibility guarantees and migration path (→ T022, T023, T024)

**Checkpoint**: Compatibility guarantees documented; no MSRV regression.

---

## Phase 6: Pattern Coverage Expansion

**Goal**: Close performance and correctness gaps for patterns not deeply covered in earlier phases.

- [x] RSL-T027 [P] Benchmark and tune `GovernorRateLimiter` under concurrent load
- [x] RSL-T028 [P] Benchmark timeout wrapper overhead and cancellation-path latency
- [x] RSL-T029 [P] Add fault-injection matrix for fallback strategies (value/function/cache/chain)
- [x] RSL-T030 [P] Add stress and correctness tests for hedge executors (`HedgeExecutor`, `AdaptiveHedgeExecutor`)
- [x] RSL-T031 Consolidate per-pattern defaults/limits guidance for timeout/fallback/hedge/governor (→ T027, T028, T029, T030)

**Checkpoint**: Governor/timeout/fallback/hedge have benchmark coverage, fault-injection checks, and documented operational guidance.

---

## Phase 7: Pattern Hardening Wave

**Goal**: Improve bulkhead/retry/fallback/timeout operational robustness beyond Phase 6 baseline.

- [x] RSL-T032 [P] Add dedicated `bulkhead` benchmark target (fast path, contention, queue-timeout)
- [x] RSL-T033 [P] Add bulkhead fairness/starvation stress tests under sustained contention
- [x] RSL-T034 [P] Add retry storm-guard guidance and adaptive jitter tuning tests
- [x] RSL-T035 [P] Add fallback staleness-policy coverage (`stale-if-error` / bounded chain guidance)
- [x] RSL-T036 [P] Add timeout short-deadline guidance by platform/runtime constraints
- [x] RSL-T037 Consolidate Phase 7 hardening guidance and update budgets (→ T033, T034, T035, T036)

**Checkpoint**: Bulkhead/retry/fallback/timeout have hardened stress coverage and consolidated operational guardrails.

---

## Phase 8: Coverage and Code-Quality Remediation

**Goal**: Close remaining test/benchmark blind spots and eliminate high-signal quality issues flagged during post-Phase-7 audit.

- [x] RSL-T038 [P] Add integration tests for `PriorityFallback` routing and default-fallback behavior
- [x] RSL-T039 [P] Add integration tests for `BimodalHedgeExecutor` semantics and side-effect safety
- [x] RSL-T040 [P] Add integration coverage for `ResilienceManager::execute_with_override` policy precedence and isolation
- [x] RSL-T041 [P] Add benchmark targets for fallback and hedge execution overhead under contention
- [x] RSL-T042 [P] Add observability hook throughput benchmark and backpressure/drop-policy tests
- [x] RSL-T043 Fix high-signal `clippy` findings in hot paths (`circuit_breaker`, `rate_limiter`, `fallback`, `manager`)
- [x] RSL-T044 Consolidate coverage map (unit/integration/bench) and define regression gate updates (→ T038, T039, T040, T041, T042, T043)

**Checkpoint**: Previously uncovered APIs have integration/bench coverage and `clippy -D warnings` passes for audited hot paths.

---

## Dependencies & Execution Order

- Phase 1 → Phase 2 → Phase 3 → Phase 4 → Phase 5 → Phase 6 → Phase 7 → Phase 8
- [P] tasks within a phase can run in parallel

## Verification (after all phases)

- [x] `cargo check -p nebula-resilience --all-features`
- [x] `cargo test -p nebula-resilience`
- [x] `cargo clippy -p nebula-resilience -- -D warnings`
- [x] `cargo doc --no-deps -p nebula-resilience`
