# Test Strategy

## Test Pyramid

- unit:
  - membership state transitions, scheduler decisions, policy validation.
- integration:
  - runtime/worker/storage interactions for placement and failover.
- contract:
  - API/CLI control command safety and semantics.
- end-to-end:
  - multi-node execution, failover, and rebalance scenarios.

## Critical Invariants

- unique workflow ownership under normal and degraded conditions.
- idempotent failover/rebalance operations.
- no scheduling without validated membership and state freshness.
- safe rejection of unauthorized/invalid control operations.

## Scenario Matrix

- happy path:
  - stable membership and balanced scheduling.
- retry path:
  - transient backend/network failures with bounded retries.
- cancellation path:
  - aborted control operations leave consistent state.
- timeout path:
  - slow consensus/storage paths handled without corruption.
- upgrade/migration path:
  - compatibility during strategy and state model evolution.

## Tooling

- property testing:
  - state-machine and placement invariants.
- fuzzing:
  - control command parser and transition requests.
- benchmarks:
  - scheduler latency and control-plane throughput.
- CI quality gates:
  - distributed integration tests with failure-injection scenarios.

## Exit Criteria

- coverage goals:
  - full coverage for safety-critical cluster transitions.
- flaky test budget:
  - zero flaky tests in membership/failover invariants.
- performance regression thresholds:
  - no significant regressions in placement/failover latency under baseline load.
