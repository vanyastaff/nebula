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
- `ActionRegistry` lives in **`nebula-runtime`** (Phase 7.5), not in `nebula-action`. The protocol crate stays free of execution concerns.
- `credential::<S>()` is the primary typed credential access path (returns `CredentialGuard<S>`). Renamed from `credential_by_type::<S>()` in Phase 2b (deprecated alias kept). `credential_by_id(id)` for string-based access (old `credential(id)` removed — name clash). `has_credential_id(id)` replaces `has_credential(id)` (deprecated alias kept). Legacy `credential_typed::<S>(id)` (string-based, consumes snapshot via `into_project::<S>()`, maps `SnapshotError` → `ActionError::Fatal`) preserved for backward compat.
- `ErrorCode` enum (8 variants, `#[non_exhaustive]`) on `ActionError::Retryable` and `Fatal` — machine-readable classification for engine retry decisions (RateLimited, AuthExpired, UpstreamTimeout, etc.).
- `ActionErrorExt` trait (in `error.rs`) — `.retryable()?` and `.fatal()?` ergonomic conversion on any `Result<T, E>`. Also `_with_code()` variants for ErrorCode attachment. Renamed from `ActionResultExt` on 2026-04-10 — name reflects that the trait produces `ActionError`, not `ActionResult`.
- Error field: `Arc<anyhow::Error>` — preserves full error chain, Clone via Arc. Factory methods accept `impl Display + Debug + Send + Sync + 'static`.

- `CredentialGuard<S: Zeroize>` — the **only** credential type re-exported from `nebula-action` (via `pub use nebula_credential::CredentialGuard` at `lib.rs`). Deref + Zeroize on drop + !Serialize. Constructed in context methods via `CredentialGuard::new()`.
- `credential::<S>()` on `ActionContext`/`TriggerContext` — type-based credential access via TypeId. Returns `CredentialGuard<S>`.
- `CredentialAccessor`, `ScopedCredentialAccessor`, `NoopCredentialAccessor`, `CredentialAccessError`, `default_credential_accessor` — NOT re-exported from `nebula-action` (purged 2026-04-10). Import directly from `nebula_credential` if building an `ActionContext` manually. Internally, `context.rs` / `testing.rs` use `nebula_credential::{CredentialAccessor, ...}` directly. `From<CredentialAccessError> for ActionError` still exists in `error.rs` and maps `AccessDenied` → `SandboxViolation`, others → `Fatal`.
- `#[derive(Action)]` now works on structs with fields (not just unit structs) — enables `type Input = Self` pattern.

- `ActionHandler` enum (5 variants: Stateless, Stateful, Trigger, Resource, Agent, `#[non_exhaustive]`) — engine match-dispatches. Replaces removed `InternalHandler`. Derives `Clone` (all variants are `Arc<dyn ...>`).
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

## Module layout (reorganised 2026-04-10)

Every action family lives in one file containing both the core trait and its DX adapters. The former grab-bag `execution.rs` / `authoring.rs` / `ext.rs` / `scoped.rs` files were deleted — no shims left behind.

- `action.rs` — base `Action` trait (identity + metadata). 21 lines, intentionally left as its own file.
- `stateless.rs` — `StatelessAction` trait + function-backed adapters (`FnStatelessAction`, `FnStatelessCtxAction`, `stateless_fn`, `stateless_ctx_fn`). Previously split between `execution.rs` and `authoring.rs`.
- `stateful.rs` — `StatefulAction` core + DX (`PaginatedAction`, `BatchAction`, `TransactionalAction`) + `macro_rules!` macros (`impl_paginated_action!`, `impl_batch_action!`, `impl_transactional_action!`). Macros generate `impl StatefulAction for $ty` — no blanket impls (Rust coherence forbids multiple). Engine never sees DX types.
- `trigger.rs` — `TriggerAction` core (moved from `execution.rs`) + DX (`WebhookAction`, `PollAction`). Typed adapters (`WebhookTriggerAdapter`, `PollTriggerAdapter`) implement `TriggerHandler` directly. Registry convenience methods: `register_webhook()`, `register_poll()`.
- `resource.rs` — `ResourceAction` trait (moved from `execution.rs`). Graph-level DI: configure/cleanup.
- `error.rs` — `ActionError` + `ErrorCode` + `ActionErrorExt` (merged from old `ext.rs`).
- `result.rs` — `ActionResult` variants, `BranchKey`, `BreakReason`, `PortKey`, `WaitCondition`, `continue_with()`, `break_completed()`, etc.
- `handler.rs` — `ActionHandler` enum + `{Stateless,Stateful,Trigger,Resource,Agent}Handler` traits + adapters.

- `migrate_state(old: Value) -> Option<Self::State>` — default method on `StatefulAction`. Adapter calls on state deser failure. Returns `None` by default (error propagated).
- `ActionResult::continue_with()`, `break_completed()`, `break_with_reason()`, `continue_with_delay()` — convenience constructors for stateful iteration results.
- `IncomingEvent` — transport-agnostic event struct (body bytes + headers map + source). Lives in `handler.rs` (not `trigger.rs`) to avoid circular imports — re-exported from `trigger.rs`.
- `TriggerEventOutcome` enum (Skip/Emit/EmitMany) on `TriggerHandler` — universal event ingress. `accepts_events()` + `handle_event()` with default error. `handle_event` takes typed `IncomingEvent` directly (NOT `Value`) — no JSON round-trip, no body bloat.
- `WebhookTriggerAdapter` stores state as `RwLock<Option<Arc<State>>>`. `handle_event` clones the `Arc` under read lock and releases BEFORE await (prevents deadlock with concurrent start/stop). `handle_event` before `start()` returns Fatal error.
- `PollTriggerAdapter::start()` blocks in a `tokio::select!` loop until cancellation. Fatal errors stop the loop, Retryable errors skip the cycle, emit failures silently dropped.

- **Phase 7.5 (2026-04-09):** `ActionRegistry` moved to `nebula-runtime` (was in `nebula-action`). Protocol crate stays free of execution concerns and `dashmap` dependency. `use nebula_runtime::ActionRegistry`.
- **`StatefulHandler::execute` borrows input** (`&Value`, not owned `Value`) — runtime stateful loop reuses input across iterations without per-iteration cloning. Adapter clones once internally for typed deserialization. User-level `StatefulAction::execute` is unchanged.
- **`InternalHandler` deleted entirely.** All execution flows through `ActionHandler` enum dispatch in `nebula_runtime::ActionRuntime::run_handler`.
- **`ActionHandler` enum is now `Clone`** — all variants wrap `Arc<dyn ...>`, so cloning is cheap pointer copies. Required for owned-tuple lookups from the registry.

## Traps
- `ActionError::retryable(...)` vs `ActionError::fatal(...)` — engine uses this to decide retry. Use `ActionErrorExt` for ergonomic `.retryable()?` / `.fatal()?`.
- `FnStatelessAction` / `stateless_fn()` for closure-based actions (testing and one-off use). `FnStatelessCtxAction` / `stateless_ctx_fn()` for closures that need `ActionContext` (credentials, resources, logger). Use `.with_context(ctx)` to inject capabilities.
- `CredentialGuard` does NOT impl Serialize — compile error if put in Output/State types. By design.
- `InternalHandler` was deleted in Phase 7.5. Use `ActionHandler` enum exclusively.
- `ResourceActionAdapter::cleanup` downcasts `Box<dyn Any>` — returns `ActionError::Fatal` on type mismatch during cleanup downcast.
- Must call `impl_paginated_action!(MyType)` after `impl PaginatedAction for MyType` — the macro generates the `StatefulAction` impl. Forgetting the macro = type won't work with `register_stateful()`.
- A type cannot use two DX macros (e.g., both `impl_paginated_action!` and `impl_batch_action!`) — duplicate `StatefulAction` impl error. Choose one pattern per type.
- `BatchAction::process_item` returning `ActionError::Fatal` aborts the entire batch. Use `ActionError::Retryable` for per-item errors that should be captured and continued.
- `PollTriggerAdapter::start()` blocks until cancellation — engine MUST spawn it in a task. Tests use `#[tokio::test(start_paused = true)]` + `tokio::time::advance` + `yield_now` for determinism. Requires `tokio` `test-util` feature.
- `StatefulActionAdapter::execute` MUST checkpoint typed_state back to the JSON state on BOTH `Ok` and `Err` paths before propagating. A Retryable with mutated cursor/counter must be flushed, or the engine replays completed work on retry — duplicated API calls, double charges, double emits. The only exception is Validation raised during input/state deserialization (typed_state never existed). If state serialization itself fails on the error path, the adapter logs via `tracing::error!` and propagates the original action error — masking it would break retry classification.
- `WebhookTriggerAdapter::handle_event` before `start()` returns `ActionError::Fatal` — webhook layer must ensure trigger is started before routing events.
- `IncomingEvent` is in `handler.rs` not `trigger.rs` — re-exported from both for public API. Both `nebula_action::handler::IncomingEvent` and `nebula_action::trigger::IncomingEvent` work.
- `ctx.cancellation` and `ctx.emitter` accessed as pub fields in PollTriggerAdapter — known tech debt, should be methods. Tracked for TriggerContext refactor.
- `PollAction::Cursor` is in-memory only — resets to `Default` on every `start()`. Cross-restart persistence requires runtime storage integration (post-v1).
- `ActionRegistry` is in `nebula-runtime`, NOT `nebula-action`. Importing from `nebula_action::ActionRegistry` will fail to compile.

## Relations
- Depends on nebula-core, nebula-parameter, nebula-credential. Used by nebula-engine, nebula-runtime, nebula-sdk.

<!-- reviewed: 2026-04-10 — module layout cleanup: execution.rs/authoring.rs/ext.rs/scoped.rs deleted; stateless/trigger/resource split into own files; ActionResultExt → ActionErrorExt in error.rs; credential re-exports purged (only CredentialGuard remains in public API) -->
<!-- reviewed: 2026-04-10 — A1: StatefulActionAdapter::execute checkpoints state on both Ok and Err paths; serde failure on error path logs via tracing and propagates original action error. StatefulHandler::execute doc updated with "State checkpointing" invariant. Added tracing as direct dep. -->