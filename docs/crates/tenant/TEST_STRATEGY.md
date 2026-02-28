# Test Strategy

## Test Pyramid

- unit:
  - tenant identity normalization, policy validation, quota arithmetic.
- integration:
  - ingress->tenant->runtime/resource/storage contract flows.
- contract:
  - cross-crate compatibility tests for tenant context and denial semantics.
- end-to-end:
  - tenant-isolated workflow execution scenarios.

## Critical Invariants

- unknown or invalid tenant context is always denied.
- tenant context cannot be overwritten by untrusted caller metadata.
- quota accounting is race-safe and monotonic.
- cross-tenant access is rejected in resource/storage/credential paths.

## Scenario Matrix

- happy path:
  - valid tenant context and within-limit operations.
- retry path:
  - transient policy backend errors recovered with retry.
- cancellation path:
  - aborted tenant lookups do not commit partial state.
- timeout path:
  - backend timeout leads to deterministic safe decision.
- upgrade/migration path:
  - strategy and policy schema changes remain compatible via migration plan.

## Tooling

- property testing:
  - quota and identity invariants under random inputs.
- fuzzing:
  - parser/validator fuzzing for tenant IDs and policies.
- benchmarks:
  - decision latency and quota contention microbenchmarks.
- CI quality gates:
  - cross-crate tenant contract suite + feature/default matrix tests.

## Exit Criteria

- coverage goals:
  - full coverage of identity/isolation/quota critical paths.
- flaky test budget:
  - zero flakiness in security-critical tenant checks.
- performance regression thresholds:
  - no significant regression in decision latency under representative load.
