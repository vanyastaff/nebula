# Test Strategy

## Test Pyramid

- unit:
  - scope containment, error helpers, config validation, dependency graph checks.
- integration:
  - manager/pool/hook/health/quarantine/autoscale interaction tests.
- contract:
  - resource-provider behavior for runtime/action integration contracts.
- end-to-end:
  - workflow execution path using realistic resource adapters (planned expansion).

## Critical Invariants

- acquire never bypasses quarantine/health/scope checks.
- guard drop never leaks semaphore permits or pooled instances.
- failed register/reload never leaves inconsistent dependency graph state.
- cross-tenant scope access is always denied unless explicitly compatible.

## Scenario Matrix

- happy path:
  - register, acquire, use, release, and shutdown.
- retry path:
  - retryable errors (`PoolExhausted`, timeout, temporary unavailable) mapped correctly.
- cancellation path:
  - acquire aborts quickly when context cancellation token fires.
- timeout path:
  - bounded wait behavior under exhausted pool.
- upgrade/migration path:
  - feature-flag combinations and config evolution compatibility tests.

## Tooling

- property testing:
  - scope and lifecycle properties (`proptest`).
- fuzzing:
  - planned for config/scope parsing and edge-case error handling.
- benchmarks:
  - `benches/pool_throughput.rs` for acquire/release throughput.
- CI quality gates:
  - default + full feature test matrix.
  - benchmark regression checks for major releases.

## Exit Criteria

- coverage goals:
  - critical lifecycle and isolation paths fully covered by integration tests.
- flaky test budget:
  - zero flaky tests in manager/pool critical path.
- performance regression thresholds:
  - no statistically significant degradation in acquire latency/throughput benchmarks.
