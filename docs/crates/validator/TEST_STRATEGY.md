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

## Contract Suite (Current)

- `tests/contract/compatibility_fixtures_test.rs`
- `tests/contract/typed_dynamic_equivalence_test.rs`
- `tests/contract/combinator_semantics_contract_test.rs`
- `tests/contract/adversarial_inputs_test.rs`
- `tests/contract/error_envelope_schema_test.rs`
- `tests/contract/safe_diagnostics_test.rs`
- `tests/contract/error_tree_bounds_test.rs`
- `tests/contract/governance_policy_test.rs`
- `tests/contract/migration_requirements_test.rs`

## CI Quality Gates (Validator-focused)

Recommended commands:

```bash
cargo test -p nebula-validator
cargo bench -p nebula-validator
cargo fmt --all -- --check
cargo clippy --workspace -- -D warnings
cargo check --workspace --all-targets
cargo doc --no-deps --workspace
```

Benchmark policy:

- regressions above agreed budget on hot combinator chains must fail CI.

Config integration gates:

- run config-validator compatibility fixtures before release.
- ensure shared category constants match config contract fixture baseline.
- fail release readiness if migration mapping is missing for category changes.

## Exit Criteria

- coverage goals:
  - all public validator modules and combinator families have unit and integration coverage.
- flaky test budget:
  - zero tolerated for contract tests.
- performance regression thresholds:
  - benchmark regressions beyond agreed budget fail CI.
