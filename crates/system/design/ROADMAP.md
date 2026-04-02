# Roadmap

## Phase 1: Contract and Safety Baseline

- **Deliverables:** Stable public API; documented error semantics; CI on Linux/macOS/Windows
- **Risks:** sysinfo API drift; platform-specific edge cases
- **Exit criteria:** `cargo test --workspace` passes; docs complete; no known safety issues

## Phase 2: Runtime Hardening

- **Deliverables:** Pressure-based integration with nebula-memory; optional metrics export
- **Risks:** Performance under load; cache invalidation strategy
- **Exit criteria:** nebula-memory integration tests pass; benchmarks within budget

## Phase 3: Scale and Performance

- **Deliverables:** Optimized process list; configurable refresh intervals; NUMA awareness
- **Risks:** Platform differences; maintenance burden
- **Exit criteria:** Process list <5ms for 100 processes; documented platform limits

## Phase 4: Ecosystem and DX

- **Deliverables:** OpenTelemetry/Prometheus metrics; health check helpers; async wrappers (optional)
- **Risks:** Scope creep; dependency bloat
- **Exit criteria:** Metrics feature documented; examples for common workflows

## Metrics of Readiness

- **Correctness:** All unit/integration tests pass; no UB in unsafe blocks
- **Latency:** `SystemInfo::get()` <1µs (cached); `memory::current()` <1ms
- **Throughput:** N/A (read-only)
- **Stability:** No panics in normal operation; graceful degradation on permission errors
- **Operability:** Clear error messages; platform-specific notes in docs
