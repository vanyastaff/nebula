# Migration

## Versioning Policy

- compatibility promise:
  - stable locale negotiation and key-render semantics within major versions.
- deprecation window:
  - one minor release minimum for non-critical key/API transitions.

## Breaking Changes

- change:
  - migration from ad-hoc localized strings to key-based catalogs.
  - old behavior:
    - inline per-crate messages with inconsistent fallback.
  - new behavior:
    - centralized key-based translation contract.
  - migration steps:
    1. extract existing messages to keys.
    2. map legacy messages to compatibility aliases.
    3. switch consumers to translator API.
- change:
  - locale negotiation precedence standardization.
  - old behavior:
    - endpoint-specific precedence rules.
  - new behavior:
    - unified precedence contract.
  - migration steps:
    1. run endpoint behavior diff.
    2. update API contracts and tests.

## Rollout Plan

1. preparation
   - define key namespaces and baseline catalogs.
2. dual-run / feature-flag stage
   - compare old/new localized outputs in staging.
3. cutover
   - enable centralized locale manager.
4. cleanup
   - remove legacy inline localization paths.

## Rollback Plan

- trigger conditions:
  - high missing-key rates, severe UX regressions, render failures.
- rollback steps:
  - revert to previous catalog/profile and disable new strict modes.
- data/state reconciliation:
  - reconcile key aliases and catalog versions.

## Validation Checklist

- API compatibility checks:
  - compile/runtime checks for api/runtime/action/validator consumers.
- integration checks:
  - negotiation/fallback/interpolation behavior across endpoints.
- performance checks:
  - render and lookup latency within baseline limits.
