# Proposals (Senior Review)

## P-001: Introduce Fail-fast vs Collect-all Execution Mode (Breaking Behavior)

Problem:
- current composition often implies specific error aggregation behavior that may not fit all high-load paths.

Proposal:
- add explicit execution mode to key combinators:
  - `FailFast`
  - `CollectAll`

Impact:
- behavior change in error output ordering/volume for some existing pipelines.

Migration:
1. default to legacy behavior initially.
2. add explicit mode APIs.
3. switch defaults only in major release.

## P-002: Stable Machine-readable Error Code Registry

Problem:
- API/UI layers rely on error codes for mapping, but code stability rules are implicit.

Proposal:
- define and version a formal error code registry in crate docs/tests.

Impact:
- non-breaking now; prevents accidental breaking API changes later.

## P-003: Typed FieldPath Instead of Raw String Paths (Potential Breaking)

Problem:
- field paths are represented as strings and manipulated manually.

Proposal:
- add `FieldPath` type with validated segments and conversion to display string.

Impact:
- stronger correctness and less path formatting drift; API signatures may change.

## P-004: Macro Expansion Debug Mode

Problem:
- complex `validator!` expansions are hard to inspect in large projects.

Proposal:
- add optional macro debugging helpers/docs (`cargo expand` recipes and internal markers).

Impact:
- non-breaking developer-experience improvement.

## P-005: Context Key Typing Strategy

Problem:
- `ValidationContext` currently uses string keys; typo risk and weak discoverability.

Proposal:
- introduce typed key wrappers or module-level constants for context keys.

Impact:
- can be introduced backward-compatibly, then tightened in a future major.
