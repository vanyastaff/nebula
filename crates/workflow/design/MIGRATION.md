# Migration

## Versioning Policy

- **Compatibility promise:** Patch/minor do not break serialized WorkflowDefinition or the validate_workflow / DependencyGraph public contract. Additive only (new optional fields, new error variants that do not change existing behavior).
- **Deprecation window:** Deprecated fields or functions get at least one minor version with deprecation notice and migration path before removal in major.

## Breaking Changes

- **Schema or validation contract change:** Major version bump.
  - **Old behavior:** Previous WorkflowDefinition JSON shape or validation semantics.
  - **New behavior:** New shape or rules; may reject previously valid definitions or accept new forms.
  - **Migration steps:** Document in release notes and MIGRATION.md; provide migration script or guide for stored workflows and API clients. Optional: compatibility layer or versioned deserializer for old schema.

## Rollout Plan

1. **Preparation:** Document breaking change; release MIGRATION.md and migration guide.
2. **Dual-run / feature-flag:** If applicable, API or storage supports both old and new schema during transition.
3. **Cutover:** Clients and storage migrate to new schema; old format no longer accepted after support window.
4. **Cleanup:** Remove compatibility code in next major.

## Rollback Plan

- **Trigger conditions:** Critical bug in new validation or schema.
- **Rollback steps:** Revert to previous crate version; ensure stored definitions and API clients match that version.
- **Data/state reconciliation:** Workflow definitions are immutable data; no runtime state in workflow crate. Storage may need to retain old format if rollback.

## Validation Checklist

- **API compatibility:** Serialized form roundtrip tests; schema snapshot (when present) passes.
- **Integration:** Engine and API tests pass with new workflow crate version.
- **Performance:** No significant regression in validate_workflow or graph build (if benchmarks exist).
