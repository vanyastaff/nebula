# Implementation Plan: nebula-sandbox

**Crate**: `nebula-sandbox` (or use nebula-runtime sandbox) | **Path**: `crates/sandbox` | **ROADMAP**: [ROADMAP.md](ROADMAP.md)

## Summary

The sandbox crate provides execution isolation for untrusted or capability-gated actions. It defines the capability model schema, enforces sandbox policies in `SandboxedContext`, and routes actions to appropriate backends (in-process, wasm, subprocess). Current focus is stabilizing the port docs, capability violation contracts, and backend selection guardrails.

## Technical Context

**Language/Edition**: Rust 2024 (MSRV 1.93)
**Async Runtime**: Tokio
**Key Dependencies**: `nebula-action`, `nebula-core`, `nebula-execution`
**Testing**: `cargo test -p nebula-sandbox`

## Current Status

| Phase | Status | Summary |
|-------|--------|---------|
| Phase 1: Contract and Safety Baseline | ⬜ Planned | Stabilize port docs, capability model, backend selection |
| Phase 2: Runtime Hardening | ⬜ Planned | Violation/audit events, capability checks in SandboxedContext |
| Phase 3: Scale and Performance | ⬜ Planned | Benchmark sandbox overhead per backend |
| Phase 4: Ecosystem and DX | ⬜ Planned | wasm/process isolation backends, authoring guidelines |

## Phase Details

### Phase 1: Contract and Safety Baseline

**Goal**: Stabilize sandbox port docs; define capability model schema; add policy guardrails for backend selection.

**Deliverables**:
- Sandbox port docs and runtime integration points stabilized
- Capability model schema and violation error contracts defined
- Policy guardrails for backend selection

**Exit Criteria**:
- Contract tests pass for in-process path and cancellation/error propagation

**Risks**:
- Mismatch between action metadata and enforceable capability semantics

### Phase 2: Runtime Hardening

**Goal**: Structured violation/audit events; enforce capability checks; improve fallback policy.

**Deliverables**:
- Structured violation/audit events and observability dashboards
- Capability checks enforced in `SandboxedContext` access paths
- Policy-driven fallback behavior on backend issues

**Exit Criteria**:
- Deterministic violation handling with low false-positive rate

**Risks**:
- False positives in capability checks causing execution failures

### Phase 3: Scale and Performance

**Goal**: Benchmark overhead per backend; optimize hot-path; establish SLOs.

**Deliverables**:
- Criterion benchmarks for sandbox overhead per backend
- Optimized hot-path context and serialization boundaries
- SLOs for sandbox decision + execution overhead

**Exit Criteria**:
- Overhead within accepted runtime budget for trusted workloads

**Risks**:
- Added policy checks increasing action latency

### Phase 4: Ecosystem and DX

**Goal**: Full-isolation backends (wasm/process); action authoring guidelines; migration tooling.

**Deliverables**:
- Full-isolation backend: `wasm` and/or `process`
- Action authoring guidelines for capability declarations
- Migration and compatibility tooling for backend transitions

**Exit Criteria**:
- Production-ready path for untrusted/community actions

## Inter-Crate Dependencies

- **Depends on**: `nebula-action` (capability model in ActionMetadata), `nebula-core`, `nebula-execution`
- **Depended by**: `nebula-runtime` (delegates isolated execution to sandbox)

## Verification

- [ ] `cargo check -p nebula-sandbox`
- [ ] `cargo test -p nebula-sandbox`
- [ ] `cargo clippy -p nebula-sandbox -- -D warnings`
- [ ] `cargo doc --no-deps -p nebula-sandbox`
