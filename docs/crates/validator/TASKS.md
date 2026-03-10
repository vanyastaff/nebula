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

## Phase 2: Compatibility and Governance (Complete)

**Goal**: Automate compatibility enforcement and governance checks.

- [x] VAL-T005 [P] Define machine-readable error code/category registry artifact (`tests/fixtures/compat/error_registry_v1.json` — 43 codes, 7 categories)
- [x] VAL-T006 [P] Add compatibility tests for error codes against registry (boolean, pattern, network, temporal, length fixtures)
- [x] VAL-T007 [P] Add compatibility tests for field paths against registry (`field_path_contract_v1.json` with json_field combinator)
- [x] VAL-T008 [P] Write deprecation and migration policy document (`docs/crates/validator/MIGRATION.md`)
- [x] VAL-T009 Add governance contract tests: stability values, semver, migration doc cross-references
- [x] VAL-T010 Expand cross-crate fixture assertions: envelope fixtures created (`envelope_contract_v1.json`); consumer-side config/api tests are separate work
- [x] VAL-T011 Freeze serializer envelope examples used by operator tooling (`envelope_contract_v1.json` + envelope shape tests)

**Checkpoint**: Backward compatibility CI checks in place; migration-map checks running for release candidates.

---

## Phase 3: Performance and Capacity Hardening ✅

**Goal**: Establish performance budgets and optimize hot paths.

- [x] VAL-T012 [P] Define benchmark budgets for common validator/combinator chains (`tests/fixtures/perf/benchmark_budgets_v1.json`)
- [x] VAL-T013 [P] Define bench profiles: quick PR profile vs release profile (`scripts/bench-validator.{ps1,sh}`)
- [x] VAL-T014 [P] Document hard threshold policy and exception process (`docs/crates/validator/PERFORMANCE.md`)
- [x] VAL-T015 Write cache strategy guidance for expensive checks using moka (`benches/cache.rs` + PERFORMANCE.md §Cache Strategy)
- [x] VAL-T016 Profile and optimize allocation in heavy failure paths (`benches/error_construction.rs` + PERFORMANCE.md §Allocation Profile)
- [x] VAL-T017 Add CI benchmark threshold enforcement (`tests/contract/benchmark_budget_test.rs` — budget integrity + memory layout)

**Checkpoint**: Benchmark thresholds enforced in CI; no allocation regressions. ✅ 451 tests green.

---

## Phase 4: Ecosystem and DX ✅

**Goal**: Expand usability for downstream consumers and plugin authors.

- [x] VAL-T018 [P] Document advanced validation patterns for workflow/plugin/sdk consumers (`PATTERNS.md` — 13 recipes)
- [x] VAL-T019 [P] Evaluate optional schema/policy layer and document findings (`SCHEMA_EVALUATION.md` — recommend defer)
- [x] VAL-T020 Write macro debugging and authoring guidance for derive validators (`MACROS.md` — 7 sections)
- [x] VAL-T021 Define stable core vs optional extension boundary and document (`BOUNDARIES.md` — 3 tiers)

**Checkpoint**: Clear stable core vs optional extension boundaries documented; advanced patterns available. ✅

---

## Dependencies & Execution Order

- Phase 1 → Phase 2 → Phase 3 → Phase 4
- [P] tasks within a phase can run in parallel

## Verification (after all phases)

- [x] `cargo check -p nebula-validator --all-features`
- [x] `cargo test -p nebula-validator` — 451 tests passing
- [x] `cargo clippy -p nebula-validator -- -D warnings` — clean
- [x] `cargo doc --no-deps -p nebula-validator`
