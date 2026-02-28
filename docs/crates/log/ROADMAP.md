# Roadmap

`nebula-log` roadmap is focused on throughput, reliability, and operational safety.

## Phase 1: Production Hardening

- implement real fanout for `WriterConfig::Multi`
- implement `Rolling::Size(u64)`
- add backpressure/drop policy docs for non-blocking file mode
- formalize env var contract and config precedence

## Phase 2: Performance and Allocation Control

- benchmark hot paths:
  - event emission
  - context propagation
  - timing macros in tight loops
- reduce per-event allocations in observability hooks
- add criterion benchmarks to CI threshold checks

## Phase 3: Telemetry and Correlation

- unify trace/log correlation fields across API/engine/runtime
- add stable semantic field naming convention
- provide default OTLP resource attributes and stronger validation

## Phase 4: Safety and Isolation

- tighten hook execution limits (timeouts/budget/circuit-breaker for external hooks)
- add stronger guarantees for hook shutdown ordering
- document and test failure modes under panic storms

## Phase 5: Toolchain Baseline

- workspace baseline today: Rust `1.92`
- prepare controlled migration to Rust `1.93+` with:
  - CI matrix updates
  - clippy/rustdoc policy revalidation
  - performance regression check
