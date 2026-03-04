# Migration

## Versioning Policy

- **Compatibility promise:** Minor = additive attributes only; generated code remains backward-compatible (no signature change, no removal of generated items). Patch = bug fixes and diagnostics only.
- **Deprecation window:** Deprecated attribute gets at least one minor version with deprecation notice and migration path before removal (major).

## Breaking Changes

- **Attribute or output contract change:** Major version bump.
  - **Old behavior:** Previous attribute set or generated code (e.g. impl Action signature, metadata fields).
  - **New behavior:** Removed or changed attribute; or generated code no longer matches previous shape. Authors must update attributes or trait crate version.
  - **Migration steps:** Document in release notes and MIGRATION.md; list attribute changes and any code changes required. Compatibility matrix: macro version X works with action/plugin/credential/resource version Y.

## Rollout Plan

1. **Preparation:** Document break; publish MIGRATION.md and compatibility matrix.
2. **Cutover:** Authors upgrade macro (and optionally trait crates); fix attributes or code per MIGRATION; recompile.
3. **Cleanup:** Deprecated attributes removed in next major.

## Rollback Plan

- **Trigger conditions:** Critical bug in new macro (wrong expansion, breakage for many authors).
- **Rollback steps:** Authors revert to previous macro version; ensure trait crates are compatible (see matrix).
- **Data/state reconciliation:** N/A (no persisted state; compile-time only).

## Validation Checklist

- **API compatibility:** Contract tests (expand + compile with trait crates) pass. Document compatible versions.
- **Integration:** SDK and author examples compile with new macro version.
