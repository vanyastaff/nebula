# Roadmap

## Phase 1: Contract and Safety Baseline

- **Deliverables:**
  - Implement real fanout for `WriterConfig::Multi` with failure policy
  - Implement `Rolling::Size(u64)`
  - Formalize env var contract (`NEBULA_LOG`, `RUST_LOG`) and config precedence
  - Add config schema versioning and snapshot tests
- **Risks:** Multi writer behavior change may affect existing deployments
- **Exit criteria:** All writers in Multi receive events; Size rolling works; snapshot tests pass

## Phase 2: Runtime Hardening

- **Deliverables:**
  - Backpressure/drop policy docs for non-blocking file mode
  - Hook execution budget proposal (P-001) implementation or deferral
  - Stronger shutdown ordering guarantees for hooks
- **Risks:** Hook policy changes may affect custom hooks
- **Exit criteria:** Documented failure modes; no hook deadlocks on shutdown

## Phase 3: Scale and Performance

- **Deliverables:**
  - Benchmark hot paths: event emission, context propagation, timing macros
  - Reduce per-event allocations in observability hooks
  - Criterion benchmarks in CI with regression thresholds
- **Risks:** Optimization may complicate code
- **Exit criteria:** Benchmarks stable; no regressions beyond threshold

## Phase 4: Ecosystem and DX

- **Deliverables:**
  - Unify trace/log correlation fields across API/engine/runtime
  - Stable semantic field naming convention
  - Default OTLP resource attributes and validation
  - Typed event names (P-002) or context IDs from core (P-003) migration
- **Risks:** Breaking changes for custom hooks/contexts
- **Exit criteria:** Correlation fields documented; migration guide for breaking changes

## Phase 5: Toolchain Baseline

- **Deliverables:**
  - Workspace baseline: Rust 1.92 (MSRV)
  - Controlled migration to Rust 1.93+ with CI matrix, clippy/rustdoc revalidation
- **Risks:** MSRV bump may affect downstream
- **Exit criteria:** CI passes on new MSRV; no performance regression

## Metrics of Readiness

- **Correctness:** All tests pass; snapshot tests for config
- **Latency:** Hot path benchmarks within threshold
- **Throughput:** Event emission and hook dispatch measured
- **Stability:** No flaky tests; panic isolation verified
- **Operability:** Env/config contract documented; runbook in RELIABILITY.md
