# Test Strategy

## Test Pyramid

- unit:
  - validator correctness per module (`length`, `range`, `pattern`, etc.).
- integration:
  - combined validator pipelines and contextual validation scenarios.
- contract:
  - error code and field-path compatibility tests for consumer crates.
- end-to-end:
  - API/workflow/plugin fixture validation against real consumer flows.

## Critical Invariants

- combinator semantics:
  - `and` short-circuits on first failure, `or` returns success when any branch succeeds.
- error integrity:
  - codes/messages/field paths and nested error counts are deterministic.
- type safety:
  - invalid validator/input combinations fail at compile time.

## Scenario Matrix

- happy path:
  - valid inputs across all built-in validator families.
- retry path:
  - not applicable in validator core; ensure callers do not misclassify deterministic failures.
- cancellation path:
  - not applicable to pure sync validation calls.
- timeout path:
  - stress tests for expensive patterns and nested payloads.
- upgrade/migration path:
  - compatibility fixtures across versions for error schema.

## Tooling

- property testing:
  - existing `proptest` suite for combinators and invariants.
- fuzzing:
  - candidate for parser-like validators (regex/date/url/json-path patterns).
- benchmarks:
  - existing criterion benches in `benches/`.
- CI quality gates:
  - `cargo fmt`, `clippy`, `test`, selected benches for regression checks.

## Exit Criteria

- coverage goals:
  - all public validator modules and combinator families have unit and integration coverage.
- flaky test budget:
  - zero tolerated for contract tests.
- performance regression thresholds:
  - benchmark regressions beyond agreed budget fail CI.
