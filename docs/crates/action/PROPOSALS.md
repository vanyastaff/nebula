# Proposals

## P001: Specialized execution trait families (from archive)

Idea:
- formalize core trait families:
  - `StatelessAction`
  - `StatefulAction`
  - `TriggerAction`
  - `ResourceAction`

Benefit:
- explicit execution semantics for engine and action authors.

Potential break:
- if introduced in core immediately, existing `Action`-only code may need migration.

## P002: DX traits as optional layer

Idea:
- add optional high-level patterns on top of core traits:
  - `InteractiveAction`
  - `TransactionalAction`
  - `WebhookAction`
  - `PollAction`

Benefit:
- fast authoring without bloating base contracts.

Potential break:
- minimal if grouped in optional in-crate module (e.g. dx).

## P003: Advanced flow-control variants (staged)

Idea:
- evaluate staged introduction of orchestration variants from drafts:
  - `Fork`
  - `Join`
  - `Delegate`
  - `Suspend`

Benefit:
- richer graph semantics for complex workflows.

Potential break:
- affects engine protocol and persistence format; must be major-version gated.

## P004: Strongly typed action keys

Idea:
- add `ActionKey` newtype with validation rules instead of plain `String`.

Benefit:
- fewer invalid key formats and cleaner registries.

Potential break:
- API signatures using raw strings may change.

## P005: Structured retry hints in `ActionResult::Retry`

Idea:
- extend retry result with jitter class / budget class metadata.

Benefit:
- engine can apply smarter retry policy without custom parsing.

Potential break:
- serialized form of `ActionResult` may change.

## P006: Capability-declared context traits

Idea:
- split `Context` into optional capability traits (`HasResources`, `HasCredentials`, etc.).

Benefit:
- clearer compile-time contracts for action authors.

Potential break:
- existing generic signatures expecting only `Context` would need updates.

## P007: Output schema contracts

Idea:
- pair `ActionOutput::Value` with optional schema id/version marker.

Benefit:
- safer cross-node compatibility checks at workflow publish time.

Potential break:
- metadata and runtime validation path becomes stricter.

## P008: Trait package for common action patterns

Idea:
- publish official pattern traits/builders (process/stateful/trigger) aligned with engine.

Benefit:
- consistent authoring model for community actions.

Potential break:
- previous experimental trait names could be deprecated/removed.
