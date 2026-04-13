# Architecture Decisions

## serde_json::Value as Universal Data Type
**Choice**: `serde_json::Value` everywhere — workflow data, action I/O, config, expressions
**Why**: Eliminates conversion layers; dates use ISO-8601, decimals use base64 convention.

## One-Way Layer Dependencies
**Choice**: Infra → Core → Business → Exec → API, enforced by `cargo deny`
**Why**: Prevents circular deps. Cross-cutting crates (log, config, resilience, eventbus, metrics, telemetry) are exempt.

## EventBus for Cross-Crate Signals
**Choice**: `EventBus<E>` for all cross-crate notifications
**Why**: Prevents circular deps (especially credential↔resource); keeps layers decoupled.

## InProcessSandbox Only (Phase 2)
**Choice**: Actions run in-process — no OS-process or WASM isolation
**Why**: Phase 3 target. Adding isolation now would require major runtime changes.

## AES-256-GCM for Credentials at Rest
**Choice**: AES-256-GCM; `SecretString` zeroizes on drop
**Why**: Credentials always encrypted before storage — no exceptions.

## Actions Use DI via Context
**Choice**: `Context` trait injects credentials, resources, logger
**Why**: Actions may run concurrently; testable without real infrastructure.

## NodeId Separate from ActionKey
**Choice**: `NodeId` = graph position; `ActionKey` = type identity
**Why**: Multiple nodes can run the same action. `NodeDefinition.action_key` is the binding.

## PostgreSQL + MemoryStorage
**Choice**: `MemoryStorage` for tests, `PostgresStorage` for production
**Why**: Tests never hit the database; `Storage` trait abstracts both.

## REST API
**Choice**: REST for CRUD, versioned at `/api/v1/`
**Why**: Simpler than GraphQL. WebSocket for real-time execution updates is planned but not yet implemented.

## Credential–Resource Integration via Typed Refs + Events
**Choice**: `CredentialRef<C>` (typed) / `ErasedCredentialRef` in `ResourceComponents`; `Pool<R>` stores credential state + `CredentialHandler<R::Instance>`; rotation via `CredentialRotationEvent` on `EventBus`
**Why**: Avoids circular dep between credential↔resource; `TypeId`-based refs were broken — typed refs enforce protocol at compile time. Resources subscribe via `rotation_subscriber()` and dispatch to affected pools by `CredentialId`.

## Channel Conventions
**Choice**: `mpsc` (bounded) for work queues; `broadcast` for status; `oneshot` for request/response; `RwLock` for shared mutable state
**Why**: Back-pressure via bounded mpsc prevents unbounded queue growth; broadcast decouples status consumers.

## Default Timeouts
**Choice**: HTTP 10 s · Database 5 s · General 30 s
**Why**: Explicit defaults prevent unbounded blocking; overridable via `ApiConfig`.

## No saga / transactional trait today
**Choice**: `nebula-action` has no saga or transactional-action DX trait. Consumers who need compensation model it as two separate actions wired via failure events.
**Why**: A real saga needs a rollback trigger + saga state store + compensation DAG. Until the engine has all three, any DX trait claiming "transactional" would be either dead code or misleading. A previous `TransactionalAction` attempt was deleted for exactly this reason — its `Pending → Executed → Compensated` state machine was unreachable past `Pending` under actual engine dispatch. The public surface stays honest: zero saga support until there's real saga support.

## `ControlAction` — adapter pattern, not blanket impl, not macro
**Context**: Nebula needs a DX family for flow-control nodes (If, Switch, Router, Filter, NoOp, Stop, Fail). Researched 23 workflow platforms (n8n, Make, Zapier, Kestra, Airflow, Argo, Prefect, Dagster, Node-RED, Inngest, Temporal, Windmill, Pipedream, AWS Step Functions, Azure Logic Apps, GCP Workflows, Power Automate, Tines, LangGraph, GitHub Actions, Activepieces, Flyte, Metaflow) and catalogued ~80 flow-control nodes into 34 semantic families. 7 families fit the control category; the other 27 are either existing DX patterns (`InteractiveAction`, `PaginatedAction`/`BatchAction`, future `DelayAction`/`LoopAction`), engine/scheduler concerns (merge/parallel/subflow/matrix, trigger_rule), or deployment config (concurrency gate, environment gate).

**Choice**:
1. `ControlAction` — public, non-sealed trait in `crates/action/src/control.rs`. Community crates may implement it.
2. Desugar to `StatelessHandler` via explicit `ControlActionAdapter<A: ControlAction>`, not blanket `impl<T: ControlAction> StatelessAction`. Mirrors the existing `PollTriggerAdapter<A: PollAction>` and `WebhookTriggerAdapter<A: WebhookAction>` pattern. Coherence lives on the concrete adapter type, not on `StatelessAction`, so any number of DX families can coexist without a "one-blanket-per-core-trait" budget problem.
3. Unified `ControlOutcome` enum, `#[non_exhaustive]`, 5 variants: `Branch`, `Route`, `Pass`, `Drop`, `Terminate`. Rationale — type-safe public contract that hides `ActionResult::Wait`/`Retry`/`Continue` from flow-control plugin authors without needing a compile-time subset check.
4. Concrete 7 nodes live **outside** `nebula-action` — in a separate crate. Exact crate name and location deferred: this is a downstream packaging decision that does not affect the trait contract. `nebula-action` stays pure trait surface regardless.
5. No separate `control_ports()` method on the trait — ports declared through standard `ActionMetadata`, same as every other DX family.
6. Author trait uses native RPITIT (`fn evaluate(...) -> impl Future<Output = ...> + Send`); handler trait (`StatelessHandler`) stays `#[async_trait]` for dyn compat. Adapter is the bridge. No `#[async_trait]` on `ControlAction`.

**Prerequisites** (block implementation — correctness bugs in the current `ActionResult` shape):
- `ActionResult::Drop { reason }` — for Filter semantics. Current `Skip` skips the whole downstream subgraph; Filter wants to drop one item while leaving the branch alive.
- `ActionResult::Terminate { reason }` + `ExecutionTerminationReason::ExplicitStop { by_node }` in `nebula-execution` — for Stop/Fail semantics. Current `Skip` does not terminate execution in parallel branches, so `StopAction` desugared to `Skip` is silently wrong.
- `ActionCategory` enum added to `ActionMetadata`, so UI editor, workflow validator, and audit log can distinguish control nodes without type-level knowledge of concrete implementations.
- `MultiOutput` join semantics documented in `result.rs` docstring — needed to pin down what `RouterAction` in all-match mode actually means to downstream nodes.

**Rejected alternatives**:
- **Declarative macro** (tech-lead proposal): rejected because it blocks community extensibility. Macros work only for built-in nodes; external plugin crates cannot build new control primitives on a macro.
- **Seven separate sealed traits** (architect proposal): rejected for the same reason — sealed traits block community implementation by definition.
- **New 6th core trait in `ActionHandler` enum**: rejected as unnecessary. Adapter + existing `StatelessHandler` dispatch is enough. Engine, runtime, and `ActionHandler` enum stay untouched.
- **Blanket `impl<T: ControlAction> StatelessAction for T`**: rejected in favour of adapter pattern. Blanket would burn the sole coherence slot on `StatelessAction`, forcing every future DX family on Stateless to go through newtype wrappers. Adapter avoids the budget problem entirely.

**Deferred to post-v1**:
- `ControlAction::validate()` hook — debug-assert in adapter vs explicit method vs author responsibility. Decide after v1 is in use and real validation bugs surface on live plugins.
- Error-as-artifact pattern (Metaflow `@catch(var=)`) — interesting DX, requires `ActionContext::previous_error()` and scheduler cooperation to propagate error as data to the next step. Out of scope for v1.
- Explicit `LoopAction` / `DelayAction` / `ScheduleUntilAction` DX over `StatefulAction` — separate work, independent of `ControlAction`.

**References**: `crates/action/src/poll.rs` and `crates/action/src/webhook.rs` (adapter pattern prior art); `crates/action/src/port.rs` (existing port model — `SupportPort`, `DynamicPort` already cover aux ports and config-driven outputs); `docs/plans/2026-04-08-action-v2-spec.md` (DX layer philosophy); research conversation 2026-04-13.
