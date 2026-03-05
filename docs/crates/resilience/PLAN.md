# Implementation Plan: nebula-resilience

**Crate**: `nebula-resilience` | **Path**: `crates/resilience` | **Roadmap**: [ROADMAP.md](ROADMAP.md)

## Summary

nebula-resilience provides circuit breaker, retry, rate limiting, and timeout patterns for the Nebula workflow engine. The roadmap is focused on correctness under load and operational clarity. Phase 1 API contract consolidation is complete; subsequent phases target performance, policy hardening, reliability, and toolchain compatibility.

## Technical Context

**Language/Edition**: Rust 2024 (MSRV 1.93)
**Async Runtime**: Tokio (sync, time)
**Key Dependencies**: async-trait, tokio, tokio-util, futures, serde, serde_json, thiserror, parking_lot, dashmap, governor, fastrand, tracing, humantime-serde, nebula-config, nebula-core, nebula-log
**Testing**: `cargo test -p nebula-resilience`

## Current Status

| Phase | Status | Notes |
|-------|--------|-------|
| Phase 1: API Contract Consolidation | ✅ Done | Typed/untyped APIs reconciled, examples aligned, stability boundaries clarified |
| Phase 2: Performance and Scalability | ⬜ Planned | Benchmark hot paths, optimize contention, profile layer composition |
| Phase 3: Policy and Config Hardening | ⬜ Planned | Policy validation, migration/versioning, dynamic config |
| Phase 4: Reliability and Safety | ⬜ Planned | Fault injection, observability in failure storms, fail-open/closed defaults |
| Phase 5: Toolchain and Compatibility | ⬜ Planned | Rust 1.93+ migration, compatibility guarantees |
| Phase 6: Pattern Coverage Expansion | ✅ Done | Governor/timeout benchmark baselines plus fallback/hedge reliability coverage and consolidated operational guidance completed |
| Phase 7: Pattern Hardening Wave | ✅ Done | Bulkhead/retry/fallback/timeout hardening guidance consolidated and performance budgets updated |

## Phase Details

### Phase 1: API Contract Consolidation (Completed)

**Goal**: Reconcile typed/untyped manager APIs and establish clear adoption path.

**Deliverables**:
- Reconcile typed/untyped manager APIs and document preferred adoption path
- Align examples with current production guidance
- Clarify stability boundaries for advanced type-system APIs

**Exit Criteria**:
- Unified API documented; examples aligned; stability boundaries clear

**Risks**:
- (none noted)

### Phase 2: Performance and Scalability

**Goal**: Optimize resilience patterns for high-cardinality, high-throughput scenarios.

**Deliverables**:
- Benchmark manager hot paths with high service cardinality
- Optimize circuit/rate limiter contention scenarios
- Profile layer composition overhead in deep chains

**Exit Criteria**:
- Benchmarks within budget; contention scenarios optimized; layer overhead profiled

**Risks**:
- Contention under high concurrency may require architectural changes

### Phase 3: Policy and Config Hardening

**Goal**: Strengthen policy validation and dynamic configuration behavior.

**Deliverables**:
- Strengthen policy validation for conflicting combinations
- Add policy migration/versioning strategy
- Tighten dynamic config behavior and reload semantics

**Exit Criteria**:
- No conflicting policies accepted; migration strategy documented; reload behavior deterministic

**Risks**:
- Conflicting policy combinations may be hard to detect exhaustively

### Phase 4: Reliability and Safety

**Goal**: Expand fault-injection testing and formalize failure defaults.

**Deliverables**:
- Expand fault-injection tests for retry+breaker+timeout interplay
- Validate observability behavior in failure storms
- Formalize fail-open/fail-closed defaults per pattern

**Exit Criteria**:
- Fault-injection tests cover all pattern combinations; defaults documented

**Risks**:
- Complex interplay between patterns may have edge cases

### Phase 5: Toolchain and Compatibility

**Goal**: Ensure forward compatibility and define serialization guarantees.

**Deliverables**:
- Prepare controlled migration to Rust 1.93+
- Define compatibility guarantees for policy serialization and metrics schema

**Exit Criteria**:
- Compatibility guarantees documented; no MSRV regression

**Risks**:
- Breaking changes in Rust edition or dependency updates

### Phase 6: Pattern Coverage Expansion

**Goal**: Close performance and correctness gaps for patterns not deeply covered in earlier phases.

**Deliverables**:
- Benchmark/tune `GovernorRateLimiter` under high concurrency
- Benchmark timeout wrapper overhead and cancellation-path latency
- Add fallback and hedge fault-injection/stress scenarios
- Document per-pattern operational limits and defaults for governor/timeout/fallback/hedge

**Exit Criteria**:
- Governor/timeout/fallback/hedge have benchmark + reliability coverage and documented guidance

**Risks**:
- Hedge and fallback semantics may introduce high-variance timing in CI unless scenarios are carefully bounded

### Phase 7: Pattern Hardening Wave

**Goal**: Improve bulkhead/retry/fallback/timeout operational robustness beyond Phase 6 baseline.

**Deliverables**:
- Add dedicated bulkhead benchmark target with fast-path/contention/queue-timeout coverage
- Add bulkhead fairness/starvation stress coverage
- Add retry storm-guard guidance and adaptive jitter tuning validation
- Add fallback staleness-policy coverage and bounded-chain guidance
- Add timeout short-deadline platform/runtime guidance and limits

**Exit Criteria**:
- Bulkhead/retry/fallback/timeout have hardened stress coverage plus consolidated operator guidance

**Risks**:
- Fairness and timer-granularity behavior may vary by platform and require tolerance-based assertions

## Dependencies

| Depends On | Why |
|-----------|-----|
| nebula-config | Reads resilience policies from configuration |
| nebula-core | Core identifiers and scope system |
| nebula-log | Structured logging for resilience events |

| Depended By | Why |
|------------|-----|
| (downstream crates) | Provides resilience patterns to workflow execution |

## Verification

- [ ] `cargo check -p nebula-resilience`
- [ ] `cargo test -p nebula-resilience`
- [ ] `cargo clippy -p nebula-resilience -- -D warnings`
- [ ] `cargo doc --no-deps -p nebula-resilience`
