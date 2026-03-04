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

## Migration Guide Template (ACT-T023)

Use this template for every action interface version bump:

1. Summary
- Change title:
- Release version:
- Interface version change: `from X.Y` -> `to A.B`
- Breaking: `yes/no`

2. Why This Change Exists
- Problem statement:
- Technical rationale:
- Alternatives considered:

3. Contract Delta
- Affected types: (`ActionMetadata`, `ActionResult`, ports, context, etc.)
- Added fields/variants:
- Removed/renamed fields/variants:
- Serialization shape changes (old vs new JSON):

4. Consumer Impact
- Action authors:
- Runtime/engine:
- API/UI/tooling:
- Existing persisted executions:

5. Migration Steps
- Code changes required by consumers:
- Feature flags or dual-compat period:
- Required version gates:

6. Rollout Plan
- Phase 1 (canary):
- Phase 2 (partial rollout):
- Phase 3 (full rollout):

7. Rollback Plan
- Rollback trigger:
- Safe rollback target:
- Data/state reconciliation notes:

8. Verification
- Contract tests updated:
- Cross-crate integration tests passed:
- Performance/slo checks passed:

9. Deprecation Timeline
- Deprecated in:
- Removal in:
- Sunset communication link:
