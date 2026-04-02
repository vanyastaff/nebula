# Migration

## Versioning Policy

- compatibility promise:
  - stable behavior for core APIs and error semantics within a major version.
- deprecation window:
  - minimum one minor release for non-critical removals.

## Breaking Changes

- change:
  - possible introduction of unified memory runtime config.
  - old behavior:
    - per-module config bootstrap only.
  - new behavior:
    - optional unified config entry point with compatibility adapters.
  - migration steps:
    1. adopt new config where useful.
    2. keep existing module configs during transition.
- change:
  - potential extraction of experimental modules.
  - old behavior:
    - all surfaces inside one crate.
  - new behavior:
    - unstable APIs moved to sibling crates.
  - migration steps:
    1. switch imports/features to new crate paths.
    2. validate behavior in integration tests.

## Rollout Plan

1. preparation
   - publish proposals, shims, and compatibility notes.
2. dual-run / feature-flag stage
   - support old and new paths concurrently.
3. cutover
   - switch defaults or major release contract.
4. cleanup
   - remove deprecated paths after window expires.

## Rollback Plan

- trigger conditions:
  - integration regressions, performance drops, or compatibility failures.
- rollback steps:
  - pin previous version and disable new feature path.
- data/state reconciliation:
  - rebuild in-memory structures after rollback; no persistent migration state required.

## Validation Checklist

- API compatibility checks:
  - downstream compile checks for runtime/action consumers.
- integration checks:
  - pressure, budget, pool/cache scenarios remain stable.
- performance checks:
  - benchmark diffs remain within accepted threshold.
