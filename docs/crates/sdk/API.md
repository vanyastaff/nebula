# API

## Public Surface

- **Stable:** Prelude re-exports (core prelude, Value, macros, NodeBuilder, TriggerBuilder, TestContext, MockExecution when features enabled). Builder output and derive output must remain compatible with nebula-action contract. Patch/minor: additive re-exports and optional parameters only.
- **Experimental:** codegen and dev-server features when added; document as experimental until stable.
- **Hidden/internal:** Internal helpers for builders and testing; not part of author contract.

## Usage Patterns

- **Prelude:** `use nebula_sdk::prelude::*;` for ids, traits, Value, optional macros and builders.
- **Builders:** NodeBuilder, ParameterBuilder, WorkflowBuilder, TriggerBuilder for programmatic node definition when derive is not used.
- **Testing:** TestContext::builder(), MockExecution::builder(), ExecutionHarness for unit and integration tests without full engine.

## Minimal Example

See README: prelude, derive(Action, Parameters), execute with TestContext.

## Error Semantics

- **Build/validation errors:** Builder or parameter validation may return error; not retryable (fix input).
- **Test failures:** TestContext/MockExecution assert; failures are test failures, not runtime retry.
- **Compatibility:** If action or runtime contract changes and sdk is not updated, author may get compile or runtime errors; we document compatibility in README/MIGRATION.

## Compatibility Rules

- **Major bump:** Breaking prelude (removal, signature change), breaking builder or macro output vs action contract. MIGRATION.md required.
- **Minor:** Additive re-exports, new optional builder options, new test helpers. No removal.
- **Deprecation:** At least one minor version with notice before removal.
