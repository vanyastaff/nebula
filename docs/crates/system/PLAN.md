# Implementation Plan: nebula-system

**Crate**: `nebula-system` | **Path**: `crates/system` | **Roadmap**: [ROADMAP.md](ROADMAP.md)

## Summary

nebula-system provides cross-platform system information and utilities (CPU, memory, disk, network, process) for the Nebula ecosystem. Phase 1 contract/safety baseline is complete; future phases focus on runtime hardening with nebula-memory integration, performance optimization, and metrics/observability.

## Technical Context

**Language/Edition**: Rust 2024 (MSRV 1.93)
**Async Runtime**: Tokio (optional, via `async` feature)
**Key Dependencies**: sysinfo, once_cell, parking_lot, thiserror, libc (optional), region (optional), winapi, serde (optional), tokio (optional)
**Testing**: `cargo test -p nebula-system`

## Current Status

| Phase | Status | Notes |
|-------|--------|-------|
| Phase 1: Contract and Safety Baseline | ✅ Done | Stable API, documented errors, CI on Linux/macOS/Windows |
| Phase 2: Runtime Hardening | ⬜ Planned | Pressure-based integration with nebula-memory, metrics export |
| Phase 3: Scale and Performance | ⬜ Planned | Optimized process list, configurable refresh, NUMA awareness |
| Phase 4: Ecosystem and DX | ⬜ Planned | OpenTelemetry/Prometheus, health checks, async wrappers |

## Phase Details

### Phase 1: Contract and Safety Baseline (Completed)

**Goal**: Establish stable public API with documented error semantics and cross-platform CI.

**Deliverables**:
- Stable public API
- Documented error semantics
- CI on Linux/macOS/Windows

**Exit Criteria**:
- `cargo test --workspace` passes
- Docs complete
- No known safety issues

**Risks**:
- sysinfo API drift
- Platform-specific edge cases

### Phase 2: Runtime Hardening

**Goal**: Integrate with nebula-memory for pressure-based decisions and add optional metrics.

**Deliverables**:
- Pressure-based integration with nebula-memory
- Optional metrics export

**Exit Criteria**:
- nebula-memory integration tests pass
- Benchmarks within budget

**Risks**:
- Performance under load
- Cache invalidation strategy

### Phase 3: Scale and Performance

**Goal**: Optimize system info collection for scale.

**Deliverables**:
- Optimized process list
- Configurable refresh intervals
- NUMA awareness

**Exit Criteria**:
- Process list <5ms for 100 processes
- Documented platform limits

**Risks**:
- Platform differences
- Maintenance burden

### Phase 4: Ecosystem and DX

**Goal**: Add observability integrations and developer experience improvements.

**Deliverables**:
- OpenTelemetry/Prometheus metrics
- Health check helpers
- Async wrappers (optional)

**Exit Criteria**:
- Metrics feature documented
- Examples for common workflows

**Risks**:
- Scope creep
- Dependency bloat

## Dependencies

| Depends On | Why |
|-----------|-----|
| (none) | Leaf crate with no internal dependencies |

| Depended By | Why |
|------------|-----|
| nebula-memory | Pressure detection data feeds memory management decisions |

## Verification

- [ ] `cargo check -p nebula-system`
- [ ] `cargo test -p nebula-system`
- [ ] `cargo clippy -p nebula-system -- -D warnings`
- [ ] `cargo doc --no-deps -p nebula-system`
