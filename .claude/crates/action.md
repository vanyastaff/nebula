# nebula-action
Action trait hierarchy and execution contract — Ports & Drivers architecture.

## Invariants
- Defines **what** actions are, not how the engine runs them. Execution environments are in nebula-runtime.
- Actions may run concurrently — no mutable state in struct fields. Use `StatefulAction` for state.
- `Context` is always injected — never construct `ActionContext` directly.

## Key Decisions
- Action subtypes: `StatelessAction`, `StatefulAction` (Continue/Break loop), `TriggerAction`, `ResourceAction`.
- `credential_typed::<S>()` and `resource_typed::<R>()` on `ActionContext` — primary typed access paths for action authors. Both return `ActionError::Fatal` on mismatch.
- `ResourceAccessor::acquire` returns `Box<dyn Any + Send>` (not `+ Sync`).
- `ActionResult` carries output data + flow-control intent (branch, wait, error).

## Traps
- `#[derive(Action)]` requires **unit structs**. Config goes in a separate injected type.
- `ActionError::retryable()` vs `fatal()` — engine uses this to decide retry.
- `testing` module removed — no more `TestContextBuilder`/`SpyLogger`.

## Relations
- Depends on nebula-core, nebula-parameter, nebula-resource, nebula-credential. Used by nebula-engine, nebula-runtime, nebula-sdk.

<!-- reviewed: 2026-04-07 — resource_typed added, testing module removed, ResourceAccessor signature changed -->
