# Proposals (Senior Review)

## P-001: Hook Execution Isolation with Budget (Breaking Runtime Behavior)

Problem:
- hooks execute inline during `emit_event`, so slow hooks can increase tail latency.

Proposal:
- introduce optional hook budget policy:
  - max execution time per hook
  - optional async offload queue
  - drop/shed strategy with counters

Impact:
- behavioral change in hook timing/order when budget mode is enabled.

Implementation:
1. add `HookPolicy` config (`Inline`, `BoundedAsync`).
2. default remains `Inline` for compatibility.
3. expose hook lag/drop metrics.

## P-002: Typed Event Names Instead of Free Strings (Breaking API)

Problem:
- string event names are typo-prone and hard to refactor.

Proposal:
- introduce typed event identifiers (`EventKind` enum or interned key type).

Impact:
- custom hook/event implementations need migration.

Implementation:
1. add dual API supporting old `name()` and new typed key.
2. deprecate string-only trait method.
3. remove old method in next major.

## P-003: Context ID Types from `nebula-core` (Potential Breaking)

Problem:
- execution/node/workflow IDs in observability contexts are currently plain `String`.

Proposal:
- migrate to typed IDs from `nebula-core` where feasible.

Impact:
- signature changes for context constructors and integrations.

Implementation:
1. add typed constructor variants while preserving string constructors temporarily.
2. migrate internal callers.
3. deprecate string-only constructors.

## P-004: Writer Multi-Fanout with Explicit Failure Policy

Problem:
- `WriterConfig::Multi` currently uses first writer only.

Proposal:
- implement true fanout with policy:
  - `FailFast`
  - `BestEffort`
  - `PrimaryWithFallback`

Impact:
- non-breaking API if implemented behind enriched config; behavior improves materially.

## P-005: Config Schema Stability Contract

Problem:
- many deployment configs depend on serde schema stability.

Proposal:
- version config schema and add snapshot tests for config serialization/deserialization.

Impact:
- prevents accidental breaking config changes in releases.
