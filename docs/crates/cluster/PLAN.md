# Implementation Plan: nebula-cluster

**Crate**: `nebula-cluster` | **Path**: `crates/cluster` | **ROADMAP**: [ROADMAP.md](ROADMAP.md)

## Summary

The cluster crate manages multi-node distributed coordination — membership, workflow placement, failover, and autoscaling. It is planned for a future phase after the single-node execution engine is stable. The crate does not yet exist in `crates/`.

## Technical Context

**Language/Edition**: Rust 2024 (MSRV 1.93)
**Async Runtime**: Tokio
**Key Dependencies**: `nebula-core`, `nebula-storage` (control-plane state), `nebula-telemetry`, `nebula-system`
**Testing**: `cargo test -p nebula-cluster`

## Current Status

| Phase | Status | Summary |
|-------|--------|---------|
| Phase 1: Contract and Safety Baseline | ⬜ Planned | Create crate skeleton, membership model, placement API |
| Phase 2: Runtime Hardening | ⬜ Planned | Failover detection, durable control-plane state, observability |
| Phase 3: Scale and Performance | ⬜ Planned | Scheduler optimization, fairness, benchmark |
| Phase 4: Ecosystem and DX | ⬜ Planned | Operator APIs/CLI, autoscaling policy, runbooks |

## Phase Details

### Phase 1: Contract and Safety Baseline

**Goal**: Create crate with membership model, safe join/leave APIs, and basic placement with deterministic behavior.

**Deliverables**:
- `crates/cluster` skeleton and core contracts
- Membership model + safe join/leave APIs
- Basic placement API with deterministic behavior

**Exit Criteria**:
- Compile-time and integration contract checks pass with runtime

**Risks**:
- Unclear ownership boundaries with existing runtime logic

### Phase 2: Runtime Hardening

**Goal**: Failover detection, idempotent rescheduling, durable control-plane state, observability hooks.

**Deliverables**:
- Failover detection and idempotent rescheduling
- Durable control-plane state integration with storage
- Observability hooks for cluster lifecycle events

**Exit Criteria**:
- Failure-injection tests show deterministic recovery

### Phase 3: Scale and Performance

**Goal**: Scheduler optimization; fairness guarantees; benchmark.

**Deliverables**:
- Optimized scheduler for high-cardinality workflow distributions
- Strategy tuning and fairness guarantees
- Benchmark cluster decision latency under load

**Exit Criteria**:
- Placement latency and throughput within target SLO budgets

### Phase 4: Ecosystem and DX

**Goal**: Operator APIs/CLI; autoscaling policy framework; runbooks.

**Deliverables**:
- Operator APIs/CLI for rebalance and maintenance
- Autoscaling policy framework and staged rollout
- Runbooks for incident triage and cluster operations

**Exit Criteria**:
- Production readiness checklist validated in staging

## Inter-Crate Dependencies

- **Depends on**: `nebula-core`, `nebula-storage` (durable state), `nebula-system`, `nebula-telemetry`
- **Depended by**: `nebula-runtime` (multi-runtime coordination), `nebula-api` (cluster management endpoints)

## Verification

- [ ] `cargo check -p nebula-cluster`
- [ ] `cargo test -p nebula-cluster`
- [ ] `cargo clippy -p nebula-cluster -- -D warnings`
- [ ] `cargo doc --no-deps -p nebula-cluster`
