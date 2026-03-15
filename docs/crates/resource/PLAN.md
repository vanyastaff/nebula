# Implementation Plan: nebula-resource

**Crate**: `nebula-resource` | **Path**: `crates/resource` | **ROADMAP**: [ROADMAP.md](ROADMAP.md)

## Summary

The resource crate manages lifecycle, pooling, and health of external resources (databases, HTTP clients, queues, caches). It provides typed resource handles, scope-controlled access, pool management with backpressure, and event emission for health state changes. Core contract, hardening, performance, and DX phases are complete. Phase 5 Neon hardening (Poison guard, Gate, CounterGuard, dedicated CB metrics) is complete. Current focus is Phase 4 final item: reference adapter crate.

## Technical Context

**Language/Edition**: Rust 2024 (MSRV 1.93)
**Async Runtime**: Tokio
**Key Dependencies**: `nebula-core`, `nebula-system` (pressure events), `nebula-telemetry`, `nebula-storage` (optional, for state)
**Feature Flags**: resource-specific driver flags (postgres, redis, kafka, http, etc.)
**Testing**: `cargo test -p nebula-resource`

## Current Status

| Phase | Status | Summary |
|-------|--------|---------|
| Phase 1: Contract and Safety Baseline | ✅ Complete | Contract docs, error taxonomy, scope invariants locked |
| Phase 2: Runtime Hardening | ✅ Complete | Shutdown/reload tests, health-to-quarantine observability |
| Phase 3: Scale and Performance | ✅ Complete | Criterion benchmarks, backpressure policies, metrics hygiene |
| Phase 4: Ecosystem and DX | 🔄 In Progress | Adapter guides, typed key migration, cookbook; RSC-T020 pending |
| Phase 5: Neon Hardening | ✅ Complete | `Poison<T>` guard, Gate/GateGuard, CounterGuard, dedicated CB metrics counters |

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

### Phase 5: Neon Hardening

**Goal**: Apply Neon-inspired safety primitives to the resource pool for correct cooperative shutdown and RAII observability.

**Deliverables**:
- `Poison<T>` / `PoisonGuard` / `PoisonError` in `crates/resource/src/poison.rs`; `PoolState` wrapped in `Mutex<Poison<PoolState>>`; drop-without-disarm permanently marks the pool poisoned with timestamp
- `Gate`/`GateGuard` cooperative shutdown barrier in `nebula-resilience::gate`; wired into `PoolInner` (maintenance task holds `GateGuard`; `shutdown()` calls `gate.close().await` before `semaphore.close()`)
- `CounterGuard` RAII replace manual `fetch_add`/`fetch_sub` pairs in `acquire_inner`
- Dedicated `NEBULA_RESOURCE_CIRCUIT_BREAKER_OPENED_TOTAL` / `NEBULA_RESOURCE_CIRCUIT_BREAKER_CLOSED_TOTAL` counters with `{resource_id, operation}` label via `MetricsCollector`

**Exit Criteria**:
- All six `gate` unit tests + doctest pass
- `cargo check --workspace --all-targets` clean
- Hardening checklist rows in ARCHITECTURE.md show Implemented

## Inter-Crate Dependencies

- **Depends on**: `nebula-core`, `nebula-system` (pressure events), `nebula-telemetry`
- **Depended by**: `nebula-action` (resource access in context), `nebula-engine`, `nebula-runtime`, `nebula-credential` (optional)

## Verification

- [x] `cargo check -p nebula-resource`
- [x] `cargo test -p nebula-resource`
- [x] `cargo clippy -p nebula-resource -- -D warnings`
- [x] `cargo doc --no-deps -p nebula-resource`
