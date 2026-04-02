# Compatibility Policy

**Phase 1: Contract freeze** — explicit stability promises for nebula-action.

## Contract surface

The following types are **schema-stable**. Their serialized form (JSON at engine/runtime/API boundaries) must not change in patch/minor releases. Changes require a major version and MIGRATION.md.

| Type | Location | Serialization | Notes |
|------|----------|---------------|-------|
| `ActionOutput<T>` | output.rs | Tagged enum `{"type":"Variant","data":...}` | `Value`, `Empty`, `Binary`, `Reference`, `Deferred`, `Streaming`, `Collection` |
| `ActionResult<T>` | result.rs | Tagged enum `{"type":"Variant",...}` | `Success`, `Skip`, `Continue`, `Break`, `Branch`, `Route`, `MultiOutput`, `Wait`, `Retry`; duration fields serialized as milliseconds |
| `FlowKind` | port.rs | String | `"main"`, `"error"` |
| `InputPort` / `OutputPort` / `SupportPort` / `DynamicPort` | port.rs | Tagged by port type | Port schema used by UI and engine |
| `BreakReason` / `WaitCondition` | result.rs | Enum/tagged enum | Flow-control compatibility contract |
| `ActionMetadata` (key, version, ports) | metadata.rs | Stable structure | Interface version governs breaking changes |

`ActionError` remains semantic-contract stable even where not serialized at all boundaries: variant names and meanings must not change in patch/minor releases.

## Enforcement

- **Contract tests** in `crates/action/tests/contracts.rs` assert JSON shape for `ActionOutput`, `ActionResult`, `FlowKind`, `SupportPort`, `DynamicPort`, `BreakReason`, and `WaitCondition`.
- CI runs these tests; accidental drift fails the build.
- Intentional changes: update expected values, bump major version, document in MIGRATION.md.

## Rules

1. **Patch/minor:** No breaking changes to serialized form or to result/error variant semantics.
2. **Major:** Document in MIGRATION.md; provide compatibility path where possible.
3. **Deprecation:** Minimum 6 months; `#[deprecated]` with replacement path.

## Metadata version

`ActionMetadata.version` (InterfaceVersion) is the interface contract version. Breaking port or parameter schema changes require a version bump and compatibility notes.
