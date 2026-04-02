# Migration

## Versioning Policy

- **Compatibility promise:** Semantic versioning; minor versions additive; patch for bug fixes
- **Deprecation window:** 2 minor versions before removal; deprecation notice in changelog and docs

## Breaking Changes

- **SystemError variant changes:** If variants renamed or removed:
  - **Old behavior:** `PlatformError(String)`, `ResourceNotFound(String)`, etc.
  - **New behavior:** Document in CHANGELOG
  - **Migration steps:** Use `#[deprecated]` helpers or new constructors; update match arms

- **Pressure threshold changes:** If 50/70/85% adjusted:
  - **Old behavior:** Current thresholds
  - **New behavior:** New thresholds; consider `PressureConfig` (P001)
  - **Migration steps:** Document; provide compatibility shim if needed

- **Feature flag removal:** If a feature merged into default:
  - **Old behavior:** Feature-gated module
  - **New behavior:** Always available
  - **Migration steps:** Remove `#[cfg(feature)]`; update Cargo.toml; no code change for consumers

## Rollout Plan

1. **Preparation:** Update CHANGELOG; add migration guide
2. **Dual-run / feature-flag stage:** Optional; new behavior behind feature if complex
3. **Cutover:** Release with migration doc; consumers update
4. **Cleanup:** Remove deprecated APIs after deprecation window

## Rollback Plan

- **Trigger conditions:** Critical regression; build failures on supported platforms
- **Rollback steps:** Revert to previous crate version; consumers pin dependency
- **Data/state reconciliation:** No persistent state; in-memory caches only

## Validation Checklist

- **API compatibility checks:** `cargo check --workspace`; dependent crates build
- **Integration checks:** `cargo test --workspace`; nebula-memory tests pass
- **Performance checks:** `cargo bench -p nebula-system`; no regression beyond threshold
