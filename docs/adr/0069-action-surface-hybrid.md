# ADR-0052: nebula-action — hybrid surface (object-safe `Action` + retained factory/erased indirection + function adapters)

**Status:** Accepted (2026-05-13)
**Tags:** action, dispatch, dx, plugin-sdk, breaking-source, refines-0042, refines-0043

## Context

The M6 / §M11 v4 surface for `nebula-action` shipped 2026-04-29 (per
ROADMAP `Status Snapshot`) under ADR-0042 (node-binding mechanism),
ADR-0043 (dependency declaration DX), ADR-0044 (resource/credential
singular supersession), and ADR-0045 (EventTrigger scope deferral). It
introduced:

- `Action: Sized` with static `metadata()`, `input_schema()`,
  `output_schema()`, `dependencies()` and typed `Input/Output` associated
  types.
- Per-field `#[resource(key = "...")]` / `#[credential(key = "...")]`
  slot binding via `#[derive(Action)]`.
- `FromWorkflowNode` async factory trait resolving slot bindings against
  `NodeDefinition.slot_bindings` per ADR-0042 hybrid mechanism.
- `ErasedAction` enum + per-variant object-safe sub-traits
  (`ErasedStateless`, `ErasedStateful`, `ErasedTrigger`, `ErasedResource`,
  `ErasedControl`).
- `ActionFactory` trait + `GenericXxxFactory<A>` adapters as the engine's
  per-execution dispatch layer (constructs a fresh
  `Box<dyn ErasedAction>` per node activation).
- `ResourceProduces<R>` Output marker for `ResourceAction`.
- `IdempotencyKey` re-exported from both `nebula-action::idempotency` and
  `nebula-execution::idempotency`.

Between 2026-04-29 and 2026-05-13 an in-flight rewrite landed on `main`
working tree (uncommitted) that **partially removes** this surface:

- Drops `pub mod erased`, `pub mod factory`, `pub mod from_workflow_node`,
  `pub mod resource_produces`, `pub mod idempotency` from `lib.rs`.
- Reduces `Action` to an object-safe `fn metadata(&self) -> &ActionMetadata`
  on `DeclaresDependencies + Send + Sync + 'static` (instance method
  instead of static).
- Moves typed `Input` / `Output` to per-shape sub-traits via AFIT
  (Rust 1.75+ stable; native idiom in 1.95).
- Adds `FnStatelessAction` / `FnStatelessCtxAction` adapters with
  `stateless_fn` / `stateless_ctx_fn` builders.

The working tree does not compile (19 errors in `nebula-action`;
downstream crates depend on the removed surface in ~14 files for
`ActionFactory` alone; `crates/action/src/stateless.rs:247-254`
`FnStatelessCtxAction` ignores its `_ctx` parameter — silent functional
bug). No commit message and no ADR explains the rollback rationale; the
retired planning iteration that originally proposed finishing it cited
only "slot-binding DX cost vs. benefit, dispatch complexity, factory layer
became a YAGNI middleman" as a task-description placeholder for this
ADR, not as substantiated evidence.

Four parallel research tracks ran on 2026-05-13:

- **ADR archeology** confirmed v4 closed five concrete pain points (§Why
  v4 below) and that the rollback narrative had no documented driver.
- **Engine deep dive** confirmed `ActionFactory::instantiate` runs per
  node activation, is the JSON↔typed marshaling boundary, and is the
  surface `nebula-plugin::Plugin::actions()` returns
  (`Vec<Arc<dyn ActionFactory>>`).
- **Working-tree intent reconstruction** confirmed the new sub-trait
  shapes (typed `Input/Output` on `StatelessAction`/`StatefulAction`/
  `TriggerAction`/`ResourceAction`/`ControlAction` via AFIT) are sound,
  and the `Action::metadata(&self)` reduction is sound.
- **Industry patterns** — Rust 1.95+ stable RPITIT, AFIT, `dyn`-compat
  research (Niko, "Box-box-box", 2025-03-24), DataFusion
  `UserDefinedLogicalNode`, Temporal `sdk-core` activity registration,
  axum `Handler<T,S>`, `erased-serde` adapter — converge on
  **typed-author trait + erased adapter sub-trait + per-variant
  registries**. v4 already implements that consensus shape.

A full rollback (drop `ActionFactory` + `ErasedAction` + `FromWorkflowNode`)
would discard the indirection that the consensus shape relies on, lose
slot-binding S-C2 mitigation, and force a churn cascade through five
sibling crates without a documented benefit. A no-op (revert the working
tree) would discard genuinely useful improvements: an object-safe
`Action` base, function-backed authoring DX, async-trait removal in
`trigger/mod.rs`.

This ADR ratifies the **hybrid**: keep the v4 dispatch core, adopt the
working tree's surface tweaks, fix the FnCtx bug.

## Why v4 (preserved capabilities)

Cited verbatim from ADRs:

1. **Zero-config single-resource binding** — slot key serves as the
   default `ResourceId` for 80%+ of actions (ADR-0042 §Positive).
2. **Multi-environment override per node** — without source modification
   via `NodeDefinition.slot_bindings` JSON (ADR-0042 §Decision).
3. **S-C2 shadow attack closure** — explicit ID-based
   `ctx.acquire_resource_by_id::<R>(id)` /
   `ctx.resolve_credential_by_id::<C>(id)` instead of type-name lookup
   (ADR-0043 §Positive).
4. **Type-system enforcement of optional/required/lazy** —
   `ResourceGuard<R>` / `Option<…>` / `Lazy<…>` wrappers vs attribute
   flags (ADR-0043 §3 vs Rejected Alternative E).
5. **Single declaration path across `nebula-action` /
   `nebula-resource` / `nebula-credential`** — one
   `#[derive(X)]` + per-field attributes, no API divergence
   (ADR-0043 §Positive).

Hybrid preserves all five.

## What changed in the working tree (deltas adopted)

1. **Object-safe `Action` base** — `fn metadata(&self) -> &ActionMetadata`
   on a trait with `DeclaresDependencies + Send + Sync + 'static`. The
   factory layer can now hold `Arc<dyn Action>` directly, simplifying
   debug surfaces and registry inspection. Hybrid keeps the typed
   `Input/Output` on the per-shape sub-traits via AFIT.
2. **Function adapters** — `FnStatelessAction<F, Input, Output>` +
   `FnStatelessCtxAction<F, Input, Output>` with `stateless_fn` /
   `stateless_ctx_fn` builders. Removes derive-macro friction for the
   slot-less authoring path.
3. **`async_trait` removal in trigger handlers** — manual
   `Pin<Box<dyn Future + Send>>` returns on dyn-side, AFIT on typed-side
   (idiomatic per Wuyts 2024 + Rust 1.95 dyn-compat boundaries).
4. **`IdempotencyKey` — both types coexist (correction).** Initial
   intent was to consolidate in `nebula-execution`. On verification
   2026-05-13, the two types serve **different layers**:
   `nebula_action::IdempotencyKey` is a transport-level dedup
   identifier returned by `TriggerAction::idempotency_key(event)`
   (simple `String` wrapper); `nebula_execution::IdempotencyKey` is
   the engine composite key
   (`{execution_id}:{node_key}:{attempt[:iteration]}`) for
   exactly-once node dispatch. Both remain in their respective
   crates; this ADR does NOT consolidate them.
5. **`Action::metadata` becomes instance** — engine call sites that used
   `<A as Action>::metadata()` (static) move to instance access via the
   action handle they already hold; `ActionFactory::metadata(&self)`
   remains the static-style entry for registry display.

## Decision

Adopt **hybrid surface** for `nebula-action`:

- `Action`: object-safe trait, `fn metadata(&self) -> &ActionMetadata`,
  supertraits `DeclaresDependencies + Send + Sync + 'static`. Compatible
  with `Arc<dyn Action>` in registries.
- Sub-traits via AFIT (Rust 1.95+):
  - `StatelessAction { type Input; type Output; async fn execute(...) }`
  - `StatefulAction { type Input; type Output; type State; async fn execute(...) }`
  - `TriggerAction { type Source; type Error; async fn start/stop(...) }`
  - `ResourceAction { type Resource; async fn configure/cleanup(...) }`
  - `ControlAction` (desugared)
- Dispatch indirection retained:
  - `ActionFactory: Send + Sync` with object-safe
    `metadata(&self) -> &ActionMetadata` and async `instantiate(node, ctx) -> Result<ErasedAction, FactoryError>`.
  - `ErasedAction` enum over `Box<dyn ErasedXxx>` per variant; the
    JSON↔typed marshaling boundary lives inside `ErasedXxxImpl<A>`.
  - `GenericXxxFactory<A>` blanket adapters for any
    `A: Action + FromWorkflowNode + StatelessAction` (etc.).
- `FromWorkflowNode` becomes opt-out via default-impl no-op:
  - When the action declares no `#[resource]` / `#[credential]` slot
    fields, `#[derive(Action)]` emits a `FromWorkflowNode` impl whose
    body is `Self::default()`. No slot-binding tax for trivial actions.
  - Function adapters (`FnStatelessAction` / `FnStatelessCtxAction`)
    implement `FromWorkflowNode` directly with the no-op body so they
    plug into `GenericStatelessFactory<A>` unchanged.
- `FnStatelessCtxAction` closure signature corrected to
  `Fn(Input, &dyn ActionContext) -> Fut`. The current ignored-`_ctx`
  signature is a bug.
- `IdempotencyKey` lives in `nebula-execution`. `nebula-action::trigger`
  imports it across the crate boundary; no re-export from
  `nebula-action`.
- `async_trait` macro is removed from `crates/action/src/trigger/mod.rs`.
  Dyn-side `TriggerHandler` uses manual `Pin<Box<dyn Future + Send>>`;
  typed-side `TriggerAction` uses AFIT.
- `ActionResult::Retry` + `unstable-retry-scheduler` feature flag are
  unchanged by this ADR (separate retry-pipeline track per ROADMAP §M2).

## Reverse-dependency citation

The dispatch indirection is referenced by:

- **`nebula-engine`** (~3 files, ~24 use sites):
  `crates/engine/src/runtime/registry.rs:67-256` (factory storage +
  `register_factory` + per-variant convenience methods),
  `crates/engine/src/runtime/runtime.rs:304-717` (`execute_action_with_node`,
  `dispatch_action`, `run_factory`, `execute_erased_*`),
  `crates/engine/src/runtime/remote_action.rs` (~9 uses).
- **`nebula-plugin`** (~3 files, ~11 use sites):
  `crates/plugin/src/plugin.rs:31-39` (`Plugin::actions() -> Vec<Arc<dyn ActionFactory>>`),
  `crates/plugin/src/registry.rs`, `crates/plugin/src/resolved_plugin.rs`.
- **`nebula-sandbox`** (~2 files, ~10 use sites):
  `crates/sandbox/src/runner.rs:43-51` (`SandboxRunner::execute` accepts
  pre-instantiated `Box<dyn ErasedXxx>`).
- **`nebula-api`** — `crates/api/src/routes/webhook.rs` and webhook
  transport.
- **`nebula-storage`** — `crates/storage/src/webhook_activation.rs`.
- **`nebula-execution`** — owns `IdempotencyKey`
  (`crates/execution/src/idempotency.rs`, `attempt.rs`, `state.rs`,
  `error.rs`, `lib.rs`).
- **`nebula-action-macros`** — emits `FromWorkflowNode` impls per
  derive call site.

Hybrid preserves every type signature these crates depend on except for
the static→instance `Action::metadata` shift. The engine update is
mechanical: replace `<A as Action>::metadata()` with `action.metadata()`
where `action: &dyn Action` is already in scope.

## Peer-project comparison

| Project | Author surface | Engine storage | Erasure |
|---|---|---|---|
| **DataFusion** | typed `impl UserDefinedLogicalNode` | `Arc<dyn UserDefinedLogicalNode>`, `Arc<dyn ExecutionPlan>` | trait dyn-safe by design (`Debug + Send + Sync`, no associated types in object-side methods) |
| **Temporal sdk-core** | `#[activity]` proc-macro on free fn | name-keyed registry of boxed closures | macro emits `Payload` (proto-bytes) ↔ typed args shim |
| **axum** | typed `async fn(T1, T2) -> R` | `MethodRouter` over `Handler<T, S>` with phantom `T` | tower `Service` blanket impl converts to uniform `Service<Request, Response=Response>` |
| **erased-serde** | typed serde traits | `&dyn erased_serde::Serialize` | adapter sub-trait + blanket impl bridge |
| **Bevy** | `impl Plugin for ...` | `Vec<Box<dyn Plugin>>` | plugin trait is itself dyn-safe; typed systems registered into `App` at `build` time |
| **apalis** | tower `Service<Job>` | `Service<Request>` | tower's universal erasure |

Every consensus point lands on the same shape: typed-author trait that
is *not* the dyn trait, plus a registration step (manual or
macro-emitted) that wraps the typed impl in a uniform-signature adapter
keyed by name/TypeId.

Hybrid for nebula-action maps directly onto this shape:

- Typed-author trait = `Action` + sub-traits (AFIT).
- Adapter at registration = `GenericXxxFactory<A>` (auto-wrapping per
  derive).
- Uniform engine storage = `Arc<dyn ActionFactory>` keyed by
  `ActionKey`.
- Dyn dispatch surface = `Box<dyn ErasedXxx>` per variant.

Sources:
[AFIT/RPITIT stabilization (Rust blog 2023-12-21)](https://blog.rust-lang.org/2023/12/21/async-fn-rpit-in-traits/),
[RFC 3185](https://rust-lang.github.io/rfcs/3185-static-async-fn-in-trait.html),
[Niko Matsakis "Box box box" 2025-03-24](https://smallcultfollowing.com/babysteps/blog/2025/03/24/box-box-box/),
[Async fundamentals — async fn in dyn trait](https://rust-lang.github.io/async-fundamentals-initiative/explainer/async_fn_in_dyn_trait.html),
[Yoshua Wuyts — async traits backed by manual Future impls](https://blog.yoshuawuyts.com/async-traits-can-be-directly-backed-by-manual-future-impls),
[DataFusion `UserDefinedLogicalNode`](https://docs.rs/datafusion/latest/datafusion/logical_expr/trait.UserDefinedLogicalNode.html),
[Temporal sdk-core ARCHITECTURE](https://github.com/temporalio/sdk-core/blob/master/ARCHITECTURE.md),
[axum `Handler` docs](https://docs.rs/axum/latest/axum/handler/trait.Handler.html),
[erased-serde](https://github.com/dtolnay/erased-serde),
[Possible Rust — 3 things to try when you can't make a trait object](https://www.possiblerust.com/pattern/3-things-to-try-when-you-can-t-make-a-trait-object).

## Consequences

### Positive

- API stability for external plugin authors: `Plugin::actions() -> Vec<Arc<dyn ActionFactory>>` unchanged.
- Slot-binding S-C2 mitigation preserved.
- Workspace aligns with industry-consensus typed-author + erased-adapter
  pattern.
- Function adapters land for slot-less authoring without forcing the
  derive path.
- `nebula-action` does not duplicate `IdempotencyKey`.
- `trigger/mod.rs` no longer needs `async_trait` proc-macro
  (-1 dep, -1 per-call `Box<dyn Future>` allocation per dispatch).
- `Action::metadata` is `&self` — registries can introspect a boxed
  action without monomorphization.

### Negative

- `Action::metadata` static→instance is a hard source break for any
  caller that wrote `<A as Action>::metadata()`. Cascade is mechanical
  but real (engine + plugin + sandbox + sibling tests).
- Macro maintenance burden from v4 (5 macros per ADR-0043 cost section)
  is unchanged. Hybrid does not consolidate them; that is a separate
  initiative.
- `FromWorkflowNode` default-impl strategy must be chosen at
  implementation time (blanket impl vs derive-emitted no-op); the
  blanket-impl path may conflict with derive output and require
  `#[doc(hidden)]` markers.
- Webhook providers
  (`crates/action/src/webhook/providers/{generic,slack,stripe}.rs`)
  must be rewritten against the AFIT sub-trait shape.

### Neutral

- `ResourceProduces<R>` marker stays. No engine change.
- ADR-0042 / ADR-0043 are **Amended** (not Superseded) — slot-binding
  mechanism survives intact; only the `Action` base trait shape and the
  authoring entry points evolve.
- ADR-0044 (resource/credential singular supersession) and ADR-0045
  (EventTrigger deferral) unaffected.

## Status of prior ADRs

- ADR-0042 (node-binding mechanism) → `Amended by ADR-0052` —
  slot-binding mechanism preserved; only the `Action` base trait shape
  changed.
- ADR-0043 (dependency declaration DX) → `Amended by ADR-0052` — slot
  field DX preserved; function adapters added as alternative authoring
  path; `FromWorkflowNode` becomes opt-out for slot-less actions.
- ADR-0044 (resource/credential singular) → unchanged.
- ADR-0045 (EventTrigger deferral) → unchanged.

## Migration

External plugin authors (`Plugin::actions()` consumers): no source
change required. Internal call sites that called
`<A as Action>::metadata()` (static) update to either
`action.metadata()` (instance, when `&dyn Action` is in scope) or
`<GenericXxxFactory<A> as ActionFactory>::metadata(&factory)` (when
holding the factory).

Action authors using `#[derive(Action)]` with `#[resource]` /
`#[credential]` fields: no change. Action authors implementing `Action`
manually: change `fn metadata() -> &ActionMetadata` (static, returning
`&'static`) to `fn metadata(&self) -> &ActionMetadata` (instance,
returning `&'self`). `OnceLock`-backed implementations are
straightforward.

## Out of scope

- Engine retry pipeline rework (ROADMAP §M2.x).
- New trait families (`EventTrigger` per ADR-0045) — deferred.
- Macro consolidation across action/credential/resource (ADR-0043 cost
  acknowledged).
- Removal of `ActionResult::Retry` / `unstable-retry-scheduler` flag —
  separate decision.
- Performance characterization of the dispatch path under hybrid —
  measure post-merge; create follow-up plan if regression appears.
