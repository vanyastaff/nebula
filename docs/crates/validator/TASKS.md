# Tasks: nebula-validator

**Roadmap**: [ROADMAP.md](ROADMAP.md) | **Plan**: [PLAN.md](PLAN.md)

## Legend

- `[P]` — Can run in parallel with other `[P]` tasks in same phase
- `VAL-TXXX` — Task ID
- `→` — Depends on previous task

---

## Phase 1: Contract and Docs Baseline (Done)

**Goal**: Establish authoritative documentation and contract baseline.

- [x] VAL-T001 [P] Write template-aligned docs with cross-crate contracts
- [x] VAL-T002 [P] Create canonical API examples aligned to current implementation
- [x] VAL-T003 [P] Draft explicit compatibility policy
- [x] VAL-T004 Verify no stale naming mismatch in public docs (→ T001, T002)

**Checkpoint**: Docs accepted as single source of truth; no naming mismatches.

---

## Phase 2: Compatibility and Governance (In Progress)

**Goal**: Automate compatibility enforcement and governance checks.

- [ ] VAL-T005 [P] Define machine-readable error code/category registry artifact (e.g. `docs/crates/validator/error-registry.json`)
- [ ] VAL-T006 [P] Add compatibility tests for error codes against registry (`crates/validator/tests/compat/`)
- [ ] VAL-T007 [P] Add compatibility tests for field paths against registry (`crates/validator/tests/compat/`)
- [ ] VAL-T008 [P] Write deprecation and migration policy document (`docs/crates/validator/MIGRATION.md`)
- [ ] VAL-T009 Add CI job that fails on non-additive minor-contract edits without migration mapping (→ T005, T008)
- [ ] VAL-T010 Expand cross-crate fixture assertions for config/api adapters (→ T006, T007)
- [ ] VAL-T011 Freeze serializer envelope examples used by operator tooling (→ T010)

**Checkpoint**: Backward compatibility CI checks in place; migration-map checks running for release candidates.

---

## Phase 3: Performance and Capacity Hardening

**Goal**: Establish performance budgets and optimize hot paths.

- [ ] VAL-T012 [P] Define benchmark budgets for common validator/combinator chains (`benches/`)
- [ ] VAL-T013 [P] Define bench profiles: quick PR profile vs release profile
- [ ] VAL-T014 [P] Document hard threshold policy and exception process
- [ ] VAL-T015 Write cache strategy guidance for expensive checks using moka (→ T012)
- [ ] VAL-T016 Profile and optimize allocation in heavy failure paths (→ T012)
- [ ] VAL-T017 Add CI benchmark threshold enforcement (→ T012, T013, T014)

**Checkpoint**: Benchmark thresholds enforced in CI; no allocation regressions.

---

## Phase 4: Ecosystem and DX

**Goal**: Expand usability for downstream consumers and plugin authors.

- [ ] VAL-T018 [P] Document advanced validation patterns for workflow/plugin/sdk consumers
- [ ] VAL-T019 [P] Evaluate optional schema/policy layer and document findings
- [ ] VAL-T020 Write macro debugging and authoring guidance for derive validators (→ T018)
- [ ] VAL-T021 Define stable core vs optional extension boundary and document (→ T018, T019)

**Checkpoint**: Clear stable core vs optional extension boundaries documented; advanced patterns available.

---

## Dependencies & Execution Order

- Phase 1 → Phase 2 → Phase 3 → Phase 4
- [P] tasks within a phase can run in parallel

## Verification (after all phases)

- [ ] `cargo check -p nebula-validator --all-features`
- [ ] `cargo test -p nebula-validator`
- [ ] `cargo clippy -p nebula-validator -- -D warnings`
- [ ] `cargo doc --no-deps -p nebula-validator`
