# Test Strategy

## Test Pyramid

- unit:
  - loaders, source parsing, merge utilities, path traversal, error mapping.
- integration:
  - builder + loader + validator + watcher combinations.
- contract:
  - precedence semantics and typed access compatibility.
- end-to-end:
  - consumer crate fixture tests (runtime/resource/credential initialization flows).

## Critical Invariants

- precedence order is deterministic.
- invalid config never becomes active.
- reload preserves last-valid state on failure.
- typed `get<T>` behavior is stable across versions.

## Scenario Matrix

- happy path:
  - defaults + file + env merge and typed retrieval.
- retry path:
  - transient source failure with caller retry.
- cancellation path:
  - auto-reload task cancellation on config drop/stop.
- timeout path:
  - slow source/watcher callbacks bounded by runtime policies.
- upgrade/migration path:
  - behavior fixtures across versions for precedence/path semantics.

## Tooling

- property testing:
  - merge idempotence and precedence invariants.
- fuzzing:
  - parser/path traversal fuzzing for format handlers.
- benchmarks:
  - parse/merge/get hot paths.
- CI quality gates:
  - fmt, clippy, tests, selected benchmark regression checks.

## Exit Criteria

- coverage goals:
  - all source types and core merge/path APIs covered.
- flaky test budget:
  - zero for precedence and validation contract tests.
- performance regression thresholds:
  - measurable budgets for load/reload/get paths enforced.
