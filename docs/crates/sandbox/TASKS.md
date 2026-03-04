# Tasks: nebula-sandbox

**ROADMAP**: [ROADMAP.md](ROADMAP.md) | **PLAN**: [PLAN.md](PLAN.md)

## Format: `[ID] [P?] Description`

- **[P]**: Can run in parallel with other [P] tasks in same phase
- IDs use prefix `SBX`

---

## Phase 1: Contract and Safety Baseline ⬜

**Goal**: Stabilize port docs, capability model schema, violation errors, backend selection policy.

- [ ] SBX-T001 [P] Update sandbox port docs — align with actual runtime integration points
- [ ] SBX-T002 Define capability model schema in `src/capability.rs` — what capabilities can be declared/denied
- [ ] SBX-T003 Define violation error contract in `src/error.rs` — `CapabilityViolation`, `BackendError`
- [ ] SBX-T004 Implement policy guardrails for backend selection — prevent invalid backend/action combinations
- [ ] SBX-T005 Write contract tests for in-process path in `tests/contracts.rs`
- [ ] SBX-T006 [P] Write contract tests for cancellation propagation through sandbox boundary
- [ ] SBX-T007 [P] Write contract tests for error propagation from sandboxed execution

**Checkpoint**: Contract tests green for in-process path; capability model schema defined; violation errors typed.

---

## Phase 2: Runtime Hardening ⬜

**Goal**: Structured violation/audit events; enforce capability checks in SandboxedContext; fallback policy.

- [ ] SBX-T008 Add structured `CapabilityViolationEvent` type and emit on any capability breach
- [ ] SBX-T009 Enforce capability checks in all `SandboxedContext` access paths (resources, credentials, etc.)
- [ ] SBX-T010 Implement policy-driven fallback behavior when backend is unavailable
- [ ] SBX-T011 [P] Add observability: violation count metrics via `nebula-telemetry`
- [ ] SBX-T012 Write deterministic violation tests — verify false-positive rate is low

**Checkpoint**: Deterministic violation handling; structured events emitted; low false-positive rate verified.

---

## Phase 3: Scale and Performance ⬜

**Goal**: Benchmark sandbox overhead; optimize hot-path; establish SLOs.

- [ ] SBX-T013 Add criterion benchmarks for in-process sandbox overhead in `benches/sandbox.rs`
- [ ] SBX-T014 [P] Add benchmarks for capability check overhead
- [ ] SBX-T015 Optimize `SandboxedContext` construction and serialization boundaries
- [ ] SBX-T016 Document SLO targets: sandbox decision overhead + execution overhead budgets

**Checkpoint**: Overhead within runtime budget for trusted workloads; benchmarks committed.

---

## Phase 4: Ecosystem and DX ⬜

**Goal**: Full-isolation backends; action authoring guidelines; migration tooling.

- [ ] SBX-T017 Implement wasm backend for full-isolation execution (behind `wasm` feature flag)
- [ ] SBX-T018 [P] Implement subprocess backend for process-level isolation (behind `process` feature flag)
- [ ] SBX-T019 Write action authoring guidelines for capability declarations
- [ ] SBX-T020 [P] Write migration tooling/guide for switching between sandbox backends
- [ ] SBX-T021 Write integration tests: untrusted action runs correctly in wasm/process backend

**Checkpoint**: Production-ready path for untrusted/community actions; wasm or process backend functional.

---

## Dependencies & Execution Order

- Phase 1 → Phase 2 → Phase 3 → Phase 4 (sequential)
- Phase 1 depends on `nebula-action` Phase 2 (ActionMetadata with capability model)
- [P] tasks within phases can run in parallel

## Verification (after all phases)

- [ ] `cargo check -p nebula-sandbox --all-features`
- [ ] `cargo test -p nebula-sandbox`
- [ ] `cargo clippy -p nebula-sandbox -- -D warnings`
- [ ] `cargo doc --no-deps -p nebula-sandbox`
