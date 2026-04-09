# nebula-action
Action trait hierarchy and execution contract — Ports & Drivers architecture.

## Invariants
- Defines **what** actions are, not how the engine runs them. Concrete execution environments are in nebula-runtime.
- Actions may run concurrently. No mutable state in action struct fields — use `StatefulAction` for stateful loops.
- `Context` is always injected — never construct `ActionContext` directly inside an action.

## Key Decisions
- Action subtypes: `StatelessAction` (one-shot), `StatefulAction` (Continue/Break loop), `TriggerAction` (starts workflow), `ResourceAction` (branch-scoped DI setup/cleanup).
- `ActionDependencies` has two complementary pairs: `credential`/`resources` (trait-object, runtime injection) and `credential_keys`/`resource_keys` (typed keys, engine validation at registration). Plus `credential_types()` → `Vec<TypeId>` for `ScopedCredentialAccessor` sandboxing. All five default to empty — no migration needed.
- `#[derive(Action)]` with `#[action(credential = T)]` or `#[action(credentials = [T1, T2])]` generates both `credential()` and `credential_types()`. Duplicate credential types in the attribute produce a compile error.
- `ActionRegistry` (`registry.rs`): keyed by `ActionKey`, supports multiple versions per key. `get()` → latest, `get_versioned(&InterfaceVersion)` → specific. `Send + Sync` — use `Arc<ActionRegistry>` for read-only sharing, `Arc<RwLock<_>>` for mutation after sharing.
- `credential_by_type::<S>()` is the primary typed credential access path (returns `CredentialGuard<S>`). Legacy `credential_typed::<S>(id)` (string-based, consumes snapshot via `into_project::<S>()`, maps `SnapshotError` → `ActionError::Fatal`) preserved for backward compat.
- `ErrorCode` enum (8 variants, `#[non_exhaustive]`) on `ActionError::Retryable` and `Fatal` — machine-readable classification for engine retry decisions (RateLimited, AuthExpired, UpstreamTimeout, etc.).
- `ActionResultExt` trait — `.retryable()?` and `.fatal()?` ergonomic conversion on any `Result<T, E>`. Also `_with_code()` variants for ErrorCode attachment.
- Error field: `Arc<anyhow::Error>` — preserves full error chain, Clone via Arc. Factory methods accept `impl Display + Debug + Send + Sync + 'static`.

- `CredentialGuard<S: Zeroize>` — re-exported from `nebula-credential`. Deref + Zeroize on drop + !Serialize. Constructed in context methods via `CredentialGuard::new()`.
- `credential_by_type::<S>()` on `ActionContext`/`TriggerContext` — type-based credential access via TypeId. Returns `CredentialGuard<S>`. Existing `credential_typed()` (string-based) kept for backward compat.
- `CredentialAccessor`, `ScopedCredentialAccessor`, `NoopCredentialAccessor`, `CredentialAccessError` — canonical home is `nebula-credential`; re-exported from `nebula-action::capability` for backward compat. `From<CredentialAccessError> for ActionError` maps `AccessDenied` to `SandboxViolation`, others to `Fatal`.
- `#[derive(Action)]` now works on structs with fields (not just unit structs) — enables `type Input = Self` pattern.

- `ActionHandler` enum (5 variants: Stateless, Stateful, Trigger, Resource, Agent, `#[non_exhaustive]`) — engine match-dispatches. Replaces deprecated `InternalHandler`.
- 5 handler traits: `StatelessHandler`, `StatefulHandler`, `TriggerHandler`, `ResourceHandler`, `AgentHandler` (stub).
- 4 adapters: `StatelessActionAdapter`, `StatefulActionAdapter`, `TriggerActionAdapter`, `ResourceActionAdapter` — bridge typed traits to JSON-erased handlers.
- `ActionRegistry` stores `ActionHandler`, has typed `register_stateless/stateful/trigger/resource` convenience methods.
- `StatefulAction::State` now requires `Serialize + DeserializeOwned + Clone`. `init_state()` is required. `StatefulHandler::init_state()` returns `Result<Value, ActionError>` (fallible).

## Testing Infrastructure (Phase 5)
- Assertion macros (`assert_success!`, `assert_branch!`, etc.) via `#[macro_export]` — match on real `ActionResult`/`ActionError` variants. Import via `use crate::assert_*` in tests.
- `TestContextBuilder`: `minimal()`, `with_credential_snapshot()`, `with_credential::<S>()` (type-based), `with_resource()`, `with_input()`, `build_trigger()`.
- `StatefulTestHarness<A>`: wraps `StatefulAction`, serializes/deserializes state between `step()` calls, exposes `state::<S>()` and `state_json()`.
- `TriggerTestHarness<A>`: wraps `TriggerAction` with `SpyEmitter`/`SpyScheduler`, exposes `emitted()`, `scheduled()`, `start()`/`stop()`.
- `SpyEmitter` / `SpyScheduler`: test doubles capturing `emit()` and `schedule_after()` calls.
- `TestResourceAccessor` uses `remove()` — a resource can only be acquired once per test (mirrors real acquire semantics).

## Traps
- `ActionError::retryable(...)` vs `ActionError::fatal(...)` — engine uses this to decide retry. Use `ActionResultExt` for ergonomic `.retryable()?` / `.fatal()?`.
- `FnStatelessAction` / `stateless_fn()` for closure-based actions (testing and one-off use).
- `CredentialGuard` does NOT impl Serialize — compile error if put in Output/State types. By design.
- `InternalHandler` is deprecated — use `ActionHandler` enum. Downstream crates use `#![allow(deprecated)]` during migration.
- `ResourceActionAdapter::cleanup` downcasts `Box<dyn Any>` — returns `ActionError::Fatal` on type mismatch during cleanup downcast.

## Relations
- Depends on nebula-core, nebula-parameter, nebula-credential. Used by nebula-engine, nebula-runtime, nebula-sdk.

<!-- reviewed: 2026-04-08 — CredentialGuard moved to nebula-credential, re-exported from action -->