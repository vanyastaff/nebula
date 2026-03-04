# Implementation Plan: nebula-worker (queue-memory driver)

**Crate**: `nebula-worker` | **Path**: `crates/worker` | **ROADMAP**: [ROADMAP.md](ROADMAP.md)

## Summary

The worker crate implements the in-process work queue driver — claiming tasks via lease, executing them through sandbox/runtime, and finalizing (ack/nack) with retry semantics. It also provides the worker operator interface: drain, health/readiness, graceful restart. Current focus is Phase 1: establishing the queue lease contract and sandbox integration.

## Technical Context

**Language/Edition**: Rust 2024 (MSRV 1.93)
**Async Runtime**: Tokio
**Key Dependencies**: `nebula-core`, `nebula-execution`, `nebula-system`, `nebula-telemetry`, `nebula-resilience`
**Testing**: `cargo test -p nebula-worker`

## Current Status

| Phase | Status | Summary |
|-------|--------|---------|
| Phase 1: Contract and Safety Baseline | ⬜ Planned | Worker config, queue lease contract, sandbox integration |
| Phase 2: Runtime Hardening | ⬜ Planned | Retry/backoff with resilience policies, failure taxonomy, health |
| Phase 3: Scale and Performance | ⬜ Planned | Adaptive concurrency, autoscaling signals, hot-path optimization |
| Phase 4: Ecosystem and DX | ⬜ Planned | Operator handbook, plugin compatibility, telemetry dashboards |

## Phase Details

### Phase 1: Contract and Safety Baseline

**Goal**: Worker config/state model; queue lease contract; sandbox integration; core metrics skeleton.

**Deliverables**:
- Worker config and state model
- Queue lease contract: claim/heartbeat/ack/nack
- Basic sandbox integration and timeout/cancel flow
- Core metrics/logging/tracing skeleton

**Exit Criteria**:
- Contract tests green for lease lifecycle and finalization idempotency
- Drain behavior validated in integration tests

**Risks**:
- Contract mismatch with runtime/queue
- Missing idempotency in finalization flow

### Phase 2: Runtime Hardening

**Goal**: Robust retry/backoff with resilience policies; structured failure taxonomy; health/readiness.

**Deliverables**:
- Retry/backoff using `nebula-resilience` policies
- Structured failure taxonomy and dead-letter strategy
- Health/readiness + graceful rolling restart behavior

**Exit Criteria**:
- Chaos tests pass for queue/runtime transient failures
- No task loss in restart simulations

**Risks**:
- Retry storms during partial outage
- False-positive unhealthy signals

### Phase 3: Scale and Performance

**Goal**: Adaptive concurrency, autoscaling signals, hot-path optimization.

**Deliverables**:
- Adaptive concurrency and queue backpressure
- Autoscaling signals: saturation, lease lag, completion latency
- Hot-path optimization for execution overhead

**Exit Criteria**:
- Target throughput and p95 latency met under stress profile
- Stable scaling behavior in load tests

### Phase 4: Ecosystem and DX

**Goal**: Operator handbook, plugin/action compatibility matrix, telemetry dashboards.

**Deliverables**:
- Worker operator handbook and runbooks
- Plugin/action compatibility matrix
- Telemetry dashboards + SLO alerts

**Exit Criteria**:
- On-call runbook drill completed
- Contract versioning and migration policy exercised once

## Inter-Crate Dependencies

- **Depends on**: `nebula-execution` (task state), `nebula-system` (pressure), `nebula-resilience` (retry/circuit-breaker), `nebula-telemetry`
- **Depended by**: `nebula-engine` (task dispatch), `nebula-runtime` (execution delegation)

## Verification

- [ ] `cargo check -p nebula-worker`
- [ ] `cargo test -p nebula-worker`
- [ ] `cargo clippy -p nebula-worker -- -D warnings`
- [ ] `cargo doc --no-deps -p nebula-worker`
