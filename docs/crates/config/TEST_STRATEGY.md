# Test Strategy

## Test Pyramid

- unit:
  - loaders, source parsing, merge utilities, path traversal, error mapping.
- integration:
  - builder + loader + validator + watcher combinations.
- contract:
  - precedence semantics, typed access compatibility, reload safety, governance/migration requirements.
- end-to-end:
  - consumer crate fixture tests (runtime/resource/credential initialization flows).

## Critical Invariants

- precedence order is deterministic.
- invalid config never becomes active.
- reload preserves last-valid state on failure.
- typed `get<T>` behavior is stable across versions.
- validator integration rejects invalid candidates without replacing active config.
- diagnostics/redaction contracts never leak sensitive validation payload values.

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

## Validator and Reload Quality Gates

- validator-focused CI gates:
  - every validator change must run contract reload rejection tests.
  - no merge if invalid candidate activation is observed.
- reload/backoff guidance:
  - failed reload attempts should be retried by caller/runtime with bounded backoff.
  - contract tests must assert failed reload leaves active snapshot unchanged.
- downstream contract requirements:
  - consumer crates must pin required config paths in fixtures.
  - CI should fail on `missing_path`/`type_mismatch` category drift.
  - config-validator category mapping fixtures must pass before release.

Validator integration contract suite:

- `tests/contract/validator_activation_contract_test.rs`
- `tests/contract/validator_reload_rejection_contract_test.rs`
- `tests/contract/validator_last_known_good_test.rs`
- `tests/contract/validator_category_compatibility_test.rs`
- `tests/contract/validator_redaction_contract_test.rs`
- `tests/contract/validator_diagnostics_context_test.rs`
- `tests/contract/validator_governance_policy_test.rs`
- `tests/contract/validator_migration_requirements_test.rs`
- `tests/contract/validator_runbook_requirements_test.rs`

Core contract suite additionally covers:

- `tests/contract/activation_atomicity_test.rs`
- `tests/contract/last_known_good_preservation_test.rs`
- `tests/contract/reload_rejection_contract_test.rs`
- `tests/contract/env_precedence_contract_test.rs`
- `tests/contract/precedence_matrix_contract_test.rs`
- `tests/contract/typed_access_compatibility_test.rs`
- `tests/contract/path_error_categories_test.rs`
- `tests/contract/merge_determinism_test.rs`
- `tests/contract/governance_policy_test.rs`
- `tests/contract/migration_requirements_test.rs`

## Exit Criteria

- coverage goals:
  - all source types and core merge/path APIs covered.
- flaky test budget:
  - zero for precedence and validation contract tests.
- performance regression thresholds:
  - measurable budgets for load/reload/get paths enforced.
