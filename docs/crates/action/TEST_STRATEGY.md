# Test Strategy

## Test Pyramid

- unit:
  - enum behavior helpers, metadata builders, component declarations.
- integration:
  - runtime adapter compatibility tests around result/output/error contracts.
- contract:
  - serialization compatibility for `ActionResult`, `ActionOutput`, `ActionError`.
- end-to-end:
  - workflow execution fixtures validating flow-control semantics.

## Critical Invariants

- control/data separation:
  - `ActionResult` semantics must remain distinct from `ActionOutput` payload form.
- retry semantics:
  - retry intent from result/error must map deterministically to resilience policy.
- compatibility:
  - metadata and port changes must obey interface version rules.

## Scenario Matrix

- happy path:
  - `Success` and default flow/output behavior.
- retry path:
  - `ActionResult::Retry` and `ActionError::Retryable` mapping.
- cancellation path:
  - `ActionError::Cancelled` handling.
- timeout path:
  - deferred resolution timeout behavior at runtime boundaries.
- upgrade/migration path:
  - old/new serialization compatibility fixtures.

## Tooling

- property testing:
  - invariant checks across result/output transformations.
- fuzzing:
  - serde roundtrip fuzzing for contract enums.
- benchmarks:
  - optional micro-bench for protocol transformations.
- CI quality gates:
  - lint, test, contract fixtures with runtime/engine integration.

## Exit Criteria

- coverage goals:
  - all public protocol types have behavior and serialization tests.
- flaky test budget:
  - zero for contract tests.
- performance regression thresholds:
  - no measurable regression in core mapping/serialization hot paths.
