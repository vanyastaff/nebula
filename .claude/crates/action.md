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
- `credential::<S>()` is the primary typed credential access path (returns `CredentialGuard<S>`). Renamed from `credential_by_type::<S>()` in Phase 2b (deprecated alias kept). `credential_by_id(id)` for string-based access (old `credential(id)` removed — name clash). `has_credential_id(id)` replaces `has_credential(id)` (deprecated alias kept). Legacy `credential_typed::<S>(id)` (string-based, consumes snapshot via `into_project::<S>()`, maps `SnapshotError` → `ActionError::Fatal`) preserved for backward compat.
- `ErrorCode` enum (8 variants, `#[non_exhaustive]`) on `ActionError::Retryable` and `Fatal` — machine-readable classification for engine retry decisions (RateLimited, AuthExpired, UpstreamTimeout, etc.).
- `ActionResultExt` trait — `.retryable()?` and `.fatal()?` ergonomic conversion on any `Result<T, E>`. Also `_with_code()` variants for ErrorCode attachment.
- Error field: `Arc<anyhow::Error>` — preserves full error chain, Clone via Arc. Factory methods accept `impl Display + Debug + Send + Sync + 'static`.

- `CredentialGuard<S: Zeroize>` — re-exported from `nebula-credential`. Deref + Zeroize on drop + !Serialize. Constructed in context methods via `CredentialGuard::new()`.
- `credential::<S>()` on `ActionContext`/`TriggerContext` — type-based credential access via TypeId. Returns `CredentialGuard<S>`. Existing `credential_typed()` (string-based) and deprecated `credential_by_type()` kept for backward compat.
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

- `stateful.rs`: core `StatefulAction` (moved from `execution.rs`) + DX traits (`PaginatedAction`, `BatchAction`, `TransactionalAction`) + `macro_rules!` macros (`impl_paginated_action!`, `impl_batch_action!`, `impl_transactional_action!`). Macros generate `impl StatefulAction for $ty` — no blanket impls (Rust coherence forbids multiple). Engine never sees DX types. `execution.rs` re-exports `StatefulAction` for backward compat.
- `migrate_state(old: Value) -> Option<Self::State>` — default method on `StatefulAction`. Adapter calls on state deser failure. Returns `None` by default (error propagated).
- `ActionResult::continue_with()`, `break_completed()`, `break_with_reason()`, `continue_with_delay()` — convenience constructors for stateful iteration results.

- `trigger.rs`: DX traits for TriggerAction — `WebhookAction` (register/handle/unregister lifecycle) + `PollAction` (blocking poll loop with in-memory cursor). Typed adapters (`WebhookTriggerAdapter`, `PollTriggerAdapter`) implement `TriggerHandler` directly. Registry convenience methods: `register_webhook()`, `register_poll()`.
- `IncomingEvent` — transport-agnostic event struct (body bytes + headers map + source). Lives in `handler.rs` (not `trigger.rs`) to avoid circular imports — re-exported from `trigger.rs`.
- `TriggerEventOutcome` enum (Skip/Emit/EmitMany) on `TriggerHandler` — universal event ingress. `accepts_events()` + `handle_event()` with default error. `handle_event` takes typed `IncomingEvent` directly (NOT `Value`) — no JSON round-trip, no body bloat.
- `WebhookTriggerAdapter` stores state as `RwLock<Option<Arc<State>>>`. `handle_event` clones the `Arc` under read lock and releases BEFORE await (prevents deadlock with concurrent start/stop). `handle_event` before `start()` returns Fatal error.
- `PollTriggerAdapter::start()` blocks in a `tokio::select!` loop until cancellation. Fatal errors stop the loop, Retryable errors skip the cycle, emit failures silently dropped.

## Traps
- `ActionError::retryable(...)` vs `ActionError::fatal(...)` — engine uses this to decide retry. Use `ActionResultExt` for ergonomic `.retryable()?` / `.fatal()?`.
- `FnStatelessAction` / `stateless_fn()` for closure-based actions (testing and one-off use). `FnStatelessCtxAction` / `stateless_ctx_fn()` for closures that need `ActionContext` (credentials, resources, logger). Use `.with_context(ctx)` to inject capabilities.
- `CredentialGuard` does NOT impl Serialize — compile error if put in Output/State types. By design.
- `InternalHandler` is deprecated — use `ActionHandler` enum. Downstream crates use `#![allow(deprecated)]` during migration.
- `ResourceActionAdapter::cleanup` downcasts `Box<dyn Any>` — returns `ActionError::Fatal` on type mismatch during cleanup downcast.
- Must call `impl_paginated_action!(MyType)` after `impl PaginatedAction for MyType` — the macro generates the `StatefulAction` impl. Forgetting the macro = type won't work with `register_stateful()`.
- A type cannot use two DX macros (e.g., both `impl_paginated_action!` and `impl_batch_action!`) — duplicate `StatefulAction` impl error. Choose one pattern per type.
- `BatchAction::process_item` returning `ActionError::Fatal` aborts the entire batch. Use `ActionError::Retryable` for per-item errors that should be captured and continued.
- `PollTriggerAdapter::start()` blocks until cancellation — engine MUST spawn it in a task. Tests use `#[tokio::test(start_paused = true)]` + `tokio::time::advance` + `yield_now` for determinism. Requires `tokio` `test-util` feature.
- `WebhookTriggerAdapter::handle_event` before `start()` returns `ActionError::Fatal` — webhook layer must ensure trigger is started before routing events.
- `IncomingEvent` is in `handler.rs` not `trigger.rs` — re-exported from both for public API. Both `nebula_action::handler::IncomingEvent` and `nebula_action::trigger::IncomingEvent` work.
- `ctx.cancellation` and `ctx.emitter` accessed as pub fields in PollTriggerAdapter — known tech debt, should be methods. Tracked for TriggerContext refactor.
- `PollAction::Cursor` is in-memory only — resets to `Default` on every `start()`. Cross-restart persistence requires runtime storage integration (post-v1).

## Relations
- Depends on nebula-core, nebula-parameter, nebula-credential. Used by nebula-engine, nebula-runtime, nebula-sdk.

<!-- reviewed: 2026-04-09 — Phase 7 DX trigger types (WebhookAction, PollAction, IncomingEvent, TriggerEventOutcome) -->