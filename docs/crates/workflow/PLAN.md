# Implementation Plan: nebula-workflow

**Crate**: `nebula-workflow` | **Path**: `crates/workflow` | **ROADMAP**: [ROADMAP.md](ROADMAP.md)

## Summary

`nebula-workflow` provides the formal workflow definition, DAG graph model, and validation used by the engine and API. It ensures a single source of truth for workflow schema -- no divergent types across engine, API, or UI. Phase 1 (contract and schema baseline) is complete. Current focus is schema stability and validation integrations.

## Technical Context

**Language/Edition**: Rust 2024 (MSRV 1.93)
**Async Runtime**: Tokio (dev-dependencies only)
**Key Dependencies**: `nebula-core`, `petgraph`, `serde`, `serde_json`, `chrono`, `thiserror`
**Testing**: `cargo test -p nebula-workflow`

## Current Status

| Phase | Status | Summary |
|-------|--------|---------|
| Phase 1: Contract and Schema Baseline | ✅ Done | WorkflowDefinition, DAG API, validation, builder |
| Phase 2: Schema Stability and Compatibility | ⬜ Planned | Snapshot tests, version field, compatibility policy |
| Phase 3: Validation and Integrations | ⬜ Planned | nebula-validator integration, field-path errors |
| Phase 4: Ecosystem and DX | ⬜ Planned | Builder ergonomics, migration tooling |

## Phase Details

### Phase 1: Contract and Schema Baseline

**Goal**: Establish formal workflow definition and DAG API as the single source of truth.

**Deliverables**:
- Formal `WorkflowDefinition` and DAG API used by engine and API; no divergent types
- Cycle and ref validation: `validate_workflow()` rejects invalid graphs; structured `WorkflowError`
- Docs (ARCHITECTURE, API) aligned with current types (definition, node, connection, graph, builder)

**Exit Criteria**:
- Engine and API depend on workflow crate for definition and graph only
- All validation paths covered by tests; no invalid workflow accepted for execution

**Risks**:
- Engine or API introducing workflow-shaped types outside this crate, causing drift

### Phase 2: Schema Stability and Compatibility

**Goal**: Guarantee serialized schema stability via snapshot tests and versioning.

**Deliverables**:
- Schema snapshot tests (JSON fixtures) for `WorkflowDefinition`, nodes, connections; CI enforces roundtrip
- Version field and compatibility policy: patch/minor = additive only; major = MIGRATION.md
- Document serialized form in API.md; compatibility rules in MIGRATION.md or CONSTITUTION

**Exit Criteria**:
- Fixtures in repo; CI checks public types roundtrip
- No breaking change without major version + MIGRATION.md

**Risks**:
- New fields added without snapshot update; breaking clients or storage

**Dependencies**: Phase 1 complete

### Phase 3: Validation and Integrations

**Goal**: Composable validation with field-path errors suitable for API responses.

**Deliverables**:
- Optional integration with `nebula-validator` for composable rules (if adopted)
- Validation errors sufficient for API 400 responses with field path
- No UI-only or execution-only fields in workflow definition; design-time DAG only

**Exit Criteria**:
- Validation contract documented; API and engine use same validation entry point
- Definition remains design-time only; execution extensions live in execution/engine

**Risks**:
- Scope creep: ephemeral nodes or execution state leaking into definition

**Dependencies**: nebula-validator (optional)

### Phase 4: Ecosystem and DX

**Goal**: Great workflow authoring experience for API, CLI, and UI consumers.

**Deliverables**:
- Builder and validation ergonomics for API and CLI (workflow create/edit)
- Migration tooling or guidance for schema version bumps
- Operator guidance: when to validate, where errors surface

**Exit Criteria**:
- Clear path for workflow authoring and validation; low-friction adoption for API/UI

**Risks**:
- Fragmentation between builder API and raw struct usage

**Dependencies**: Phase 2 (version field), Phase 3 (validation)

## Inter-Crate Dependencies

- **Depends on**: nebula-core (identifiers, scope types)
- **Depended by**: nebula-execution (workflow types for execution planning), nebula-engine (DAG scheduling, workflow loading), nebula-api (workflow CRUD endpoints)

## Verification

- [ ] `cargo check -p nebula-workflow`
- [ ] `cargo test -p nebula-workflow`
- [ ] `cargo clippy -p nebula-workflow -- -D warnings`
- [ ] `cargo doc --no-deps -p nebula-workflow`
