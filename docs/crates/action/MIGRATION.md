# Migration

## Versioning Policy

- compatibility promise:
  - additive protocol changes in minor versions.
  - semantic/serialization contract breaks only in major versions.
- deprecation window:
  - keep deprecated contract helpers for at least one minor cycle unless security-critical.

## Breaking Changes

- likely future candidates:
  - introducing specialized core trait families
  - changing serialized contract of advanced flow variants
  - stronger typed keys/context capability trait split

## Rollout Plan

1. preparation:
  - add new contracts as additive fields/variants where possible.
2. dual-run / feature-flag stage:
  - runtime supports both old and new contract mapping.
3. cutover:
  - switch defaults and version gates.
4. cleanup:
  - remove deprecated branches after migration window.

## Rollback Plan

- trigger conditions:
  - runtime/engine incompatibility or consumer breakage in serialization semantics.
- rollback steps:
  - revert to prior compatible contract version and mapping layer.
- data/state reconciliation:
  - keep persisted execution states parseable across rollback boundary.

## Validation Checklist

- API compatibility checks:
  - compile checks across downstream crates.
- integration checks:
  - runtime/engine/sandbox contract fixtures.
- performance checks:
  - ensure no regressions in protocol handling overhead.
