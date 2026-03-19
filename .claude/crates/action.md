# nebula-action
Action trait hierarchy and execution contract — Ports & Drivers architecture.

## Invariants
- Defines **what** actions are, not how the engine runs them. Concrete execution environments (InProcessSandbox) are in nebula-runtime.
- Actions may run concurrently. Do not store mutable state in action struct fields. Use `StatefulAction` if state is required (it carries explicit state type).
- `Context` is always injected — never construct `ActionContext` directly inside an action.

## Key Decisions
- Action subtypes: `StatelessAction` (one-shot), `StatefulAction` (Continue/Break loop), `TriggerAction` (starts workflow), `ResourceAction` (branch-scoped DI setup/cleanup).
- `ActionDependencies` declares resource and credential requirements statically — the engine reads this at registration time.
- `ActionResult` carries both output data and flow-control intent (branch, wait, error).
- `ActionOutput` has 4 variants: inline JSON value, binary blob reference, deferred (async result), stream.
- `capability` module: `CredentialAccessor`, `ResourceAccessor`, `ActionLogger`, `ExecutionEmitter`, `TriggerScheduler` — these are the DI interfaces.

## Traps
- `#[derive(Action)]` requires **unit structs** (no fields). Config goes in a separate type injected as a dependency.
- `ActionError` distinguishes retryable from fatal. `ActionError::retryable(...)` vs `ActionError::fatal(...)` — the engine uses this to decide retry.
- `FnStatelessAction` / `stateless_fn()` for closure-based actions (testing and one-off use).

## Relations
- Depends on nebula-core, nebula-parameter (re-exports Field/Schema). Used by nebula-engine, nebula-runtime, nebula-sdk.
