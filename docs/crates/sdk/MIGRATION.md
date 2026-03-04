# Migration

## Versioning Policy

- **Compatibility promise:** Minor = additive re-exports and optional builder/test helpers only. No removal or signature change in prelude or stable builders without major.
- **Deprecation window:** At least one minor version with deprecation notice and migration path before removal.

## Breaking Changes

- **Prelude or builder contract break:** Major version bump.
  - **Old behavior:** Previous re-exports or builder output.
  - **New behavior:** Removed or changed; authors must update imports or code.
  - **Migration steps:** Document in release notes; provide upgrade guide (import changes, builder API changes). Compatibility matrix: sdk X requires core/action Y.

## Rollout Plan

1. **Preparation:** Document break in MIGRATION.md and release notes; publish compatibility matrix.
2. **Cutover:** Authors upgrade sdk and fix code; no dual-run (library only).
3. **Cleanup:** Deprecated APIs removed in next major.

## Rollback Plan

- **Trigger conditions:** Critical bug in new sdk (e.g. wrong re-export, broken TestContext).
- **Rollback steps:** Authors revert to previous sdk version; ensure Cargo.lock or version bound allows it.
- **Data/state reconciliation:** N/A (no persisted state in sdk).

## Validation Checklist

- **Compatibility:** Prelude and builder contract tests pass; document compatible core/action versions.
- **Integration:** Tests with action and optional engine pass after upgrade.
