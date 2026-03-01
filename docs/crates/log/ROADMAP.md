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

## Phase 2: Runtime Hardening (Completed)

- **Completed:**
  - Backpressure/drop policy docs for non-blocking file mode
  - Hook execution budget proposal (P-001): v1 budget diagnostics implemented; async offload deferred and tracked
  - Stronger shutdown ordering guarantees for hooks (LIFO shutdown + shutdown-time dispatch quiesce)
- **Risks:** Hook policy changes may affect custom hooks
- **Exit criteria:** Documented failure modes; no hook deadlocks on shutdown

## Phase 3: Scale and Performance (Completed)

- **Completed:**
  - Benchmark hot paths: event emission, context propagation, timing macros (expanded criterion bench coverage)
  - Reduced per-event allocations in observability hooks (visitor payload API + logging/metrics hook fast paths)
  - Criterion benchmarks in CI with regression thresholds (for `nebula-log` hot-path suite)
- **Risks:** Optimization may complicate code
- **Exit criteria:** Benchmarks stable; no regressions beyond threshold

## Phase 4: Format and Writer Gaps (Next)

- **Deliverables:**
  - Implement real `Format::Logfmt` output (currently falls through to `Compact`)
  - Wire OTLP exporter into `otel::build_layer` and integrate into builder pipeline (`build_layer` exists but is never called; no exporter attached)
  - Implement `FieldsLayer` properly or remove the no-op placeholder (global fields currently work via root span only)
  - Support custom time format in `make_timer` (parameter is currently ignored)
  - Make `Config::test()` available to consumers (currently `#[cfg(test)]` only)
  - Configurable size rolling retention (currently keeps only 1 rotated backup `.1`)
  - Implement graceful OTLP shutdown (`otel::shutdown()` is currently a no-op)
- **Risks:** Logfmt and OTLP changes touch format/telemetry layers; may require type-system workarounds
- **Exit criteria:** All declared `Format` variants produce distinct output; OTLP pipeline exports spans; size rolling retention configurable

## Phase 5: Ecosystem and DX

- **Deliverables:**
  - Unify trace/log correlation fields across API/engine/runtime
  - Stable semantic field naming convention
  - Default OTLP resource attributes and validation
  - Typed event names (P-002) or context IDs from core (P-003) migration
  - P-001 v2: async hook offload with queue accounting and drop strategy
- **Risks:** Breaking changes for custom hooks/contexts
- **Exit criteria:** Correlation fields documented; migration guide for breaking changes

## Phase 6: Toolchain Baseline

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
