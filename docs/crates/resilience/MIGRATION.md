# Migration

## Versioning Policy

- **Compatibility promise:** semantic versioning; minor versions additive; patch for bug fixes.
- **Deprecation window:** 2 minor versions before removal; deprecation notice in changelog and doc.

## Breaking Changes

- **Policy schema change:** if `ResiliencePolicy` or `RetryPolicyConfig` fields change incompatibly:
  - **Old behavior:** deserialization fails or ignores unknown fields.
  - **New behavior:** new schema with migration path.
  - **Migration steps:** provide migration script or config transformer; document in CHANGELOG.

- **Pattern order contract (P-001):** if canonical order is enforced:
  - **Old behavior:** caller-defined order; possible inconsistency.
  - **New behavior:** fixed order (e.g., timeout → bulkhead → circuit → retry).
  - **Migration steps:** audit existing compositions; adjust if order differs; document in PROPOSALS.

- **Cancellation semantics (P-005):** if unified:
  - **Old behavior:** subtle differences between patterns.
  - **New behavior:** consistent propagation.
  - **Migration steps:** test cancellation paths; fix edge cases; document guarantees.

## Rollout Plan

1. **Preparation:** update policy schema/order/cancellation; add compatibility layer if needed.
2. **Dual-run / feature-flag stage:** optional; run new behavior behind flag for validation.
3. **Cutover:** release with migration doc; consumers update config/code.
4. **Cleanup:** remove deprecated APIs after deprecation window.

## Rollback Plan

- **Trigger conditions:** critical regression; policy load failures; performance degradation.
- **Rollback steps:** revert to previous crate version; restore previous config if schema changed.
- **Data/state reconciliation:** resilience is stateless per execution; no persistent state to reconcile. Policy config in external store may need manual rollback.

## Validation Checklist

- **API compatibility checks:** `cargo check --workspace`; dependent crates build.
- **Integration checks:** `cargo test --workspace`; engine/runtime integration tests pass.
- **Performance checks:** `cargo bench -p nebula-resilience`; no regression beyond threshold.
