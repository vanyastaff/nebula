# Migration

## Versioning Policy

- **Compatibility promise:** Patch/minor preserve public API and serialized forms. No breaking changes without major version bump.
- **Deprecation window:** Minimum 6 months. Deprecated items have `#[deprecated]` with replacement path; MIGRATION.md documents migration steps.

## Breaking Changes

### P-001: CoreError Domain Variants (Planned)

- **Old behavior:** `CoreError` includes `WorkflowExecution`, `NodeExecution`, `Cluster`, `Tenant`, etc.
- **New behavior:** These variants move to owning crates (`engine`, `runtime`, `cluster`, `tenant`).
- **Migration steps:**
  1. Add crate-local error enums in owning crates
  2. Replace `CoreError::WorkflowExecution` with `engine::EngineError::WorkflowExecution`, etc.
  3. Use `From<CoreError>` where foundation errors still needed

### P-002: Strict Scope Containment (Planned)

- **Old behavior:** `ScopeLevel::is_contained_in` uses simplified semantics; execution/workflow containment not fully verified.
- **New behavior:** `is_contained_in_strict(&self, other, resolver)` requires ownership resolver.
- **Migration steps:**
  1. Implement `ScopeResolver` in engine/runtime
  2. Replace `is_contained_in` with `is_contained_in_strict` where security matters
  3. Remove simplified behavior in next major

### P-003: Constants Migration (Planned)

- **Old behavior:** `constants.rs` contains domain-specific defaults (api, runtime, storage, etc.).
- **New behavior:** Domain constants move to owning crates.
- **Migration steps:**
  1. Update imports from `nebula_core::constants::*` to crate-local constants
  2. Deprecated aliases in core for one cycle
  3. Remove aliases in next major

## Rollout Plan

1. **Preparation:** Document breaking changes; add deprecation attributes; update PROPOSALS status
2. **Dual-run / feature-flag stage:** N/A for core — consumers adopt new APIs
3. **Cutover:** Consumers migrate; core releases major version
4. **Cleanup:** Remove deprecated items

## Rollback Plan

- **Trigger conditions:** Critical bug in new behavior; consumer migration blocked
- **Rollback steps:** Revert to previous major; consumers pin to old version
- **Data/state reconciliation:** Core has no persistent state; no reconciliation needed

## Validation Checklist

- **API compatibility checks:** `cargo check --workspace` with consumer crates
- **Integration checks:** Full workspace tests pass
- **Performance checks:** P-005 benchmarks; no regression
