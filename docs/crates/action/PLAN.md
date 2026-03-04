# Implementation Plan: nebula-action

**Crate**: `nebula-action` | **Path**: `crates/action` | **ROADMAP**: [ROADMAP.md](ROADMAP.md)

## Summary

`nebula-action` defines executable node contracts (traits, result types, error types, port declarations) for the Nebula workflow engine. It is a protocol crate, not a runtime. Current focus is finishing the contract freeze, stabilizing the context/capability model, and removing stale terminology.

## Technical Context

**Language/Edition**: Rust 2024 (MSRV 1.93)
**Async Runtime**: Tokio (via `tokio-util` for CancellationToken)
**Key Dependencies**: `nebula-core`, `nebula-credential`, `nebula-parameter`, `nebula-resource`, `async-trait`, `serde`, `serde_json`, `thiserror`, `chrono`, `tokio-util`
**Testing**: `cargo test -p nebula-action`

## Current Status

| Phase | Status | Summary |
|-------|--------|---------|
| Phase 1: Contract freeze and cleanup | 🔄 In Progress | Core surface locked; contract tests done; stale terminology remains |
| Phase 2: Context and capability model | 🔄 In Progress | ActionContext/TriggerContext added; Context trait methods and capability modules pending |
| Phase 3: Deferred and streaming hardening | ⬜ Planned | Lock deferred/streaming resolution behavior |
| Phase 4: Port and metadata governance | ⬜ Planned | Freeze port schema semantics, add compat checks |
| Phase 5: Ecosystem and DX rollout | ⬜ Planned | End-to-end examples, authoring layer |

## Phase Details

### Phase 1: Contract freeze and cleanup

**Goal**: Lock the current stable API surface and remove all ambiguity between current API and aspirational design.

**Deliverables**:
- Lock current stable surface (`Action`, metadata, components, result/output/error/ports) -- done
- Contract tests in `crates/action/tests/contracts.rs` for `ActionOutput`, `FlowKind` -- done
- Compatibility policy in `COMPATIBILITY.md` -- done
- Remove stale terminology in docs and examples (StatelessAction vs ProcessAction) — done for active docs; archive left as historical context

**Exit Criteria**:
- No ambiguity between current API and aspirational design
- Contract tests for serialization and compatibility pass -- done

**Dependencies**: None

### Phase 2: Context and capability model

**Goal**: Establish production context types and capability model (`ActionContext`/`TriggerContext`).

**Deliverables**:
- Replace temporary `NodeContext` bridge: `ActionContext` and `TriggerContext` added; `NodeContext` removed -- done
- `Context` trait with `execution_id()`, `node_id()`, `workflow_id()`, `cancellation()` methods
- Core execution traits: `StatelessAction`, `StatefulAction`, `TriggerAction`, `ResourceAction` -- done
- Capability modules (resources, credentials, logger) to be added by runtime/sandbox as fields on context structs

**Exit Criteria**:
- Engine/sandbox/runtime can all implement the same context contract -- done (trait + concrete types)
- Capability checks map to deterministic action errors (future: SandboxViolation on undeclared access)

**Risks**:
- Context trait method signatures may need revision if engine/runtime requirements conflict
- Capability module design must not leak engine-specific behavior into action traits

**Dependencies**: `nebula-resource` (ResourceProvider port), `nebula-credential` (CredentialProvider port)

### Phase 3: Deferred and streaming hardening

**Goal**: Lock deferred/streaming resolution behavior and define downstream compatibility.

**Deliverables**:
- Lock deferred/streaming resolution behavior expected from engine
- Define compatibility matrix for downstream nodes consuming each output form
- Document persistence/checkpoint requirements for long-running outputs

**Exit Criteria**:
- Resume/recovery scenarios for deferred outputs are fully specified
- Streaming backpressure semantics are testable and documented

**Risks**:
- Deferred output persistence depends on engine state store integration (nebula-engine Phase 1)
- Streaming backpressure may require changes to ActionOutput variants

**Dependencies**: `nebula-engine` (state store integration for deferred outputs)

### Phase 4: Port and metadata governance

**Goal**: Freeze port schema semantics and provide validation tooling for action authors.

**Deliverables**:
- Freeze dynamic/support port schema semantics
- Add compatibility checks for metadata version changes
- Provide validation tools for action package authors

**Exit Criteria**:
- CI-level contract validation for action packages
- Clear migration guide for version bumps

**Risks**:
- Port schema changes may break existing action implementations

**Dependencies**: None

### Phase 5: Ecosystem and DX rollout

**Goal**: Make it easy for external authors to build n8n-style action nodes.

**Deliverables**:
- Publish end-to-end examples with runtime + action implementations
- Define recommended error-to-retry mapping patterns
- Deliver ergonomic authoring layer in same crate (e.g. `dx`/`authoring` module)

**Exit Criteria**:
- External action authors can build n8n-style nodes with predictable behavior
- Runtime and sandbox integrations are documented end-to-end

**Risks**:
- DX layer must not create coupling between action crate and engine internals

**Dependencies**: `nebula-runtime` (sandbox in-crate)

## Inter-Crate Dependencies

- **Depends on**: `nebula-core`, `nebula-credential`, `nebula-parameter`, `nebula-resource`
- **Depended by**: `nebula-engine`, `nebula-runtime`, `nebula-plugin`

## Verification

- [ ] `cargo check -p nebula-action`
- [ ] `cargo test -p nebula-action`
- [ ] `cargo clippy -p nebula-action -- -D warnings`
- [ ] `cargo doc --no-deps -p nebula-action`
