# Migration

## Versioning Policy

- **Compatibility promise:** Patch/minor releases preserve config schema and public API
- **Deprecation window:** Minimum 6 months for breaking changes; migration guide in release notes

## Breaking Changes

- **Observability payload API (implemented):**
  - Old behavior: `ObservabilityEvent::data() -> Option<serde_json::Value>`
  - New behavior: `ObservabilityEvent::visit_fields(&mut dyn ObservabilityFieldVisitor)`
  - Compatibility helper: `event_data_json(&dyn ObservabilityEvent)` for JSON consumers
  - Migration steps:
    1. Replace `data()` implementations with `visit_fields()`
    2. In hooks, prefer visitor-based processing for hot paths
    3. If JSON is still required, use `event_data_json(event)` at integration boundaries

- **Typed event names (P-002):**
  - Old behavior: `ObservabilityEvent::name() -> &str`
  - New behavior: Add `EventKind` or typed key; deprecate string-only
  - Migration steps: Implement new trait method; migrate call sites; remove deprecated in next major

- **Context ID types (P-003):**
  - Old behavior: `ExecutionContext` etc. take `String` IDs
  - New behavior: Typed IDs from `nebula-core`
  - Migration steps: Add typed constructors; deprecate string constructors; migrate; remove

- **Hook policy (P-001):**
  - Old behavior: Inline hook execution only
  - Current behavior: Optional `Bounded` mode with budget diagnostics (inline dispatch)
  - Deferred behavior: Optional async offload mode with queue/drop accounting
  - Migration steps: Default unchanged; opt-in for bounded policy, and later opt-in for async offload when available

## Rollout Plan

1. **Preparation:** Document change; add deprecation warnings; migration guide
2. **Dual-run / feature-flag stage:** New API available; old API deprecated
3. **Cutover:** Consumers migrate; deprecation period elapses
4. **Cleanup:** Remove deprecated API in next major

## Rollback Plan

- **Trigger conditions:** Critical bug in new implementation; compatibility issue
- **Rollback steps:** Revert release; consumers pin previous version
- **Data/state reconciliation:** N/A; no persistent state in log crate

## Validation Checklist

- **API compatibility checks:** `cargo check` with dependent crates
- **Integration checks:** Full workspace test
- **Performance checks:** Criterion benchmarks; no regression
- **Schema compatibility checks:** Config fixtures and compatibility tests must pass for supported minor versions
