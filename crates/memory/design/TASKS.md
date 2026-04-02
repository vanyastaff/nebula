# Tasks: nebula-memory

**Roadmap**: [ROADMAP.md](ROADMAP.md) | **Plan**: [PLAN.md](PLAN.md)

## Legend

- `[P]` -- Can run in parallel with other `[P]` tasks in same phase
- `MEM-TXXX` -- Task ID
- `->` -- Depends on previous task

---

## Phase 1: Contract and Safety Baseline

**Goal**: Finalize SPEC-aligned docs and cross-crate memory contracts

- [ ] MEM-T001 [P] Audit public API surface and classify as stable vs. unstable (`src/lib.rs` re-exports)
- [ ] MEM-T002 [P] Document safety invariants for all `unsafe` blocks in arena and pool modules
- [ ] MEM-T003 [P] Finalize SPEC-aligned docs: align ARCHITECTURE.md and API.md with real code
- [ ] MEM-T004 Define cross-crate memory contracts for consumer crates (runtime, action, expression) (-> T001)
- [ ] MEM-T005 Write contract tests validating stable API subset (`tests/contracts.rs`) (-> T001)
- [ ] MEM-T006 Mark unstable APIs with `#[doc(hidden)]` or feature gate (-> T001)
- [ ] MEM-T007 Review consumer crate assumptions against documented contracts (-> T004)

**Checkpoint**: Docs map to real code; stable/unstable surface clearly marked; review-ready for runtime/action teams.

---

## Phase 2: Runtime Hardening

**Goal**: Validate concurrent behavior and pressure handling under spikes

- [ ] MEM-T008 [P] Write concurrency stress tests for shared object pools (`tests/stress_pool.rs`)
- [ ] MEM-T009 [P] Write concurrency stress tests for cache subsystem (`tests/stress_cache.rs`)
- [ ] MEM-T010 [P] Write pressure handling tests: validate budget enforcement under memory spikes
- [ ] MEM-T011 Audit error-path behavior consistency across pool/cache/arena modules (-> T008, T009)
- [ ] MEM-T012 Fix any non-deterministic behavior found in stress tests (-> T008, T009, T010)
- [ ] MEM-T013 Add regression tests for contention scenarios (-> T012)
- [ ] MEM-T014 Verify no critical flaky tests remain in concurrency test suite (-> T013)

**Checkpoint**: Deterministic behavior in stress scenarios; no critical flaky tests.

---

## Phase 3: Scale and Performance

**Goal**: Benchmark-guided tuning with published sizing profiles

- [ ] MEM-T015 [P] Create benchmark suite for arena allocator performance (`benches/arena.rs`)
- [ ] MEM-T016 [P] Create benchmark suite for object pool throughput (`benches/pool.rs`)
- [ ] MEM-T017 [P] Create benchmark suite for cache hit/miss rates (`benches/cache.rs`)
- [ ] MEM-T018 Analyze benchmark results and identify tuning opportunities (-> T015, T016, T017)
- [ ] MEM-T019 Apply benchmark-guided tuning to hot paths (-> T018)
- [ ] MEM-T020 Publish sizing profiles for common workflow load classes (small/medium/large) (-> T018)
- [ ] MEM-T021 Measure and optimize feature-on overhead for monitoring/stats paths (-> T019)
- [ ] MEM-T022 Add mitigation for over-optimization: verify semantic parity after tuning (-> T019)

**Checkpoint**: Measurable p95/p99 improvement on representative scenarios; sizing profiles documented.

---

## Phase 4: Ecosystem and DX

**Goal**: Reference integration patterns and migration tooling

- [ ] MEM-T023 [P] Write reference integration pattern for runtime crate memory usage
- [ ] MEM-T024 [P] Write reference integration pattern for action crate memory usage
- [ ] MEM-T025 [P] Evaluate unified config bootstrap proposal for memory subsystem
- [ ] MEM-T026 Implement unified config bootstrap if proposal accepted (-> T025)
- [ ] MEM-T027 Create migration checklist for feature and API evolution (-> T001)
- [ ] MEM-T028 Document end-to-end integration flow from config to runtime (-> T023, T024)
- [ ] MEM-T029 Verify no consumer crate breakage after API cleanup (-> T027)

**Checkpoint**: At least one fully documented end-to-end integration flow; migration tooling available.

---

## Dependencies & Execution Order

- Phase 1 -> Phase 2 -> Phase 3 -> Phase 4
- [P] tasks within a phase can run in parallel
- Phase 2 stress tests can begin once Phase 1 API surface is settled (T001)
- Phase 3 benchmarks can begin once Phase 2 hardening is stable
- Phase 4 integration patterns can begin once Phase 1 contracts are documented

## Verification (after all phases)

- [ ] `cargo check -p nebula-memory --all-features`
- [ ] `cargo test -p nebula-memory`
- [ ] `cargo clippy -p nebula-memory -- -D warnings`
- [ ] `cargo doc --no-deps -p nebula-memory`
