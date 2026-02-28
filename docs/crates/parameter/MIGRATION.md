# Migration

## Versioning Policy

- **Compatibility promise:** Patch/minor preserve public API and `ParameterDef`/`ValidationRule`/`ParameterError` variants and codes
- **Deprecation window:** Minimum 6 months; announce in release notes; document in CHANGELOG

## Breaking Changes

- **Typed value layer (P-001):** If adopted
  - Old behavior: `ParameterValues` stores raw `serde_json::Value`; `get`/`set` with Value
  - New behavior: Typed `ParameterRuntimeValue`; JSON API deprecated
  - Migration steps: Use typed getters; migrate `set` to typed setters; remove deprecated API after window
- **ParameterKey newtype (P-003):** If adopted
  - Old behavior: `get(key: &str)`, `get_by_key(key: &str)`, etc.
  - New behavior: `get(key: ParameterKey)` or `get(key: impl Into<ParameterKey>)`
  - Migration steps: Wrap string keys in `ParameterKey::from(str)`; update lookup calls

## Rollout Plan

1. **Preparation:** Document breaking change; add migration guide; deprecate old API in minor release
2. **Dual-run / feature-flag stage:** New API available; old API deprecated but functional
3. **Cutover:** Major release removes deprecated API
4. **Cleanup:** Update all consumers; remove compatibility shims

## Rollback Plan

- **Trigger conditions:** Critical bug in new API; consumer migration blocked
- **Rollback steps:** Revert to previous minor; consumers pin version
- **Data/state reconciliation:** Schema and values are stateless; no persistence in this crate

## Validation Checklist

- **API compatibility checks:** `cargo check --workspace` with all dependents
- **Integration checks:** action, credential, macros tests pass
- **Performance checks:** Criterion benchmarks; no regression in validation latency
