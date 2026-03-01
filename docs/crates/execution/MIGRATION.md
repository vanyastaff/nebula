# Migration

## Versioning Policy

- **Compatibility promise:** Patch and minor releases do not break public API or serialized form of ExecutionStatus, ExecutionState, NodeExecutionState, ExecutionOutput, NodeOutput, JournalEntry, IdempotencyKey. New optional fields and new enum variants (with default handling) are allowed in minor. Breaking changes only in major with this migration guide.
- **Deprecation window:** Deprecated APIs are marked in rustdoc and retained for at least one minor cycle; removal in next major.

## Breaking Changes

(No breaking changes yet. Template for future.)

- **Change:** [Description]
  - **Old behavior:** [What used to happen]
  - **New behavior:** [What happens now]
  - **Migration steps:** [How to update callers or data]

## Rollout Plan

1. **Preparation:** Update dependent crates (engine, api) to use new execution crate version; run tests.
2. **Dual-run / feature-flag stage:** If behavior change, run both old and new in shadow or behind flag where applicable.
3. **Cutover:** Deploy engine/API with new execution crate; persist state in new format if schema changed.
4. **Cleanup:** Remove deprecated APIs in next major; remove compatibility shims.

## Rollback Plan

- **Trigger conditions:** Regression in engine (e.g. transition or idempotency behavior); API response shape broken.
- **Rollback steps:** Revert engine/API to previous version that depends on previous execution crate; ensure persisted state is readable (no format change in patch/minor).
- **Data/state reconciliation:** If major upgrade changed state schema, rollback may require migration of persisted state or run with compatibility layer; document per major release.

## Validation Checklist

- **API compatibility:** If API returns execution state or result, ensure JSON shape unchanged in patch/minor; run contract or snapshot tests.
- **Integration:** Engine and runtime tests pass with new execution crate version.
- **Performance:** No regression in plan build or transition hot path (if benchmarks exist).
