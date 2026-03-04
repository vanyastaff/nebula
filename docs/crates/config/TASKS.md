# Tasks: nebula-config

**Roadmap**: [ROADMAP.md](ROADMAP.md) | **Plan**: [PLAN.md](PLAN.md)

## Legend

- `[P]` — Can run in parallel with other `[P]` tasks in same phase
- `CFG-TXXX` — Task ID
- `→` — Depends on previous task

---

## Phase 1: Contract Baseline and Documentation (Done)

**Goal**: Align docs and fixtures as single source of truth.

- [x] CFG-T001 [P] Write SPEC-template docs and interaction contracts
- [x] CFG-T002 [P] Document precedence/path semantics in API and fixtures
- [x] CFG-T003 Codify governance/migration requirements in contract tests (→ T001, T002)

**Checkpoint**: Docs and fixtures treated as source of truth in CI.

---

## Phase 2: Compatibility and Validation Hardening (Done)

**Goal**: Harden validation integration and ensure backward compatibility.

- [x] CFG-T004 [P] Create compatibility fixtures for precedence/path/type conversion (`crates/config/tests/fixtures/compat/`)
- [x] CFG-T005 [P] Integrate direct validator trait bridge into ConfigValidator
- [x] CFG-T006 Add validator compatibility and governance contract tests (→ T004, T005)

**Checkpoint**: Contract suite remains green across crate changes and releases.

---

## Phase 3: Reliability and Reload Semantics (Mostly Done)

**Goal**: Ensure reliable reload behavior under all failure modes.

- [x] CFG-T007 [P] Verify atomic reload behavior with tests
- [x] CFG-T008 [P] Verify reload failure preserves last-known-good state
- [x] CFG-T009 [P] Document failure-mode guidance
- [ ] CFG-T010 Write watcher lifecycle/backoff guidance for high-frequency reload workloads
- [ ] CFG-T011 Add targeted stress tests for high-frequency reload scenarios (→ T010)

**Checkpoint**: Explicit watcher/backoff guidance documented; stress tests pass.

---

## Phase 4: Source Ecosystem Expansion

**Goal**: Expand config sources beyond local files.

- [ ] CFG-T012 [P] Design source adapter trait for remote/database/kv backends
- [ ] CFG-T013 [P] Define security model for remote source auth and trust
- [ ] CFG-T014 Implement database/kv source adapter (→ T012)
- [ ] CFG-T015 Implement remote HTTP/API source adapter (→ T012, T013)
- [ ] CFG-T016 Write contract tests for source adapters (→ T014, T015)
- [ ] CFG-T017 Write security tests for remote source auth (→ T013, T015)
- [ ] CFG-T018 Document source adapter usage and configuration (→ T014, T015)

**Checkpoint**: Source adapter contracts and security tests pass; documentation complete.

---

## Dependencies & Execution Order

- Phase 1 → Phase 2 → Phase 3 → Phase 4
- [P] tasks within a phase can run in parallel

## Verification (after all phases)

- [ ] `cargo check -p nebula-config --all-features`
- [ ] `cargo test -p nebula-config`
- [ ] `cargo clippy -p nebula-config -- -D warnings`
- [ ] `cargo doc --no-deps -p nebula-config`
