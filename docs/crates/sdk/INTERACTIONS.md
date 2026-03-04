# Interactions

## Ecosystem Map

**nebula-sdk** is the developer facade: re-exports and builders for action/plugin authors. It depends on core, action, macros, and optionally other crates; no crate in the workspace depends on sdk (authors depend on it as a library).

### Upstream (sdk depends on)

- **nebula-core** — prelude, identifiers, traits.
- **nebula-action** — Action trait, context types, metadata (re-exported or used by builders).
- **nebula-macros** — derive(Action), Parameters, etc. (optional/feature).
- **nebula-workflow** — WorkflowBuilder, definition types (for testing/workflow build).
- **serde_json** — Value (re-exported).
- Optional: expression, parameter, validator (for richer builders or testing).

### Downstream (consume sdk)

- **Plugin/action authors** — external crates; depend on nebula-sdk for prelude, builders, TestContext, MockExecution. Not in workspace.
- **Tests and examples** — in-repo tests may use sdk for node authoring and testing.

### Planned

- CLI (nebula init, build, test) may use sdk for authoring and run; dev server may use sdk.

## Downstream Consumers

- **Action authors:** Expect stable prelude, NodeBuilder/TriggerBuilder or derive output compatible with engine/runtime; TestContext and MockExecution that match action/runtime contract.
- **Plugin authors:** Same; sdk is the single entry point for "how to build a node."

## Upstream Dependencies

- **core, action, macros:** Required for prelude and authoring. Breaking change in those crates may require sdk major or minor to align.
- **Fallback:** None; sdk is a facade, not a fallback.

## Interaction Matrix

| This crate ↔ Other | Direction | Contract | Sync/Async | Failure handling | Notes |
|--------------------|-----------|----------|------------|------------------|-------|
| sdk → core | in | prelude re-exports, IDs, traits | sync | N/A | |
| sdk → action | in | Action, context, metadata; builder output | sync | N/A | |
| sdk → macros | in | derive output | sync | compile-time | |
| sdk → workflow | in | WorkflowBuilder, definition (testing) | sync | N/A | |
| authors → sdk | in | prelude, builders, TestContext, MockExecution | sync/async | test failures | |

## Runtime Sequence

1. Author adds nebula-sdk dependency; uses prelude and/or builders/macros to define node.
2. Build or macro produces type implementing Action (or equivalent); engine/runtime load and run it.
3. In tests, author uses TestContext/MockExecution to run node without full engine; contract: same context shape as runtime.

## Cross-Crate Ownership

- **sdk** owns: prelude surface, builder API, TestContext/MockExecution, compatibility story for authors.
- **action** owns: Action trait and execution contract; sdk re-exports and builds against it.
- **macros** owns: derive output; sdk re-exports and documents usage.
- **engine/runtime** own: execution; sdk does not run workflows.

## Versioning and Compatibility

- Prelude and re-export stability: minor = additive only; removal or signature change = major. Document in API.md and MIGRATION.md.
- Compatibility matrix: sdk version X works with core/action/macros version Y; document in README or MIGRATION.
