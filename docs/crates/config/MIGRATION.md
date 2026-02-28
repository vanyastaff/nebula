# Migration

## Versioning Policy

- compatibility promise:
  - additive source/validator/watcher APIs in minor releases.
  - precedence/path semantic changes only in major releases.
- deprecation window:
  - keep deprecated APIs for at least one minor cycle unless security-critical.

## Breaking Changes

- likely future candidates:
  - merge strategy profile defaults
  - typed path API replacing free-form string paths
  - remote source trust policy enforcement defaults

## Rollout Plan

1. preparation:
  - introduce new behavior behind explicit builder options.
2. dual-run / feature-flag stage:
  - run old and new semantics in validation mode to compare outcomes.
3. cutover:
  - switch defaults in major release.
4. cleanup:
  - remove deprecated behavior paths after migration window.

## Rollback Plan

- trigger conditions:
  - unexpected consumer breakage in precedence/path/typed retrieval behavior.
- rollback steps:
  - revert to previous compatible release and source settings.
- data/state reconciliation:
  - ensure last-known-good active config snapshot remains valid and restorable.

## Validation Checklist

- API compatibility checks:
  - compile and integration checks against consumer crates.
- integration checks:
  - startup and reload scenarios with mixed source sets.
- performance checks:
  - compare load/reload/read latency against baseline.
