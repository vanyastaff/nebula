# Migration

## Versioning Policy

- **Compatibility promise:** Patch/minor do not break Plugin trait, PluginRegistry public API, or PluginMetadata/PluginComponents shape. Additive only.
- **Deprecation window:** At least one minor version with notice before removal.

## Breaking Changes

- **Plugin trait or registry contract change:** Major version bump.
  - **Old behavior:** Previous Plugin::register signature, registry get/list return type, or metadata fields.
  - **New behavior:** New signature or shape; engine and API must adapt.
  - **Migration steps:** Document in release notes and MIGRATION.md; update engine and API to new API; recompile plugins (if trait changed).

## Rollout Plan

1. **Preparation:** Document break; release MIGRATION.md; coordinate with engine and API.
2. **Cutover:** Engine and API upgrade plugin crate; recompile and test.
3. **Cleanup:** Deprecated APIs removed in next major.

## Rollback Plan

- **Trigger conditions:** Critical bug in new plugin crate (e.g. registry get wrong, loader crash).
- **Rollback steps:** Revert to previous plugin version; ensure engine and API are compatible.
- **Data/state reconciliation:** Registry is in-memory; no persisted state. Reload plugins after rollback if needed.

## Validation Checklist

- **API compatibility:** Plugin trait and registry contract tests pass.
- **Integration:** Engine and API tests pass with new plugin version.
- **Dynamic-loading:** If used, load test lib and register; no regression.
