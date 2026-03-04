# Implementation Plan: nebula-resource

**Crate**: `nebula-resource` | **Path**: `crates/resource` | **ROADMAP**: [ROADMAP.md](ROADMAP.md)

## Summary

The resource crate manages lifecycle, pooling, and health of external resources (databases, HTTP clients, queues, caches). It provides typed resource handles, scope-controlled access, pool management with backpressure, and event emission for health state changes. Current focus is contract consolidation — finalizing cross-crate contracts (INTERACTIONS, API, MIGRATION) and formalizing error handling guidance.

## Technical Context

**Language/Edition**: Rust 2024 (MSRV 1.93)
**Async Runtime**: Tokio
**Key Dependencies**: `nebula-core`, `nebula-system` (pressure events), `nebula-telemetry`, `nebula-storage` (optional, for state)
**Feature Flags**: resource-specific driver flags (postgres, redis, kafka, http, etc.)
**Testing**: `cargo test -p nebula-resource`

## Current Status

| Phase | Status | Summary |
|-------|--------|---------|
| Phase 1: Contract and Safety Baseline | 🔄 In Progress | Finalizing INTERACTIONS, API, MIGRATION docs |
| Phase 2: Runtime Hardening | ⬜ Planned | Shutdown/reload tests, health-to-quarantine observability |
| Phase 3: Scale and Performance | ⬜ Planned | Benchmark-driven acquire latency, backpressure policies |
| Phase 4: Ecosystem and DX | ⬜ Planned | Adapter crate guidance, typed key migration, cookbook |

## Phase Details

### Phase 1: Contract and Safety Baseline

**Goal**: Finalize cross-crate contracts; formalize error handling; lock scope invariants.

**Deliverables**:
- Finalized `INTERACTIONS.md`, `API.md`, `MIGRATION.md` published
- Error handling guidance: retryable, fatal, validation categories
- Scope invariants locked; deny-by-default behavior documented

**Exit Criteria**:
- Contract docs map 1:1 with implementation
- No unresolved scope or error taxonomy ambiguities

**Risks**:
- Hidden assumptions in action/runtime integration paths

### Phase 2: Runtime Hardening

**Goal**: Strengthen shutdown and reload behavior tests; improve health-to-quarantine observability.

**Deliverables**:
- Deterministic shutdown tests under in-flight load
- Health-to-quarantine propagation observability
- Operational guardrails for invalid config reload attempts

**Exit Criteria**:
- Deterministic shutdown in stress tests
- No leaked permit/instance in CI race scenarios

**Risks**:
- Regressions in pool swap and long-tail create/cleanup failures

### Phase 3: Scale and Performance

**Goal**: Benchmark-driven acquire latency tuning; backpressure policy profiles; high-cardinality metrics.

**Deliverables**:
- Criterion benchmarks for acquire latency and pool contention
- Backpressure policy profiles: fail-fast, bounded wait, adaptive
- High-cardinality metrics hygiene for multi-tenant workloads

**Exit Criteria**:
- p95 acquire latency and exhaustion rates within SLO targets
- No perf regressions vs current throughput baseline

**Risks**:
- Policy complexity and incorrect defaults under burst traffic

### Phase 4: Ecosystem and DX

**Goal**: Adapter crate guidance; typed key migration; cookbook for runtime/action integration.

**Deliverables**:
- Adapter crate guidance (resource-postgres, resource-redis, etc.)
- Typed key migration path and developer ergonomics updates
- Cookbook for runtime/action integration patterns

**Exit Criteria**:
- At least one reference adapter and end-to-end sample integration

## Inter-Crate Dependencies

- **Depends on**: `nebula-core`, `nebula-system` (pressure events), `nebula-telemetry`
- **Depended by**: `nebula-action` (resource access in context), `nebula-engine`, `nebula-runtime`, `nebula-credential` (optional)

## Verification

- [ ] `cargo check -p nebula-resource`
- [ ] `cargo test -p nebula-resource`
- [ ] `cargo clippy -p nebula-resource -- -D warnings`
- [ ] `cargo doc --no-deps -p nebula-resource`
