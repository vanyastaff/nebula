# Implementation Plan: nebula-memory

**Crate**: `nebula-memory` | **Path**: `crates/memory` | **Roadmap**: [ROADMAP.md](ROADMAP.md)

## Summary

`nebula-memory` provides high-performance memory management for Nebula, including arena allocators, object pools, caches, memory budgets, and pressure detection. It supports both `std` and `no_std` environments with extensive feature flags. Current focus is on contract/safety baseline and runtime hardening.

## Technical Context

**Language/Edition**: Rust 2024 (MSRV 1.93)
**Async Runtime**: Tokio (optional, behind `async` feature)
**Key Dependencies**: `nebula-core`, `nebula-system`, `nebula-log` (optional), `parking_lot`, `crossbeam-queue`, `hashbrown`, `dashmap`, `heapless`, `spin`, `thiserror`
**Testing**: `cargo test -p nebula-memory`

## Current Status

| Phase | Status | Notes |
|-------|--------|-------|
| Phase 1: Contract and Safety Baseline | Planned | Docs, API surface, safety invariants |
| Phase 2: Runtime Hardening | Planned | Concurrency/stress tests, pressure handling |
| Phase 3: Scale and Performance | Planned | Benchmarks, sizing profiles, feature overhead |
| Phase 4: Ecosystem and DX | Planned | Integration patterns, config bootstrap |

## Phase Details

### Phase 1: Contract and Safety Baseline

**Goal**: Finalize SPEC-aligned docs and cross-crate memory contracts.

**Deliverables**:
- Finalize SPEC-aligned docs and cross-crate memory contracts
- Define stable API subset and explicit unstable surface
- Document safety invariants for unsafe-heavy internals

**Exit Criteria**:
- Docs map to real code and are review-ready for runtime/action teams

**Risks**:
- Hidden assumptions in consumer crates

### Phase 2: Runtime Hardening

**Goal**: Validate concurrent behavior and pressure handling under spikes.

**Deliverables**:
- Expand concurrency/stress tests for shared pools/caches
- Validate pressure handling and budget enforcement under spikes
- Tighten error-path behavior consistency

**Exit Criteria**:
- Deterministic behavior in stress scenarios and no critical flaky tests

**Risks**:
- Contention regressions and flaky concurrency tests

### Phase 3: Scale and Performance

**Goal**: Benchmark-guided tuning with published sizing profiles.

**Deliverables**:
- Benchmark-guided tuning across allocator/pool/cache modes
- Published sizing profiles for common workflow load classes
- Optimize feature-on overhead for monitoring/stats paths

**Exit Criteria**:
- Measurable p95/p99 improvement on representative scenarios

**Risks**:
- Over-optimization for synthetic benchmarks

### Phase 4: Ecosystem and DX

**Goal**: Reference integration patterns and migration tooling.

**Deliverables**:
- Reference integration patterns for runtime and action crates
- Optional unified config bootstrap if proposal accepted
- Improve migration tooling/checklists for feature and API evolution

**Exit Criteria**:
- At least one fully documented end-to-end integration flow

**Risks**:
- Breaking consumer assumptions during API cleanup

## Dependencies

| Depends On | Why |
|-----------|-----|
| nebula-core | Core types and identifiers |
| nebula-system | Platform utilities, pressure detection |
| nebula-log | Structured logging (optional, behind `logging` feature) |

| Depended By | Why |
|------------|-----|
| nebula-expression | Cache subsystem for expression evaluation |
| nebula-engine | Memory pools and budgets for execution |
| nebula-runtime | Memory management during workflow runs |

## Verification

- [ ] `cargo check -p nebula-memory`
- [ ] `cargo test -p nebula-memory`
- [ ] `cargo clippy -p nebula-memory -- -D warnings`
- [ ] `cargo doc --no-deps -p nebula-memory`
