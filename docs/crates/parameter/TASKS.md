# Tasks: nebula-parameter

**ROADMAP**: [ROADMAP.md](ROADMAP.md) | **PLAN**: [PLAN.md](PLAN.md)

## Format: `[ID] [P?] Description`

- **[P]**: Can run in parallel with other [P] tasks in same phase
- IDs use prefix `PAR`

---

## Phase 1: Contract and Safety Baseline ⬜

**Goal**: Align docs, stable naming conventions, document required/nullable behavior, schema lint.

- [ ] PAR-T001 [P] Align all docs and examples in `docs/crates/parameter/` with actual API
- [ ] PAR-T002 [P] Publish stable naming conventions for parameter keys/paths in API.md
- [ ] PAR-T003 Document required vs nullable behavior for each `ParameterKind` variant
- [ ] PAR-T004 Implement schema lint (`P-004`) — validate parameter schemas against naming and type rules
- [ ] PAR-T005 Add error code stability documentation to API.md
- [ ] PAR-T006 Verify all existing consumers pass the schema lint in CI

**Checkpoint**: All consumers pass lint; error codes stable and documented; required/nullable behavior documented.

---

## Phase 2: Runtime Hardening ⬜

**Goal**: Benchmarks for deep nested validation; stress tests; deterministic error ordering.

- [ ] PAR-T007 Add criterion benchmarks for deep nested object/list validation in `benches/validation.rs`
- [ ] PAR-T008 [P] Optimize recursive path building — minimize allocations in error paths
- [ ] PAR-T009 [P] Write stress tests for large collections (1000+ params) and high error counts
- [ ] PAR-T010 Implement deterministic error ordering (`P-002`) — same input always produces same error order
- [ ] PAR-T011 Verify no allocation regression in common validation paths (micro-benchmark)

**Checkpoint**: Benchmarks in CI; deterministic error ordering confirmed; no allocation regression.

---

## Phase 3: Scale and Performance ⬜

**Goal**: Typed extraction helpers; conversion contracts; optional typed value layer.

- [ ] PAR-T012 Add typed extraction helpers: `extract_string`, `extract_number`, `extract_bool` etc. in `src/extract.rs`
- [ ] PAR-T013 [P] Clarify and document conversion contracts for number/integer/decimal types
- [ ] PAR-T014 Reduce ambiguity in "any"-typed parameter flows — add guidance or stricter coercion
- [ ] PAR-T015 [P] Implement optional typed value layer (`P-001`) behind feature flag
- [ ] PAR-T016 Write migration path doc for transitioning to typed extraction API

**Checkpoint**: Typed API available; migration path documented; any-typed ambiguity addressed.

---

## Phase 4: Ecosystem and DX ⬜

**Goal**: Display rule analysis, key linting, ParameterKey newtype, ValidationRule versioning.

- [ ] PAR-T017 Implement dependency graph extraction from display rules in `src/display.rs`
- [ ] PAR-T018 [P] Add cycle detection in display rule dependency graphs
- [ ] PAR-T019 [P] Add contradictory visibility detection at schema build time
- [ ] PAR-T020 Add diagnostics for unreachable parameters in schema
- [ ] PAR-T021 Introduce `ParameterKey` newtype (`P-003`) replacing bare strings
- [ ] PAR-T022 Add versioning to `ValidationRule` (`P-005`) — include version metadata in persisted schemas
- [ ] PAR-T023 Write display rule lint and add to CI

**Checkpoint**: Display rule lint in CI; `ParameterKey` newtype adopted; validation schemas versioned.

---

## Dependencies & Execution Order

- Phase 1 → Phase 2 → Phase 3 → Phase 4 (sequential)
- [P] tasks within phases can run in parallel

## Verification (after all phases)

- [ ] `cargo check -p nebula-parameter`
- [ ] `cargo test -p nebula-parameter`
- [ ] `cargo clippy -p nebula-parameter -- -D warnings`
- [ ] `cargo doc --no-deps -p nebula-parameter`
- [ ] Schema lint passes against all existing parameter schemas
