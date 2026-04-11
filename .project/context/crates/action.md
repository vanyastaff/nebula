# nebula-action
Action trait hierarchy and execution contract. Defines **what** actions are; the
engine runs them from `nebula-runtime`. `ActionRegistry` lives there, not here.

## Subtypes
- `StatelessAction` — one-shot.
- `StatefulAction` — `Continue`/`Break` loop. `State: Serialize + DeserializeOwned + Clone`.
- `TriggerAction` — starts workflows from outside the graph. DX specializations: `WebhookAction` (push) + `PollAction` (pull).
- `ResourceAction` — branch-scoped DI, single `type Resource`.

## Dispatch
- `ActionHandler` (`#[non_exhaustive]`, `Clone`) — engine's sum type. Variants wrap `Arc<dyn XxxHandler>`.
- `{Stateless,Stateful,Trigger,Resource}Handler` — JSON-erased contracts. Typed authors write `impl XxxAction`; matching `XxxActionAdapter` bridges typed ⇄ JSON at registration.
- Canonical path: `nebula_action::{ActionHandler, StatelessHandler, IncomingEvent, ...}` from crate root. No `handler::X` alias surface.

## Credentials
- `CredentialGuard<S: Zeroize>` — only credential type re-exported here. Deref + Zeroize on drop, `!Serialize` (compile error in `Output`/`State`).
- `ctx.credential::<S>() -> CredentialGuard<S>` via `TypeId`. `ctx.credential_by_id(id)` for string-keyed access.
- `ActionDependencies` declares: `credential_keys`/`resource_keys` (typed, validated at registration) + `credential_types() -> Vec<TypeId>` (for `ScopedCredentialAccessor` sandboxing). Default empty.
- `#[derive(Action)]` + `#[action(credentials = [T1, T2])]` generates both. Duplicate types → compile error.

## Errors
- `ActionError::Validation { field: &'static str, reason: ValidationReason, detail: Option<String> }`. `field` is compile-time constant by type — cannot carry user input. `detail` is sanitized (control chars → `\uXXXX`, truncated to `MAX_VALIDATION_DETAIL = 256` B).
- `retryable(..)` vs `fatal(..)` decides retry. `ActionErrorExt` gives `.retryable()?`/`.fatal()?` on any `Result`.
- `RetryHintCode` (`#[non_exhaustive]`) — action-supplied retry hint. Attached via `retryable_with_hint`/`fatal_with_hint`. Distinct from `nebula_error::Classify::code()`.
- Payload is `Arc<anyhow::Error>` — full chain, cheap clone.

## Traps
- **State checkpointing.** `StatefulActionAdapter::execute` MUST flush `typed_state` back to JSON on both Ok and Err paths before propagating. A `Retryable` with mutated cursor that isn't flushed makes the engine replay — duplicated API calls, double charges, double emits. Only exception: `Validation` from input/state deserialization (typed state never existed). If state serialization itself fails on the error path, log and propagate the original error — masking breaks retry classification.
- **Webhook double-start.** `WebhookTriggerAdapter::start` rejects double-start with `Fatal` (read-lock pre-check + write-lock re-check with orphan `on_deactivate`). Silent re-registration leaks external webhook registrations. Guards scoped so they never cross `.await` (parking_lot is `!Send`).
- **Poll double-start.** `PollTriggerAdapter::start` rejects via `AtomicBool` + `compare_exchange(AcqRel, Acquire)`, cleared by `StartedGuard` RAII (defused, never `mem::forget`). Shared cursor would double-emit. `start()` blocks until cancellation — MUST be spawned.
- **Two trigger shapes.** `TriggerHandler::start` is either **setup-and-return** (webhook) or **run-until-cancelled** (poll). Sequentially awaiting multiple shape-2 triggers deadlocks. Always spawn.
- **Poll interval floor.** Clamped to `POLL_INTERVAL_FLOOR = 100 ms`. Tests must use ≥100 ms. Clamp logs one-shot warn via `ctx.logger` at `start()`. Per-cycle retryable/serialize/emit failures log via `ctx.logger` with per-kind `WarnThrottle` (30 s cooldown).
- **Event limits.** `IncomingEvent::try_new` enforces `DEFAULT_MAX_BODY_BYTES` (1 MiB) + `MAX_HEADER_COUNT` (128). Returns `DataLimitExceeded` / `Validation`. Use `try_new_with_limits` for providers needing larger caps. Header keys lowercased at construction — `header()` is O(1). Duplicate keys collapse to last value.
- **Batch error semantics.** `BatchAction::process_item` → `Fatal` aborts the batch. Use `Retryable` for per-item failures the batch should capture and skip past.
- **DX macro requirement.** `impl_paginated_action!`/`impl_batch_action!` generate the `StatefulAction` impl — forgetting the macro → `register_stateful()` fails. One macro per type.
- **Resource downcast.** `ResourceActionAdapter::cleanup` downcasts `Box<dyn Any>` — mismatch is an engine bug, returns `Fatal`.
- **In-memory poll cursor.** `PollAction::Cursor` resets on every `start()`. Cross-restart persistence is a runtime concern (post-v1).
- **`ctx.cancellation` / `ctx.emitter`** are `pub` fields on `TriggerContext` — known tech debt, tracked for an accessor refactor.

## Module layout (one domain per file)
`action.rs` (base) · `stateless.rs` · `stateful.rs` (+ `PaginatedAction`/`BatchAction` DX + `impl_*_action!` macros) · `trigger.rs` (base + `IncomingEvent` + `TriggerEventOutcome`) · `webhook.rs` (+ HMAC primitives via `subtle::ConstantTimeEq`) · `poll.rs` · `resource.rs` · `handler.rs` (`ActionHandler` enum + `AgentHandler` stub + cross-variant tests) · `error.rs` · `result.rs` · `context.rs` · `metadata.rs` · `output.rs` · `port.rs` · `dependency.rs` · `capability.rs` · `validation.rs` · `testing.rs`.

## Testing
- `TestContextBuilder` — `minimal()`, `with_credential_snapshot()`, `with_credential::<S>()`, `with_resource()`, `with_input()`, `build_trigger()`.
- `StatefulTestHarness<A>` / `TriggerTestHarness<A>` with `SpyEmitter` / `SpyScheduler`.
- Assertion macros (`assert_success!`, `assert_branch!`, …) match on real variants.
- Poll tests: `#[tokio::test(start_paused = true)]` + `tokio::time::advance` + `yield_now` (requires tokio `test-util`).

## Relations
Depends on `nebula-core`, `nebula-parameter`, `nebula-credential`.
Used by `nebula-engine`, `nebula-runtime`, `nebula-sdk`.
