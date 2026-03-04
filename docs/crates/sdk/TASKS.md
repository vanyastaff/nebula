# Tasks: nebula-sdk

**ROADMAP**: [ROADMAP.md](ROADMAP.md) | **PLAN**: [PLAN.md](PLAN.md)

## Format: `[ID] [P?] Description`

- **[P]**: Can run in parallel with other [P] tasks in same phase
- IDs use prefix `SDK`

---

## Phase 1: Contract and Prelude Stability ⬜

**Goal**: Formal stability policy; compatibility tests; aligned docs.

- [ ] SDK-T001 Write formal prelude stability policy in README — additive in minor, removal = major
- [ ] SDK-T002 [P] Add compatibility tests: macro output (`#[derive(Action)]`) satisfies nebula-action contract
- [ ] SDK-T003 [P] Add compatibility tests: `NodeBuilder` and `TriggerBuilder` output work with engine/runtime
- [ ] SDK-T004 Align API.md with actual prelude exports — list every re-export with stability annotation
- [ ] SDK-T005 Align README with actual builders and testing modules
- [ ] SDK-T006 Add CI test: SDK compatibility passes against current nebula-action, nebula-core, nebula-macros

**Checkpoint**: Prelude content documented; compatibility tests green; no undocumented breaking re-exports.

---

## Phase 2: Testing and Authoring DX ⬜

**Goal**: TestContext/MockExecution stable; feature flag matrix; assertion helpers.

- [ ] SDK-T007 Sync `TestContext` with current nebula-action/runtime contract — update any stale assertions
- [ ] SDK-T008 [P] Sync `MockExecution` — verify it matches real runtime execution behavior
- [ ] SDK-T009 Define feature flag matrix: `minimal` (types only), `testing` (TestContext, MockExecution), `full`
- [ ] SDK-T010 [P] Implement `ExecutionHarness` assertion helpers in `src/testing/harness.rs`
- [ ] SDK-T011 Document feature matrix in README — which features to use in release vs CI builds
- [ ] SDK-T012 Verify authors can write and test nodes using SDK only (integration test without nebula-runtime)

**Checkpoint**: Authors can write and test nodes with SDK only; feature matrix clear; MockExecution matches real behavior.

---

## Phase 3: Codegen and Tooling ⬜

**Goal**: Optional codegen feature; dev server; SDK stays lean.

- [ ] SDK-T013 Implement OpenAPI spec generation behind `codegen` feature flag in `src/codegen/mod.rs`
- [ ] SDK-T014 [P] Implement type generator (Rust types from schema) behind `codegen` feature flag
- [ ] SDK-T015 [P] Implement dev server with hot-reload behind `dev-server` feature flag
- [ ] SDK-T016 Verify default features keep SDK lean — no heavy deps in minimal build
- [ ] SDK-T017 Document optional features in README with examples

**Checkpoint**: Optional features documented and gated; default build is lean; codegen produces valid output.

---

## Phase 4: Ecosystem and Compatibility ⬜

**Goal**: Version matrix; migration guide; cookbook.

- [ ] SDK-T018 Write version compatibility matrix in README — SDK X ↔ core/action/runtime Y
- [ ] SDK-T019 [P] Write migration guide for prelude/builder breaking changes (major version)
- [ ] SDK-T020 [P] Write cookbook: create an action node with TestContext-driven tests
- [ ] SDK-T021 [P] Write cookbook: create a plugin with multiple action components
- [ ] SDK-T022 Publish example first-party action using SDK prelude

**Checkpoint**: External authors have predictable upgrade path; cookbook examples work end-to-end.

---

## Dependencies & Execution Order

- Phase 1 → Phase 2 → Phase 3 → Phase 4 (sequential)
- Phase 1 depends on `nebula-action` Phase 2 and `nebula-macros` Phase 1 being stable
- [P] tasks within phases can run in parallel

## Verification (after all phases)

- [ ] `cargo check -p nebula-sdk --all-features`
- [ ] `cargo test -p nebula-sdk`
- [ ] `cargo clippy -p nebula-sdk -- -D warnings`
- [ ] `cargo doc --no-deps -p nebula-sdk`
