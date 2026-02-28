# Test Strategy

## Test Pyramid

- unit:
  - sandbox context guard checks (cancellation/capability matcher once added).
- integration:
  - runtime-to-sandbox execution flows and error propagation.
- contract:
  - `SandboxRunner` behavior consistency across backends.
- end-to-end:
  - workflow execution scenarios with sandbox policy selection.

## Critical Invariants

- sandbox always checks cancellation before expensive execution work.
- backend never bypasses declared policy constraints.
- deterministic mapping of sandbox violations to runtime-visible errors.
- backend selection policy is explicit and auditable.

## Scenario Matrix

- happy path:
  - successful in-process sandbox execution.
- retry path:
  - transient backend failures surfaced for resilience policy.
- cancellation path:
  - cancelled context fails fast.
- timeout path:
  - execution limits trigger deterministic error path.
- upgrade/migration path:
  - backend parity maintained during rollout of new driver.

## Tooling

- property testing:
  - capability matcher behavior for random capability sets.
- fuzzing:
  - metadata/capability parser and boundary serialization fuzzing.
- benchmarks:
  - sandbox overhead benchmark per backend.
- CI quality gates:
  - contract tests in `ports` + driver integration tests + backend parity suite.

## Exit Criteria

- coverage goals:
  - all security-critical sandbox invariants tested.
- flaky test budget:
  - zero flaky tests on cancellation/violation critical paths.
- performance regression thresholds:
  - sandbox boundary overhead regressions within accepted limits.
