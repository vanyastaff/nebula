# Tasks: nebula-system

**Roadmap**: [ROADMAP.md](ROADMAP.md) | **Plan**: [PLAN.md](PLAN.md)

## Legend

- `[P]` — Can run in parallel with other `[P]` tasks in same phase
- `SYS-TXXX` — Task ID
- `→` — Depends on previous task

---

## Phase 1: Contract and Safety Baseline (Done)

**Goal**: Establish stable public API with documented error semantics and cross-platform CI.

- [x] SYS-T001 [P] Define stable public API surface
- [x] SYS-T002 [P] Document error semantics for all public functions
- [x] SYS-T003 [P] Set up CI on Linux/macOS/Windows
- [x] SYS-T004 Verify no known safety issues in unsafe blocks (→ T001)
- [x] SYS-T005 Write complete API docs (→ T001, T002)

**Checkpoint**: `cargo test --workspace` passes; docs complete; no known safety issues.

---

## Phase 2: Runtime Hardening

**Goal**: Integrate with nebula-memory for pressure-based decisions and add optional metrics.

- [ ] SYS-T006 [P] Design pressure-based API for nebula-memory integration (`src/pressure.rs`)
- [ ] SYS-T007 [P] Define metrics export trait/interface for system metrics
- [ ] SYS-T008 Implement pressure detection integration with nebula-memory (→ T006)
- [ ] SYS-T009 Implement optional metrics export (→ T007)
- [ ] SYS-T010 Write integration tests for nebula-memory pressure feedback (→ T008)
- [ ] SYS-T011 Add benchmarks for pressure detection paths and verify within budget (→ T008)

**Checkpoint**: nebula-memory integration tests pass; benchmarks within budget.

---

## Phase 3: Scale and Performance

**Goal**: Optimize system info collection for scale.

- [ ] SYS-T012 [P] Optimize process list collection for large process counts
- [ ] SYS-T013 [P] Implement configurable refresh intervals for system info caches
- [ ] SYS-T014 [P] Investigate and implement NUMA awareness where applicable
- [ ] SYS-T015 Add benchmark: process list <5ms for 100 processes (→ T012)
- [ ] SYS-T016 Document platform-specific limits and behavior differences (→ T012, T013, T014)

**Checkpoint**: Process list <5ms for 100 processes; documented platform limits.

---

## Phase 4: Ecosystem and DX

**Goal**: Add observability integrations and developer experience improvements.

- [ ] SYS-T017 [P] Implement OpenTelemetry/Prometheus metrics feature (`src/metrics.rs`)
- [ ] SYS-T018 [P] Implement health check helpers (`src/health.rs`)
- [ ] SYS-T019 [P] Add async wrappers for blocking system calls (optional, behind `async` feature)
- [ ] SYS-T020 Document metrics feature usage and configuration (→ T017)
- [ ] SYS-T021 Write examples for common workflows (→ T017, T018, T019)

**Checkpoint**: Metrics feature documented; examples for common workflows available.

---

## Dependencies & Execution Order

- Phase 1 → Phase 2 → Phase 3 → Phase 4
- [P] tasks within a phase can run in parallel

## Verification (after all phases)

- [ ] `cargo check -p nebula-system --all-features`
- [ ] `cargo test -p nebula-system`
- [ ] `cargo clippy -p nebula-system -- -D warnings`
- [ ] `cargo doc --no-deps -p nebula-system`
