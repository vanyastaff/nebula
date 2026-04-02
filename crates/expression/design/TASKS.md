# Tasks: nebula-expression

**ROADMAP**: [ROADMAP.md](ROADMAP.md) | **PLAN**: [PLAN.md](PLAN.md)

## Format: `[ID] [P?] Description`

- **[P]**: Can run in parallel with other [P] tasks in same phase
- IDs use prefix `EXP`

---

## Phase 1: Contract and Safety Baseline ✅

**Goal**: Stable public API, evaluator safety guards, doc alignment.

- [x] EXP-T001 Formalize stable public API surface — annotate with stability levels
- [x] EXP-T002 Document evaluator safety guard behavior in API.md
- [x] EXP-T003 Align function naming and behavior docs with implementation
- [x] EXP-T004 Validate stable contract docs via integration tests

**Checkpoint**: ✅ Done — stable contract docs validated by integration tests.

---

## Phase 2: Runtime Hardening ✅

**Goal**: Error context consistency; integration tests with runtime/action/parameter; edge case hardening.

- [x] EXP-T005 Improve error context — ensure all errors include expression location and context
- [x] EXP-T006 [P] Add integration tests with nebula-action expressions in `tests/action_integration.rs`
- [x] EXP-T007 [P] Add integration tests with nebula-parameter value types
- [x] EXP-T008 Harden context resolution edge cases — null access, missing fields, type coercion

**Checkpoint**: ✅ Done — deterministic failures; no contract ambiguities.

---

## Phase 3: Scale and Performance 🔄

**Goal**: Benchmark-driven cache tuning; hot-path optimization; expose cache observability.

- [ ] EXP-T009 [P] Add criterion benchmarks for expression evaluation in `benches/eval.rs`
- [ ] EXP-T010 [P] Add criterion benchmarks for template rendering overhead
- [ ] EXP-T011 Profile and optimize hot evaluator paths — minimize allocations in common cases
- [ ] EXP-T012 Optimize template rendering overhead — pre-compile templates where possible
- [ ] EXP-T013 Verify `ExpressionEngine::cache_overview()` provides actionable cache hit/miss info
- [ ] EXP-T014 [P] Add cache size and eviction metrics export
- [ ] EXP-T015 Verify throughput/latency improvements with semantic parity (no behavior change)

**Checkpoint**: Criterion benchmarks committed; measurable throughput gains; cache_overview() useful.

---

## Phase 4: Ecosystem and DX ⬜

**Goal**: Strict mode rollout; migration tooling; production examples.

- [ ] EXP-T016 Expand strict mode coverage — add more strict checks for boolean/type safety
- [ ] EXP-T017 [P] Write migration tooling for function/grammar evolution across versions
- [ ] EXP-T018 [P] Add production examples for common workflow expression patterns
- [ ] EXP-T019 Document compatibility modes in README — when to use strict vs compatibility mode
- [ ] EXP-T020 Write operator guidance for enabling strict mode in production

**Checkpoint**: Strict mode covers all common misuse patterns; migration tooling available; examples documented.

---

## Dependencies & Execution Order

- Phases 1–2 complete; current work is Phase 3
- Phase 3 → Phase 4 (sequential)
- [P] tasks within phases can run in parallel

## Verification (after all phases)

- [ ] `cargo check -p nebula-expression`
- [ ] `cargo test -p nebula-expression`
- [ ] `cargo clippy -p nebula-expression -- -D warnings`
- [ ] `cargo doc --no-deps -p nebula-expression`
- [ ] `cargo bench -p nebula-expression`
