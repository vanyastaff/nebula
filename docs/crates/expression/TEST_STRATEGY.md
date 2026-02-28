# Test Strategy

## Test Pyramid

- unit:
  - lexer/parser/evaluator operator and function semantics.
- integration:
  - engine/context/template behavior and cache-enabled flows.
- contract:
  - runtime/action/parameter integration contracts for context and error semantics.
- end-to-end:
  - workflow-like expression/template scenarios in higher-level runtime suites.

## Critical Invariants

- expression semantics are deterministic for same input/context.
- cache on/off paths produce identical results.
- safety guards (regex/recursion/template-limits) reliably block unsafe patterns.
- template rendering preserves non-expression text exactly.

## Scenario Matrix

- happy path:
  - standard expression and template resolution.
- retry path:
  - transient internal error handling by caller policy.
- cancellation path:
  - caller-side cancellation boundaries around evaluation path.
- timeout path:
  - long/complex expression containment via guards and runtime controls.
- upgrade/migration path:
  - compatibility suite for function/grammar evolution.

## Tooling

- property testing:
  - parser/evaluator equivalence and value invariants.
- fuzzing:
  - parser/tokenizer and template parser fuzz campaigns.
- benchmarks:
  - `benches/baseline.rs` and integration benchmark suites.
- CI quality gates:
  - unit + integration + benchmark regression checks; feature-matrix runs.

## Exit Criteria

- coverage goals:
  - critical parser/evaluator/template paths comprehensively tested.
- flaky test budget:
  - zero flaky tests in expression correctness/security suites.
- performance regression thresholds:
  - no significant regression in baseline benchmark tracks.
