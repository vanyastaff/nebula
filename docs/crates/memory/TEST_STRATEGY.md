# Test Strategy

## Test Pyramid

- unit:
  - allocator/pool/cache/budget invariants and error classification.
- integration:
  - cross-module behavior under realistic concurrency and pressure.
- contract:
  - compatibility tests for runtime/action/resilience integration contracts.
- end-to-end:
  - workflow-like scenarios in higher-level runtime test suites.

## Critical Invariants

- unsafe-backed implementations preserve alignment/layout and ownership guarantees.
- pooled objects are returned safely and never exceed configured capacity rules.
- budget accounting remains consistent under concurrent request/release.
- retryability semantics stay stable for consumer policy logic.

## Scenario Matrix

- happy path:
  - successful alloc/reuse/release across primary modules.
- retry path:
  - temporary exhaustion recovers after release/cleanup.
- cancellation path:
  - async-enabled operations obey cancellation boundaries.
- timeout path:
  - bounded waits/exhaustion behavior in configured modes.
- upgrade/migration path:
  - feature/default changes preserve documented compatibility expectations.

## Tooling

- property testing:
  - randomized size/alignment and state transitions.
- fuzzing:
  - edge-case stress for unsafe-adjacent logic and config parsing.
- benchmarks:
  - criterion benches for allocator throughput and realistic workload scenarios.
- CI quality gates:
  - default + full feature tests, plus selected feature subsets.

## Exit Criteria

- coverage goals:
  - critical safety and lifecycle paths comprehensively tested.
- flaky test budget:
  - zero tolerated flaky tests in core concurrency/safety suites.
- performance regression thresholds:
  - no material regression in key benchmark tracks relative to baseline.
