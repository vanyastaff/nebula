# Migration

## Versioning Policy

- **Compatibility promise:** Patch/minor preserve `Plugin`, `PluginMetadata`, `PluginComponents`, `PluginRegistry`, `PluginType`, `PluginVersions`, `PluginError`. Breaking changes via major version.
- **Deprecation window:** Minimum 6 months for public API deprecations.

## Breaking Changes

### InternalHandler Deprecation (Future)

- **Old behavior:** `PluginComponents::handler(Arc<dyn InternalHandler>)`; `InternalHandler` trait
- **New behavior:** Typed `process_action`, `stateful_action` restored; `InternalHandler` removed
- **Migration steps:**
  1. Replace `components.handler(adapter)` with `components.process_action(action)` (or stateful_action)
  2. Remove `InternalHandler` impl
  3. Update to new nebula-plugin version

### PluginMetadata Schema Change (If Any)

- **Old behavior:** Current serde schema
- **New behavior:** New fields or renamed fields
- **Migration steps:** Document in release notes; ensure backward-compatible deserialization where possible

## Rollout Plan

1. **Preparation:** Document breaking change in MIGRATION.md; release notes
2. **Dual-run / feature-flag stage:** N/A for plugin (no runtime feature flags)
3. **Cutover:** Consumers update to new API
4. **Cleanup:** Remove deprecated APIs in next major

## Rollback Plan

- **Trigger conditions:** Critical bug in new version
- **Rollback steps:** Downgrade nebula-plugin; revert consumer code
- **Data/state reconciliation:** Registry is in-memory; no persisted state in plugin

## Validation Checklist

- **API compatibility checks:** `cargo check` with dependent crates
- **Integration checks:** `cargo test --workspace`
- **Performance checks:** Registry lookup benchmark (if added)
