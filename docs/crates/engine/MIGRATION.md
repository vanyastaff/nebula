# Migration

## Versioning Policy

- **Compatibility promise:** Patch/minor do not break execute_workflow signature, ExecutionResult, or execution/context contract. Additive only.
- **Deprecation window:** Deprecated APIs get at least one minor version with notice and migration path before removal in major.

## Breaking Changes

- **Execution or context contract change:** Major version bump.
  - **Old behavior:** Previous ExecutionResult shape, context fields, or execute_workflow options.
  - **New behavior:** New shape or semantics; API/workers must adapt.
  - **Migration steps:** Document in release notes and MIGRATION.md; update API and worker call sites; ensure state store and execution crate align.

## Rollout Plan

1. **Preparation:** Document breaking change; release MIGRATION.md; coordinate with api, runtime, execution.
2. **Dual-run:** If applicable, run new engine version behind feature flag or in canary.
3. **Cutover:** API and workers migrate to new engine version; old version retired.
4. **Cleanup:** Remove deprecated code in next major.

## Rollback Plan

- **Trigger conditions:** Critical bug in new engine (e.g. wrong scheduling, state corruption).
- **Rollback steps:** Revert to previous engine version; ensure API/workers and state store are compatible.
- **Data/state reconciliation:** Execution state is in execution crate / state store; ensure format matches reverted engine.

## Validation Checklist

- **API compatibility:** Execute_workflow and ExecutionResult contract tests pass.
- **Integration:** Engine + runtime + execution tests pass; API/worker (if in repo) pass.
- **Performance:** No significant regression in execute_workflow benchmarks (if present).
