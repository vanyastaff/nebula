# nebula-action
Action trait hierarchy and execution contract — Ports & Drivers architecture.

## Invariants
- Defines **what** actions are, not how the engine runs them. Concrete execution environments are in nebula-runtime.
- Actions may run concurrently. No mutable state in action struct fields — use `StatefulAction` for stateful loops.
- `Context` is always injected — never construct `ActionContext` directly inside an action.

## Key Decisions
- Action subtypes: `StatelessAction` (one-shot), `StatefulAction` (Continue/Break loop), `TriggerAction` (starts workflow), `ResourceAction` (branch-scoped DI setup/cleanup).
- `ActionDependencies` has two complementary pairs: `credential`/`resources` (trait-object, runtime injection) and `credential_keys`/`resource_keys` (typed keys, engine validation at registration). All four default to empty — no migration needed.
- `ActionRegistry` (`registry.rs`): keyed by `ActionKey`, supports multiple versions per key. `get()` → latest, `get_versioned("major.minor")` → specific. Not `Sync` — wrap in `Arc<RwLock<_>>` for shared access.
- `credential_typed::<S>()` on `ActionContext`/`TriggerContext` consumes snapshot via `into_project::<S>()`, maps `SnapshotError` → `ActionError::Fatal`. Primary typed credential access path.

## Traps
- `#[derive(Action)]` requires **unit structs** (no fields). Config goes in a separate injected type.
- `ActionError::retryable(...)` vs `ActionError::fatal(...)` — engine uses this to decide retry.
- `FnStatelessAction` / `stateless_fn()` for closure-based actions (testing and one-off use).

## Relations
- Depends on nebula-core, nebula-parameter. Used by nebula-engine, nebula-runtime, nebula-sdk.

<!-- reviewed: 2026-04-07 — added ActionRegistry + typed credential_keys/resource_keys to ActionDependencies -->
