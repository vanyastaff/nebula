# Compatibility Policy

**Phase 1: Contract freeze** — explicit stability promises for nebula-action.

## Contract surface

The following types are **schema-stable**. Their serialized form (JSON at engine/runtime/API boundaries) must not change in patch/minor releases. Changes require a major version and MIGRATION.md.

| Type | Location | Serialization | Notes |
|------|----------|---------------|-------|
| `ActionOutput<T>` | output.rs | Tagged enum `{"type":"Variant","data":...}` | `Value`, `Empty`, `Binary`, `Reference`, `Deferred`, `Streaming`, `Collection` |
| `FlowKind` | port.rs | String | `"main"`, `"error"` |
| `InputPort` / `OutputPort` / `SupportPort` | port.rs | Tagged by port type | Port schema used by UI and engine |
| `ActionMetadata` (key, version, ports) | metadata.rs | Stable structure | Interface version governs breaking changes |

Types not yet serialized at boundaries (e.g. `ActionResult`, `ActionError`, `BreakReason`, `WaitCondition`) are still part of the *semantic* contract: variant names and semantics must not change in patch/minor; adding serde later must not break existing consumers.

## Enforcement

- **Contract tests** in `crates/action/tests/contracts.rs` assert JSON shape for `ActionOutput` and `FlowKind`.
- CI runs these tests; accidental drift fails the build.
- Intentional changes: update expected values, bump major version, document in MIGRATION.md.

## Rules

1. **Patch/minor:** No breaking changes to serialized form or to result/error variant semantics.
2. **Major:** Document in MIGRATION.md; provide compatibility path where possible.
3. **Deprecation:** Minimum 6 months; `#[deprecated]` with replacement path.

## Metadata version

`ActionMetadata.version` (InterfaceVersion) is the interface contract version. Breaking port or parameter schema changes require a version bump and compatibility notes.
