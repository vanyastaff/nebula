# Migration

## Versioning Policy

- compatibility promise:
  - minor releases keep behavior-compatible validator semantics.
  - major releases may change semantics with explicit migration steps.
- deprecation window:
  - at least one minor release before removal of deprecated APIs (unless security-critical).

## Breaking Changes

- currently planned:
  - none committed.
- potential future breaking candidates:
  - typed `FieldPath`
  - explicit fail-fast/collect-all default policy change
  - strict error code registry enforcement

## Rollout Plan

1. preparation:
  - introduce new APIs in additive form behind clear docs.
2. dual-run / feature-flag stage:
  - allow old and new behavior side-by-side where possible.
3. cutover:
  - switch defaults only in major release.
4. cleanup:
  - remove deprecated path after migration window.

## Rollback Plan

- trigger conditions:
  - consumer breakage in error-code/path contracts.
- rollback steps:
  - revert to previous stable version and restore compatibility mapping.
- data/state reconciliation:
  - ensure persisted validation error envelopes remain parseable by consumers.

## Validation Checklist

- API compatibility checks:
  - compile-time checks for public trait signatures.
- integration checks:
  - consumer fixtures for `api`, `workflow`, `plugin`.
- performance checks:
  - benchmark comparison against previous baseline.
