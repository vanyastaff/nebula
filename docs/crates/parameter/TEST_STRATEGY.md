# Test Strategy

## Test Pyramid

- **Unit:** `ParameterMetadata`, `ParameterKind` capabilities, `DisplayCondition::evaluate`, `ValidationRule` builders, `ParameterValues` snapshot/diff, `ParameterError` codes
- **Integration:** `ParameterCollection::validate` with nested Object/List; display rule evaluation with `DisplayContext`; serde round-trip for all parameter types
- **Contract:** action/credential schema compatibility; validator rule mapping
- **End-to-end:** Examples as smoke tests; credential protocol validation flows

## Critical Invariants

- Validation never panics on valid `ParameterValues`; returns `Vec<ParameterError>` for invalid
- Display-only kinds (Notice, Group) are skipped in validation
- `hide_when` takes priority over `show_when`
- Error aggregation: all failures collected, not first-only
- Serde round-trip preserves schema semantics for all `ParameterDef` variants

## Scenario Matrix

- **Happy path:** Minimal schema + valid values → `Ok(())`
- **Retry path:** N/A (deterministic)
- **Cancellation path:** N/A
- **Timeout path:** N/A
- **Upgrade/migration path:** Schema version changes; see MIGRATION.md

## Tooling

- **Property testing:** proptest for `ParameterValues` (optional)
- **Fuzzing:** Serde deserialization; validation with random values (optional)
- **Benchmarks:** criterion for deep nesting validation; large collection
- **CI quality gates:** `cargo test -p nebula-parameter -- --nocapture`; `cargo clippy`; format check

## Exit Criteria

- **Coverage goals:** All `ParameterDef` variants; validation paths; display conditions
- **Flaky test budget:** Zero
- **Performance regression thresholds:** Criterion benchmarks with CI baseline (Phase 2)
