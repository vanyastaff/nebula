# Compatibility Fixtures

This directory contains versioned contract fixtures for `nebula-config`.

## Format

- `fixture_version`: semantic version for fixture schema.
- `contract`: contract name (`precedence`, `path_access`, etc).
- `data` / `defaults` / `sources`: input layers used in the scenario.
- `expected`: resolved output or typed access expectation.
- `error_cases`: expected stable error categories for negative scenarios.

## Governance

- Existing fixture behavior is immutable within minor releases.
- Add new fixture files for additive behavior.
- If a behavior changes incompatibly, require:
  - major version bump
  - migration mapping update in `docs/crates/config/MIGRATION.md`
  - corresponding contract test updates

