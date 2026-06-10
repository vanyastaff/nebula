---
name: nebula-action
role: Action Trait Family + Execution Policy Metadata (Ports & Adapters)
status: frontier
last-reviewed: 2026-04-29
canon-invariants: [L1-3.5, L2-11.3, L2-13.4, L2-13.5]
related: [nebula-core, nebula-schema, nebula-credential, nebula-resource, nebula-resilience, nebula-plugin]
---

# nebula-action

## Purpose

Workflow nodes need a typed contract between "what this step does" and "how the engine orchestrates it." Without one, every action re-invents credential plumbing, retry folklore, and checkpoint placement — and the engine cannot enforce guarantees across them. `nebula-action` defines that contract: a trait family (`StatelessAction`, `StatefulAction`, `TriggerAction`, `ResourceAction` and DX specializations) that determines iteration semantics, trigger lifecycle, and graph-scoped resource supply; plus `ActionMetadata` that carries the static descriptor (key, ports, parameters, isolation, category) the engine uses for discovery, validation, and dispatch. Action authors describe what their node does via slot-binding fields and a typed `Self::Input`; the engine wires, validates, and executes.

## Role

**Action Trait Family + Execution Policy Metadata (Ports & Adapters pattern)**. Core types and traits live here; the engine dispatches actions **in-process** (`InProcessSandbox`). Process/WASM isolation is a non-goal (ADR-0091, `docs/PRODUCT_CANON.md` §12.6).

Pattern inspiration: *Ports & Adapters / Hexagonal Architecture* — action authors program to traits; the engine wires driver adapters. Adding a new trait family requires a canon revision (§3.5, §0.2).

## Public API (v4 — M6 / dependency redesign, 2026-04-29)

The v4 surface lands per ADR-0042 (binding mechanism), ADR-0043 (dependency declaration DX), ADR-0044 (Resource::Credential supersession), and ADR-0045 (EventTrigger scope deferral). Phase 0–10 of the M6 plan.

### Base `Action` trait — `Sized + type Input/Output + static fns`

```rust
pub trait Action: Sized + Send + Sync + 'static {
    type Input:  HasSchema + DeserializeOwned + Send + Sync;
    type Output: HasSchema + Serialize         + Send + Sync;

    fn metadata()       -> &'static ActionMetadata;
    fn dependencies()   -> &'static Dependencies;  // slot-binding metadata
}
// No schema method — the `Input`/`Output: HasSchema` bound is the single
// source of truth; read it via `nebula_schema::schema_of::<A::Input>()`
// (ADR-0052 P3).
```

`Action` is **not object-safe** — `dyn Action` will not compile. Engine dispatch goes through `ActionFactory` + `ErasedAction` (see below).

### Sub-traits — execution shapes inherit `<Self as Action>::Input/Output`

| Trait | Shape | Method |
|---|---|---|
| `StatelessAction` | one-shot pure | `async fn execute(&self, input: Self::Input, ctx) -> Result<ActionResult<Self::Output>, ActionError>` |
| `StatefulAction` | iterative | `async fn execute(&self, input, state: &mut Self::State, ctx) -> Result<ActionResult<Self::Output>, ActionError>` |
| `TriggerAction` | start/stop trigger | `async fn start(&self, input, scheduler, ctx)` + `async fn stop(&self)` |
| `ResourceAction` | scoped resource provider | `Output = ResourceProduces<Self::Resource>`; `configure` / `cleanup` |
| `ControlAction` | flow-control (If, Switch, NoOp, Stop, …) | desugared to a stateless surface |
| `PaginatedAction` / `BatchAction` | DX over `StatefulAction` | cursor / chunk patterns |
| `WebhookAction` / `PollAction` | DX over `TriggerAction` | webhook + interval polling |

### Slot-binding fields — `#[resource]` / `#[credential]` per-field attrs

Action structs hold **only slot fields** (resources + credentials). User-facing form data lives on `Self::Input` (a separate `#[derive(Schema, Deserialize)]` companion struct).

```rust
use nebula_action::{Action, StatelessAction};
use nebula_credential::CredentialGuard;
use nebula_resource::ResourceGuard;
use nebula_schema::Schema;
use serde::{Deserialize, Serialize};

#[derive(Schema, Deserialize)]
struct SendTelegramInput {
    #[field(label = "Chat ID")]
    #[validate(required)]
    chat_id: i64,

    #[field(label = "Text")]
    #[validate(required, length(max = 4096))]
    text: String,
}

#[derive(Schema, Serialize)]
struct MessageId(i64);

#[derive(Action)]
#[action(
    key = "telegram.send",
    version = "1.0",
    input  = SendTelegramInput,
    output = MessageId,
)]
struct SendTelegram {
    #[resource(key = "bot")]
    bot: ResourceGuard<TelegramBot>,
    #[credential(key = "auth")]
    token: CredentialGuard<<TelegramCredential as nebula_credential::Credential>::Scheme>,
}

impl StatelessAction for SendTelegram {
    async fn execute(
        &self,
        input: SendTelegramInput,
        ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<nebula_action::ActionResult<MessageId>, nebula_action::ActionError> {
        let id = self.bot.send(input.chat_id, &input.text, &self.token).await?;
        Ok(nebula_action::ActionResult::ok(MessageId(id)))
    }
}
```

**Why slots-only on `Self`?** Eliminates `self.text` vs `input.text` ambiguity at compile time. `Self` carries deps; `Self::Input` carries form data. Single source of truth per field.

#### Field-type matrix

| Field type | Semantics |
|---|---|
| `ResourceGuard<R>` / `CredentialGuard<C::Scheme>` | required + eager |
| `Option<ResourceGuard<R>>` / `Option<CredentialGuard<C::Scheme>>` | optional + eager |
| `Lazy<ResourceGuard<R>>` / `Lazy<CredentialGuard<C::Scheme>>` | required + lazy (`.get(ctx).await`) |
| `Option<Lazy<…>>` | optional + lazy |

`Lazy<X>` uses `nebula_core::sync::Lazy` (cancel-safe `tokio::sync::OnceCell`).

> **Note** — `#[credential]` slot fields hold `CredentialGuard<C::Scheme>` (the projected auth scheme), not `CredentialGuard<C>` (the credential type). The framework projects state→scheme before populating the slot. The `key` attribute names the slot; binding to a concrete `CredentialId` per workflow node uses the ADR-0042 hybrid mechanism (default = slot key, explicit override via `node.slot_bindings`).

### `FromWorkflowNode` async factory

Every concrete action also implements `FromWorkflowNode`. The engine calls it once per dispatch to resolve slot bindings.

```rust
pub trait FromWorkflowNode: Sized + Send + 'static {
    type Error: Send;

    fn from_workflow_node<'a>(
        node: &'a NodeDefinition,
        ctx:  &'a dyn ActionContext,
    ) -> impl Future<Output = Result<Self, Self::Error>> + Send + 'a;
}
```

`#[derive(Action)]` emits the body — read `node.resource_binding(slot)` / `node.credential_binding(slot)` (falling back to the slot's `default_id`), call `ctx.acquire_resource_by_id::<R>(id)` / `ctx.resolve_credential_by_id::<C>(id)`, assemble `Self`. Plugin authors never write the body by hand.

### Engine-side dispatch — `ActionFactory` + `ErasedAction`

Because `Action: Sized` is not object-safe, the engine's registry holds `Arc<dyn ActionFactory>` (object-safe, generic over factory variant) and dispatches through `Box<dyn ErasedXxx>` per-variant trait objects:

| Erased trait | Mirrors |
|---|---|
| `ErasedStateless` | `StatelessHandler` (legacy) |
| `ErasedStateful`  | `StatefulHandler` |
| `ErasedTrigger`   | `TriggerHandler` |
| `ErasedResource`  | `ResourceHandler` |
| `ErasedControl`   | dyn-erased control flow |

Generic factories (`GenericStatelessFactory<A>`, `GenericStatefulFactory<A>`, …) wrap any `A: Action + FromWorkflowNode + StatelessAction` (etc.) into an `ActionFactory` automatically — see `crates/action/src/factory.rs`.

### `ResourceProduces<R>` marker

`ResourceAction` is required to set `type Output = ResourceProduces<<Self as ResourceAction>::Resource>`. The marker carries the topology tag for catalog/UI scope-binding visualizations and produces an empty schema (resource action outputs are graph-side effects, not data).

### Other public API

- `ActionMetadata`, `ActionMetadataBuilder` — static type descriptor (key, version, ports, isolation, category).
- `ActionResult` — execution result with flow-control intent (Success, Skip, Branch, Wait, Stop, Fail).
- `ActionOutput` — first-class output type: inline value, blob ref, stream.
- `ActionError`, `RetryHintCode` — typed error distinguishing retryable from fatal.
- `Context`, `ActionContext`, `TriggerContext`, `ActionContextExt` — execution context traits + extension helpers (`acquire_resource_by_id`, `resolve_credential_by_id`).
- `Dependencies`, `SlotField`, `SlotKind` (re-exported from `nebula-core`) — declarative slot metadata.
- `WebhookConfig`, `SignaturePolicy`, `RequiredPolicy`, `SignatureScheme` — ADR-0022 signature enforcement.
- `IsolationLevel`, `ActionCategory` — sandbox routing + UI grouping.
- `TestContextBuilder`, `StatefulTestHarness`, `TriggerTestHarness`, `SpyEmitter`, `SpyLogger`, `SpyScheduler` — testing utilities.

### Macros

- `#[derive(Action)]` — emits `Action` + `FromWorkflowNode` impls; parses `#[action(key, version, input, output, …)]` struct attribute and `#[resource]` / `#[credential]` field attributes.
- `#[action]` attribute macro — reserved for advanced cases requiring ADR-0035 phantom-shim field rewriting (`CredentialRef<dyn Bearer>` → `CredentialRef<dyn BearerPhantom>`); 95% of plugin authors use the derive form.
- `validate_action_package`, `ActionPackageValidationError` — package-level validation.

## Migration recipe (pre-v4 → v4)

The v4 surface is a hard break per `feedback_no_shims.md` / `feedback_hard_breaking_changes.md`. There is no automated codemod; migrate by hand:

1. **Split form data off `Self`.** Move `#[field]`-bearing fields off the action struct into a `<Name>Input: HasSchema + Deserialize` companion struct. Add `type Input = <Name>Input` to the `Action` impl (or `input = <Name>Input` to the derive's struct attribute).
2. **Drop `metadata()` boilerplate from `Self`.** The derive emits a `OnceLock` `metadata()` from the `#[action(key, version, …)]` arguments. Delete the manual `impl Action::metadata` block.
3. **Replace `dependencies()` macros with field attributes.** Old: a separate `DeclaresDependencies` impl listing `ResourceKey`s and `CredentialKey`s. New: `#[resource(key = "…")]` / `#[credential(key = "…")]` per field on the struct.
4. **Update sub-trait method signatures** to take `Self::Input` explicitly: `execute(&self, input: SendTelegramInput, ctx)` not `execute(&self, ctx)`.
5. **Replace `dyn Action` with `Arc<dyn ActionFactory>`** anywhere the engine or plugin loader stored an erased action. Existing transports / SDK harnesses that wrap `Arc<dyn StatelessHandler>` continue to work — four production paths intentionally stay on the legacy handler surface: webhook routing, sandbox discovery, SDK runtime, EventSource adapter. The original architectural rationale survives in `git log` (commits up to the retire-AI-Factory pass).
6. **For `ResourceAction` impls**, set `type Output = ResourceProduces<Self::Resource>`. The marker auto-derives `HasSchema`.
7. **For `Resource` impls**, drop `type Credential` (per ADR-0044) and declare credential deps via `#[credential(key = "…")]` field attributes — see `crates/resource/README.md` for the resource-side migration.

The full architectural rationale and per-phase migration shape for this redesign survive in `git log` only — the original plan and phase-block notes (the retired `.ai-factory/plans/m6-resource-finalization-integration-audit.md` and `PHASE3/4/7_BLOCKED.md` artifacts) were removed with the AI Factory framework.

## Runnable examples

The headline patterns are exercised end-to-end in the workspace `examples/` member:

- `cargo run -p nebula-examples --example resource_postgres_pool` — Pool topology + scoped resource configure/cleanup
- `cargo run -p nebula-examples --example resource_telegram_multi_workflow` — Resident topology + cross-workflow shared-resource dedupe (1 bot, 10 workflows, 1 `Resource::create` call)
- `cargo run -p nebula-examples --example resource_resident_http` — Resident topology + OAuth-style credential refresh hook

The examples deliberately wire slot resolution manually (no `#[derive(Action)]`) because they run outside an engine; the slot-binding **mental model** is illustrated explicitly. For derive-form authoring, see `crates/action/tests/derive_action.rs` and the trybuild probes under `crates/action/tests/probes/`.

## Contract

- **[L1-§3.5]** The action trait family (`StatelessAction`, `StatefulAction`, `TriggerAction`, `ResourceAction`) is the typed dispatch surface. Adding a new trait requires a canon revision (§0.2). The engine routes by trait, not by `ActionCategory` — that field is metadata for UI and tooling only.
- **[L2-§11.3]** For non-idempotent or risky side effects (payments, writes without natural upsert), action handlers must guard execution with the engine idempotency key path before calling the remote system. See `crates/execution/src/idempotency.rs`.
- **[L2-§13.4]** For `TriggerAction`-backed workflow starts, tests must cover the declared delivery contract (at-least-once): no silent drop, and duplicate delivery is handled via stable event identity and dedup/idempotency. Seam: `TriggerAction::start`, `TriggerEvent`.
- **[ADR-0022]** `WebhookAction::config()` is the declarative seam for webhook-transport signature enforcement. Default is `SignaturePolicy::Required` with an empty secret (fail-closed); the HTTP transport returns `401 problem+json` on signature mismatch and `500 problem+json` when `Required` is used without a secret. `OptionalAcceptUnsigned` is the explicit opt-out; `Custom(fn)` composes the primitives in `webhook.rs`. Secret material never flows through the dyn `TriggerHandler` surface — webhook configuration is read from the typed action at activation time and forwarded to `WebhookTransport::activate` as an explicit parameter.
- **[L2-§13.5]** For ordinary `StatelessAction` instances that cause irreversible external effects, integration tests must prove single-effect safety under retry/restart pressure. Seam: `StatelessAction::execute` + idempotency key guard.
- **CheckpointPolicy status** — `ActionMetadata` carries `IsolationLevel` and `ActionCategory` but does NOT currently carry a `CheckpointPolicy` field. `docs/INTEGRATION_MODEL.md` and older canon text reference `CheckpointPolicy` as a planned `ActionMetadata` field. Status: `planned` — not yet in the type. Tracked in `docs/MATURITY.md` row for `nebula-action` and noted in `docs/INTEGRATION_MODEL.md` §`nebula-action` status box. Do not document it as a current capability.

## Non-goals

- Not the execution state machine — see `nebula-execution` (`ExecutionStatus`, `ExecutionPlan`, CAS transitions).
- Not the only retry surface — per ADR-0042 the engine carries an
  operator-declared retry layer (`NodeDefinition.retry_policy`) that
  fires AFTER the action returns its final outcome. Action authors keep
  using `nebula-resilience::retry_with` internally for in-call retry;
  workflow authors declare engine-level retry at the node level. The two
  layers compose because they trigger at different boundaries (in-call
  vs. post-finalisation).
- Not the execution driver — the engine runs actions in-process (`InProcessSandbox`); isolation is a future additive concern (ADR-0091).
- Not a schema system — `ActionMetadata.parameters` holds a `ValidSchema` from `nebula-schema`; field definitions and validation rules live there.
- Not WASM — see canon §12.6.

## Maturity

See `docs/MATURITY.md` row for `nebula-action`.

- API stability: `frontier` — Variant A trait shape (Sized + type Input/Output + static metadata + slot-binding derive + FromWorkflowNode factory + ErasedAction dispatch) shipped under M6 / §M11 (2026-04-29). The `ActionHandler` enum and per-variant `XxxHandler` traits remain part of the public surface for transports / SDK harnesses / event sources that operate outside the workflow-node dispatch loop — the four production paths kept on the legacy handler surface are enumerated in the migration recipe above.
- `#![forbid(unsafe_code)]`, `#![warn(missing_docs)]` enforced.
- `CheckpointPolicy`: `planned` — not in `ActionMetadata` yet; engine does not consume it end-to-end.
- DX specializations (`PaginatedAction`, `BatchAction`, `WebhookAction`, `PollAction`) are implemented and tested; cross-action-type integration tests: partial.

## Related

- Canon: `docs/PRODUCT_CANON.md` §3.5 (action trait family; adding a trait = canon revision), §11.2 (retry surface lives in `nebula-resilience`, not the engine), §11.3 (idempotency), §12.6 (WASM non-goal), §13.4 (trigger delivery), §13.5 (non-idempotent side effects).
- ADRs: `docs/adr/0081-m6-resource-credential-integration.md` (M6 binding cascade — consolidates ADR-0042/0043/0044/0045).
- Integration model: `docs/INTEGRATION_MODEL.md` §`nebula-action` (including `CheckpointPolicy` status note).
- In-process plugin registry: `crates/plugin/README.md` — `Plugin` trait + `PluginRegistry` (ADR-0091).
- Siblings: `nebula-schema` (`ValidSchema` + `#[derive(Schema)]` for `Self::Input`), `nebula-credential` (`CredentialGuard` slot fields), `nebula-resource` (`ResourceGuard` slot fields, `ResourceAction`), `nebula-resilience` (retry/timeout/circuit-breaker inside actions).
- Resource sharing across nodes/workflows: see `crates/resource/README.md` "Shared resource pattern" — when multiple actions or workflows acquire the same `Resource` at the same scope, the manager dedupes by `(R::key(), ScopeLevel)` so a single `Resource::create` call serves every acquirer (e.g. one `TelegramBot` client for ten workflows).
