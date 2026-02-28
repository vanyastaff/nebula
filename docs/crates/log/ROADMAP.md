# Roadmap

## Phase 1: Contract and Safety Baseline (Completed)

- **Completed:**
  - Real fanout for `WriterConfig::Multi` with failure policy (`FailFast`, `BestEffort`, `PrimaryWithFallback`)
  - `Rolling::Size(u64)` implementation
  - Env/config precedence behavior with explicit compatibility tests
  - Config schema version guard (`schema_version`) and compatibility checks
  - Stable config schema snapshot contract tests with versioned fixtures
  - Formal env var contract table published in API docs
- **Risks:** Snapshot governance drift if contract fixtures are not versioned per release
- **Exit criteria:** Snapshot contract tests pass and docs specify precedence unambiguously

## Phase 2: Runtime Hardening (Next)

- **Deliverables:**
  - Backpressure/drop policy docs for non-blocking file mode (completed)
  - Hook execution budget proposal (P-001): v1 budget diagnostics implemented; async offload deferred and tracked
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

- **Correctness:** All tests pass; schema snapshot contract tests for config
- **Latency:** Hot path benchmarks within threshold
- **Throughput:** Event emission and hook dispatch measured
- **Stability:** No flaky tests; panic isolation verified
- **Operability:** Env/config contract documented; runbook in RELIABILITY.md
