# Implementation Plan: nebula-sdk

**Crate**: `nebula-sdk` | **Path**: `crates/sdk` | **ROADMAP**: [ROADMAP.md](ROADMAP.md)

## Summary

The SDK crate is the all-in-one developer toolkit for action and plugin authors — a facade that re-exports stable prelude, builders (`NodeBuilder`, `TriggerBuilder`), and testing utilities (`TestContext`, `MockExecution`). No orchestration lives in SDK; it is a versioned facade over core/action/runtime. Current focus is Phase 1: prelude stability and compatibility tests.

## Technical Context

**Language/Edition**: Rust 2024 (MSRV 1.93)
**Async Runtime**: Tokio
**Key Dependencies**: `nebula-action`, `nebula-core`, `nebula-macros`, `nebula-parameter`, `nebula-expression`
**Feature Flags**: `testing`, `codegen`, `full`
**Testing**: `cargo test -p nebula-sdk`

## Current Status

| Phase | Status | Summary |
|-------|--------|---------|
| Phase 1: Contract and Prelude Stability | ⬜ Planned | Formal prelude stability, compatibility tests, API.md alignment |
| Phase 2: Testing and Authoring DX | ⬜ Planned | TestContext/MockExecution sync, feature flags, assertion helpers |
| Phase 3: Codegen and Tooling | ⬜ Planned | Optional codegen feature, dev server |
| Phase 4: Ecosystem and Compatibility | ⬜ Planned | Version matrix, migration guide, cookbook |

## Phase Details

### Phase 1: Contract and Prelude Stability

**Goal**: Formal prelude stability policy; compatibility tests; API.md and README aligned.

**Deliverables**:
- Prelude stability policy: minor = additive only; removal/signature change = major
- Compatibility tests: macro output and builder output conform to nebula-action contract
- API.md and README aligned with actual prelude, builders, and testing modules

**Exit Criteria**:
- Prelude content documented and test-covered; no undocumented breaking re-exports
- NodeBuilder/TriggerBuilder + derive output work with engine/runtime

**Risks**:
- Downstream crates changing contract without SDK update

### Phase 2: Testing and Authoring DX

**Goal**: TestContext + MockExecution contracts stable; feature flag matrix; assertion helpers.

**Deliverables**:
- TestContext and MockExecution synced with action/runtime expectations
- Optional deps/feature flags: `minimal` (types only) vs `full` (builders, testing, codegen)
- Assertion helpers and ExecutionHarness — additive only in minor

**Exit Criteria**:
- Authors can write and test nodes with SDK only
- Feature matrix clear for release builds and CI

**Risks**:
- Test utilities drifting from real runtime behavior

### Phase 3: Codegen and Tooling

**Goal**: Optional codegen feature; dev server; no new domain logic.

**Deliverables**:
- Optional codegen: OpenAPI spec generation, type generators (behind `codegen` feature flag)
- Dev server with hot-reload (optional DX, behind `dev-server` feature)
- No new domain logic in SDK

**Exit Criteria**:
- Optional features documented; default features keep SDK lean

### Phase 4: Ecosystem and Compatibility

**Goal**: Version compatibility matrix; migration guide; cookbook.

**Deliverables**:
- Version compatibility matrix: SDK X works with core/action/runtime Y
- Migration guide for prelude or builder breaking changes
- Cookbook: authoring nodes and testing with TestContext/MockExecution

**Exit Criteria**:
- External action authors can depend on SDK with predictable upgrades

## Inter-Crate Dependencies

- **Depends on**: `nebula-action`, `nebula-core`, `nebula-macros`, `nebula-parameter`, `nebula-expression`
- **Depended by**: Action/plugin authors (external consumers), `nebula-api` (type re-exports)

## Verification

- [ ] `cargo check -p nebula-sdk --all-features`
- [ ] `cargo test -p nebula-sdk`
- [ ] `cargo clippy -p nebula-sdk -- -D warnings`
- [ ] `cargo doc --no-deps -p nebula-sdk`
