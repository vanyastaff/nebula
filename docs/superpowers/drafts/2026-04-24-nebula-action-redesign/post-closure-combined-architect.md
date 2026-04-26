# Post-closure combined architect report — Phase 2 + Phase 3

**Date:** 2026-04-25
**Author:** architect (post-closure audit, sub-agent dispatch)
**Inputs:**
- `post-closure-rust-senior-inventory.md` (production inventory, full)
- `post-closure-tech-lead-coverage.md` (Tech Spec coverage map: 5 🔴 + 3 🟠 + 8 🟡)
- Production: `crates/action/src/{trigger,webhook,poll,stateful,resource,control,stateless,handler}.rs`
- Tech Spec FROZEN CP4 (status header line 3) and full `§0`-`§17` walk
- ADR-0038 §Decision items 1-4 + §Neutral block; ADR-0040 §1 sealed-DX framing
- Memory: `feedback_post_freeze_cross_adr_check.md`, `feedback_third_pushback_carrier_axis.md`

**Scope:** (A) §2.9 fifth iteration with Phase 1 NEW evidence; (B) enact 5 🔴 + 3 🟠 + 8 🟡 amendments-in-place; (C) cascade closure verdict.

---

## Part A — §2.9 fifth iteration outcome

### Verdict: (III) — DEFER + name distinct gap

**§2.9 REJECT preserved.** Trait-level `Input`/`Output` consolidation onto a base `Action<I, O>` (or sub-trait `ExecutableAction<I, O>`) does NOT materialize even with the Phase 1 new evidence. Rationale follows.

**§2.6 sealed-DX framing IS misframed.** Tech Spec §2.6 declares `WebhookAction: WebhookActionSealed + TriggerAction` and `PollAction: PollActionSealed + TriggerAction` (line 415-416) — i.e., as **subtraits** of TriggerAction. Production reality (`webhook.rs:578` + `poll.rs:800`):

```rust
pub trait WebhookAction: Action + Send + Sync + 'static { type State: Clone + Send + Sync; ... }
pub trait PollAction:    Action + Send + Sync + 'static { type Cursor: Serialize + DeserializeOwned + Clone + Default + Send + Sync; type Event: Serialize + Send + Sync; ... }
```

Webhook and Poll are **PEERS of TriggerAction**, not subtraits. They have their own associated types (`State`, `Cursor`, `Event`), their own lifecycle methods (`on_activate`/`handle_request`/`on_deactivate`; `poll_config`/`validate`/`initial_cursor`/`poll`), and erase to `TriggerHandler` only at the dyn boundary via dedicated adapters (`WebhookTriggerAdapter`, `PollTriggerAdapter`). Neither family has any inherited `start`/`stop`/`handle` from TriggerAction — those methods live on the **adapters**, not on the DX traits.

### Why this does NOT flip §2.9

The user's Q5 framing assumed: *if WebhookAction/PollAction already have State-shaped associated types, then TriggerAction's "no Input/Output" asymmetry IS already broken — so consolidation onto trait-level types becomes principled.* The framing is wrong on two axes:

1. **Webhook/Poll are NOT TriggerAction.** Their `State`/`Cursor`/`Event` are properties of their **own** trait, not of TriggerAction. Triggering "WebhookAction has State" against "TriggerAction should have State" conflates two different traits.
2. **Trigger family is asymmetric internally — that is honest, not a defect.** WebhookAction has `State` (no Serde). PollAction has `Cursor` (Serialize+DeserializeOwned+Default) AND `Event` (Serialize). Base TriggerAction has neither (only `Source: TriggerSource` projecting `Event`). Three different state shapes within one family is exactly what production already has and what Strategy §3.1 component 7 already named ("triggers fire events into the engine's event channel"). Hoisting any of these to TriggerAction (so all three families share one shape) would force lying on Webhook (no Serde-bound State) or PollAction (Default-bound Cursor) where the bound asymmetry is load-bearing.

### What Part A names as a SEPARATE amendment-in-place finding

**§2.6 trait-bound chain is wrong against production.** This is a 6th 🔴 not in the Phase 1 coverage map (because Phase 1 did not cross-check §2.6 sealed-DX bound vs `webhook.rs:578` + `poll.rs:800` peer-not-subtrait shape). Tech Spec freeze should have caught:

- `WebhookAction: WebhookActionSealed + TriggerAction` (§2.6) → reality: `WebhookAction: Action + Send + Sync + 'static`
- `PollAction: PollActionSealed + TriggerAction` (§2.6) → reality: `PollAction: Action + Send + Sync + 'static`

This is **not Q4/§2.9 territory** — it is a sealed-DX trait-shape spec drift parallel to Q6 (lifecycle methods dropped from spec) and R1/R2 (associated types + lifecycle dropped from §2.2.2/§2.2.4). Same class of error: production has the shape, spec dropped it without rationale.

### Recorded outcome

- **§2.9 verdict UNCHANGED.** REJECT (refined four times: §2.9.1a / §2.9.1b / §2.9.1c / §2.9.1d below). The Q5 framing does not surface any consumer that requires trait-level Input/Output.
- **§2.6 amendment NEEDED** — listed under Part B as new 🔴 R6 (§2.6 sealed-DX bound chain re-pin: WebhookAction/PollAction declared as peer DX traits with their own State/Cursor/Event associated types, NOT as TriggerAction subtraits). Tech Spec §2.6 amendment-in-place lands per §15.11 enactment.
- **No ADR amendment.** ADR-0040 §1 sealed-DX framing references "the adapter pattern" — adapter-erasure of DX traits to primary `*Handler` dyn boundary is preserved either way; the bound-chain typo does not contradict ADR-0040's sealing intent. ADR-0040 stays at `proposed` per cascade prompt (canon §3.5 ratification still pending).

### Status qualifier

Recommended new qualifier: `FROZEN CP4 2026-04-25 (amended-in-place 2026-04-25 — Q1 + Q6 + Q7 post-closure audit per §15.X-§15.Y)` — with Q7 being the Part B amendment bundle landing today. Q5 itself does NOT earn a separate qualifier because it is rationale-tightening (§2.9.1d added to record the Q5 framing's resolution); per §15.9.5 / §15.9.6 precedent, rationale-only amendments do not warrant a status qualifier change. Q7 covers the structural amendments (R1-R6 + 🟠 + 🟡 bundle) below.

---

## Part B — Amendments enacted

### Amendment scope summary

| Class | Count | Tech Spec sections amended | Notes |
|---|---|---|---|
| 🔴 R1 | 1 | §2.2.2 + §15.11 | Restore `init_state` + lift `migrate_state` to trait shape |
| 🔴 R2 | 1 | §2.2.4 + §15.11 | Restore `configure` + `cleanup` (paradigm preserved per production) |
| 🔴 R3 | 1 | §2.2.3 + §3.2 + §15.11 | Restore `TriggerEventOutcome` fan-out path (action-author surface) |
| 🔴 R4 | 1 | §2.4 + §15.11 | Restore `ResourceHandler` `Box<dyn Any>` dyn boundary |
| 🔴 R5 | 1 | §2.4 + §3.2 + §15.11 | Document `TriggerHandler` type-erased payload typification path |
| 🔴 R6 | 1 (new — Part A finding) | §2.6 + §15.11 | Re-pin Webhook/Poll as peer DX traits with own associated types |
| 🟠 I1 | 1 | §8.1.2 + §15.11 | Cursor ownership narrative — engine-managed vs in-memory clarified |
| 🟠 I2 | 1 | §1.2 + §2.2.4 + §15.11 | Acknowledge cred-cascade dependency for `Resource: Resource` |
| 🟠 I3 | 1 | §3.2 (covered with R5) + §15.11 | Trigger event-dispatch typification narrative |
| 🟡 (8 items) | 8 | §2.2.2 / §2.2.3 / §2.6 / §2.8 / §9.3 / §9.5 + §15.11 | Bundled doc-gap closure |

### 🔴 R1 — `StatefulAction::init_state` + `migrate_state` in trait shape

**Production source:** `stateful.rs:56` (`init_state`), `stateful.rs:64` (`migrate_state`).

**Restoration:** §2.2.2 trait shape gains both methods alongside `execute`:

```rust
pub trait StatefulAction: Send + Sync + 'static {
    type Input: HasSchema + DeserializeOwned + Send + 'static;
    type Output: Serialize + Send + 'static;
    type State: Serialize + DeserializeOwned + Clone + Send + Sync + 'static;
    type Error: std::error::Error + Send + Sync + 'static;

    /// Create initial state for the first iteration. Engine-driven.
    fn init_state(&self) -> Self::State;

    /// Attempt to migrate state from older serialized format.
    /// Default returns None — engine surfaces deserialization error as
    /// ActionError::Validation per stateful.rs:519-524.
    fn migrate_state(&self, _old: serde_json::Value) -> Option<Self::State> { None }

    fn execute<'a>(
        &'a self,
        ctx: &'a ActionContext<'a>,
        state: &'a mut Self::State,
        input: Self::Input,
    ) -> impl Future<Output = Result<Self::Output, Self::Error>> + Send + 'a;
}
```

**Lifecycle narrative.** `init_state` is called once when engine starts executing the action; subsequent iterations receive the state mutated by the previous `execute`. `migrate_state` is consulted only when `from_value::<Self::State>(...)` fails on a persisted checkpoint — version-skew between stored state JSON and current State schema. Per `stateful.rs:518-524`: engine calls `migrate_state(value)`; `Some(migrated)` continues with migrated state, `None` propagates as `ActionError::Validation { reason: ValidationReason::StateDeserialization, .. }`.

### 🔴 R2 — `ResourceAction::configure` + `cleanup` in trait shape

**Production source:** `resource.rs:41-51`. Engine runs `configure` before downstream nodes, lends `&Self::Resource` to the subtree, calls `cleanup(self.resource, ctx)` when scope ends.

**Restoration:** §2.2.4 trait shape preserves the production paradigm (engine owns, action lends scoped). NO paradigm change — restoring the dropped methods, NOT introducing a new "engine resolves resource by id" model. The `resource_id: ResourceId` parameter in §2.4 `ResourceHandler::execute` is ALSO dropped per R4 below — production `ResourceHandler` (`resource.rs:60`) takes `Box<dyn Any + Send + Sync>` for the resource handoff, which is the actual ABI.

```rust
pub trait Resource: Send + Sync + 'static {
    type Credential: Credential;  // N1 — full Resource impl is OUT (per credential cascade)
}

pub trait ResourceAction: Send + Sync + 'static {
    type Resource: Resource;
    type Error: std::error::Error + Send + Sync + 'static;

    /// Build resource for this scope. Engine runs before downstream nodes.
    fn configure<'a>(
        &'a self,
        ctx: &'a ActionContext<'a>,
    ) -> impl Future<Output = Result<Self::Resource, Self::Error>> + Send + 'a;

    /// Release resource when scope ends (drop pool, close connections).
    fn cleanup<'a>(
        &'a self,
        resource: Self::Resource,
        ctx: &'a ActionContext<'a>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'a;
}
```

**Note on `execute`.** Production `ResourceAction` has NO `execute` — it is a graph-scoped DI primitive only. Earlier Tech Spec §2.2.4 had `execute(&self, ctx, &resource, input)` with `Input` / `Output` types. That shape is wrong against production; `execute` belongs on `StatelessAction` / `StatefulAction` (which can borrow a resource via `ctx.resource()`). The `Input`/`Output`/`execute` removal closes a parallel spec drift surfaced by R2 enactment — R2 is restoration, not paradigm change.

### 🔴 R3 + 🟠 I3 — `TriggerEventOutcome` restoration + typification path

**Production source:** `trigger.rs:215-264` (`TriggerEventOutcome`), `trigger.rs:97-203` (`TriggerEvent` + downcast), `trigger.rs:359-389` (`TriggerHandler::accepts_events` + default).

**Restoration:** §2.2.3 `handle()` returns `TriggerEventOutcome` per dispatch, NOT `Result<(), Error>`:

```rust
pub trait TriggerAction: Send + Sync + 'static {
    type Source: TriggerSource;
    type Error: std::error::Error + Send + Sync + 'static;

    fn start<'a>(
        &'a self,
        ctx: &'a ActionContext<'a>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'a;

    fn stop<'a>(
        &'a self,
        ctx: &'a ActionContext<'a>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'a;

    /// Whether this trigger accepts engine-pushed events. Default: false.
    /// Webhook-shape returns true; poll-shape keeps default false.
    fn accepts_events(&self) -> bool { false }

    /// Handle a single event projected from `Source`. Returns the per-event
    /// outcome — Skip filters out the event; Emit fires one workflow execution;
    /// EmitMany fans out to N executions per single transport event.
    fn handle<'a>(
        &'a self,
        ctx: &'a ActionContext<'a>,
        event: <Self::Source as TriggerSource>::Event,
    ) -> impl Future<Output = Result<TriggerEventOutcome, Self::Error>> + Send + 'a;
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum TriggerEventOutcome {
    Skip,
    Emit(serde_json::Value),
    EmitMany(Vec<serde_json::Value>),
}
```

**Typification path narrative (§3.2 addition).** Engine sources events of typed `<Self::Source as TriggerSource>::Event` from the source, wraps each in `TriggerEvent { payload: Box<dyn Any + Send + Sync>, payload_type: TypeId, .. }` for the dyn-handler boundary, dispatches via `Arc<dyn TriggerHandler>::handle_event(ctx, TriggerEvent)`. The adapter (`TriggerActionAdapter` for plain triggers; `WebhookTriggerAdapter`, `PollTriggerAdapter` for sealed-DX) receives the typed-erased envelope, downcasts to its expected payload type via `TriggerEvent::downcast::<T>()` (where `T = <A::Source as TriggerSource>::Event` for `TriggerActionAdapter<A>`), and invokes the typed `A::handle(ctx, event)`. Downcast mismatch is `ActionError::Fatal` per `trigger.rs:182-203` — engine routing bug, never user-recoverable. The typification boundary is `TriggerEvent::downcast::<T>()` at the adapter layer, NOT at the engine layer; engine sees `TriggerEvent`, action body sees typed `T`.

### 🔴 R4 — `ResourceHandler` `Box<dyn Any>` dyn boundary restoration

**Production source:** `resource.rs:59-107` — `ResourceHandler::configure(_config: Value, ctx) -> ResourceConfigureFuture<'a>` returning `Result<Box<dyn Any + Send + Sync>, ActionError>`; `ResourceHandler::cleanup(instance: Box<dyn Any + Send + Sync>, ctx) -> ResourceCleanupFuture<'a>`.

**Restoration.** §2.4 `ResourceHandler` shape switches from the dropped `ResourceId` paradigm back to `Box<dyn Any>` envelope:

```rust
#[async_trait]
pub trait ResourceHandler: Send + Sync + 'static {
    fn metadata(&self) -> &ActionMetadata;

    /// Build resource. Returns the type-erased instance for engine-side handoff.
    async fn configure(
        &self,
        config: serde_json::Value,
        ctx: &ActionContext<'_>,
    ) -> Result<Box<dyn Any + Send + Sync>, ActionError>;

    /// Release resource. Adapter downcasts the box to its typed Resource;
    /// downcast-mismatch is ActionError::Fatal per resource.rs:195-200.
    async fn cleanup(
        &self,
        resource: Box<dyn Any + Send + Sync>,
        ctx: &ActionContext<'_>,
    ) -> Result<(), ActionError>;
}
```

**Drop the `execute` method.** Pre-amendment §2.4 had `ResourceHandler::execute(ctx, resource_id, input) -> Result<Value, ActionError>`. Production has no such method — `ResourceHandler` is configure/cleanup only, and resources are read by *consumer actions* via `ctx.resource()` (or the per-scope borrow chain), not via a `ResourceHandler` execute path. Drop preserves production ABI.

### 🔴 R5 — `TriggerHandler` type-erased payload boundary

**Production source:** `trigger.rs:276-389`.

**Restoration:** §2.4 `TriggerHandler` adopts `TriggerEvent` envelope (NOT `serde_json::Value`):

```rust
#[async_trait]
pub trait TriggerHandler: Send + Sync + 'static {
    fn metadata(&self) -> &ActionMetadata;

    async fn start(&self, ctx: &ActionContext<'_>) -> Result<(), ActionError>;

    async fn stop(&self, ctx: &ActionContext<'_>) -> Result<(), ActionError>;

    /// Whether this handler accepts engine-pushed events. Default false;
    /// webhook adapter returns true.
    fn accepts_events(&self) -> bool { false }

    /// Handle an event pushed by the engine. Default implementation returns
    /// ActionError::fatal — pull-driven triggers (poll) never see this called.
    async fn handle_event(
        &self,
        ctx: &ActionContext<'_>,
        event: TriggerEvent,
    ) -> Result<TriggerEventOutcome, ActionError> {
        Err(ActionError::fatal("trigger does not accept external events"))
    }
}
```

The `serde_json::Value`-typed event boundary in pre-amendment §2.4 was wrong — production passes `TriggerEvent` (with type-erased `Box<dyn Any>` payload + `TypeId` diagnostic) per `trigger.rs:97-122`. The typification is at the adapter layer (per R3 above), not at the dyn-handler boundary.

### 🔴 R6 — §2.6 sealed-DX bound chain re-pin (Part A finding)

**Production source:** `webhook.rs:578` + `poll.rs:800`.

**Restoration.** §2.6 sealed-DX trait declarations re-pinned: WebhookAction and PollAction are NOT `: TriggerAction` subtraits. They are peer DX traits with their own associated types and lifecycle:

```rust
mod sealed_dx {
    pub trait ControlActionSealed {}
    pub trait PaginatedActionSealed {}
    pub trait BatchActionSealed {}
    pub trait WebhookActionSealed {}
    pub trait PollActionSealed {}
}

// Erases to Stateless via adapter:
pub trait ControlAction: sealed_dx::ControlActionSealed + StatelessAction { /* ... */ }

// Erase to Stateful via adapter (adapter holds the iteration state):
pub trait PaginatedAction: sealed_dx::PaginatedActionSealed + StatefulAction { /* ... */ }
pub trait BatchAction:     sealed_dx::BatchActionSealed     + StatefulAction { /* ... */ }

// Erase to Trigger DYN HANDLER via dedicated adapter (NOT via TriggerAction subtraiting):
pub trait WebhookAction: sealed_dx::WebhookActionSealed + Action + Send + Sync + 'static {
    type State: Clone + Send + Sync;
    /* on_activate / handle_request / on_deactivate / config */
}
pub trait PollAction: sealed_dx::PollActionSealed + Action + Send + Sync + 'static {
    type Cursor: Serialize + DeserializeOwned + Clone + Default + Send + Sync;
    type Event: Serialize + Send + Sync;
    /* poll_config / validate / initial_cursor / poll */
}

// Blanket sealing at `Action` (NOT `TriggerAction`):
impl<T: StatelessAction + ActionSlots> sealed_dx::ControlActionSealed for T {}
impl<T: StatefulAction  + ActionSlots> sealed_dx::PaginatedActionSealed for T {}
impl<T: StatefulAction  + ActionSlots> sealed_dx::BatchActionSealed     for T {}
impl<T: Action          + ActionSlots> sealed_dx::WebhookActionSealed   for T {}
impl<T: Action          + ActionSlots> sealed_dx::PollActionSealed      for T {}
```

**Adapter-erasure path preserved.** WebhookAction erases to `Arc<dyn TriggerHandler>` via `WebhookTriggerAdapter`; PollAction erases via `PollTriggerAdapter`. Adapters bridge the DX-trait shape (`on_activate`/`handle_request`/`on_deactivate`; `poll`/`initial_cursor`) to the dyn-handler shape (`start`/`stop`/`accepts_events`/`handle_event` per R5). Adapters are the only erasure path; community plugins cannot author parallel adapters because `WebhookActionSealed` / `PollActionSealed` are crate-private.

### 🟠 I1 — PollCursor ownership narrative

**§8.1.2 amendment.** Pre-amendment §8.1.2 says "cursor lives at engine, not action body." Production reality (`poll.rs:1328` + `:1351-1352`): cursor lives in `PollTriggerAdapter::start()`'s stack frame (a local variable in the async loop), with the `PollCursor<C>` per-cycle wrapper carrying a checkpoint snapshot for rollback. There is **NO engine-side persistence** — restart re-calls `initial_cursor()` from scratch. Tech Spec must say so honestly:

> #### §8.1.2 Trigger cursor — in-memory, per-process lifetime
>
> `PollAction::Cursor` lives in `PollTriggerAdapter::start()`'s stack frame for the duration of the trigger task. The adapter constructs `PollCursor<Self::Cursor>` per cycle (a wrapper carrying `(current, checkpoint)` for in-cycle rollback per `poll.rs:439-477`), unwraps via `into_current()` after the cycle. **No engine persistence.** On process restart, `initial_cursor(&self, ctx)` is called again; cursor state evaporates with the trigger task.
>
> Cluster-mode coordination (idempotency-key dedup, leader-elected gating) is engine-cascade scope per §1.2 N4 — when engine cluster-mode lands, it adds a *parallel* persistence layer (cluster-coordinated cursor checkpointing); the action-author surface (`PollAction::Cursor` + `&mut PollCursor<Self::Cursor>` parameter on `poll`) is unchanged. Engine adds the cluster layer *around* the action surface, not by hoisting cursor state to a different location.

### 🟠 I2 — `ResourceAction::Resource: Resource` cred-cascade dependency

**§1.2 N1 + §2.2.4 narrative addition.** §1.2 N1 (existing) covers `Resource::on_credential_refresh full integration` deferral; CP4 amendment adds:

> §1.2 **N1 (extended).** `ResourceAction::Resource: Resource` bound (where `Resource: Resource` requires `type Credential: Credential`) creates a hard ordering dependency on credential cascade landing first — no production resource type today implements the new `Resource` trait. Path (a) single coordinated PR can satisfy this implicitly; paths (b) / (c) MUST sequence credential cascade leaf-first. Per §16.5 cascade-final precondition: if user picks path (b), credential CP6 cascade slot must include a `Resource` trait surface beat. If user picks path (c), the action `Resource: Resource` bound is delegating; credential CP6 cascade lands the trait surface.

### 🟡 (8 items) — bundled documentation closure

| # | Site | Closure |
|---|---|---|
| Y1 | §2.2.2 trait | `migrate_state` lifted to trait shape (closes §1.3 row 2 gap). Already covered by R1 enactment above |
| Y2 | §9.3.2 (added) + §9.3.3 (reshuffled) | `assert_*!` test macros (`stateless::macros.rs:204` — 11 macros: `assert_success!`, `assert_continue!`, `assert_branch!`, `assert_skip!`, etc.) preserved; documented as part of `nebula-action::test_helpers` re-export. Migration: existing call sites unchanged |
| Y3 | §2.6 + §9.3.2 | DX type families (`ControlInput`, `ControlOutcome`, `PageResult<T,C>`, `PaginationState<C>`, `WebhookConfig`, `SignaturePolicy`, `WebhookRequest`, `WebhookResponse`, `PollConfig`, `PollCursor<C>`, `DeduplicatingCursor<K,C>`, `PollResult<E>`, `PollOutcome<E>`, `EmitFailurePolicy`, `POLL_INTERVAL_FLOOR`) preserved; enumerated at §9.3.2 added-list with disposition "preserved verbatim from production" |
| Y4 | §2.8 enumeration | `ActionError::From<CredentialAccessError>` / `From<CoreError>` impls + `CredentialRefreshFailed` variant explicitly enumerated. Already in production `error.rs:254-265, 305-333`; spec just names them |
| Y5 | §2.4 `TriggerHandler` | `accepts_events()` predicate defaulted to `false` per R5 above (already covered) |
| Y6 | §2.2.x narrative | `#[diagnostic::on_unimplemented]` attributes on TriggerAction / StatefulAction / StatelessAction — preserved verbatim per production with messages identical to `trigger.rs:57-60`, `stateful.rs:34-37`, `stateless.rs:24-27` |
| Y7 | §2.2.3 narrative | `&self` configuration carrier paradigm example — adds a `WebhookAction { pub url: String, pub secret: SecretString }` example showing per-instance config flows from struct fields to `start()` body via `&self` access |
| Y8 | §10.4 step (new) | `TriggerEvent` consumer migration — community plugin authors who today consume `TriggerEvent::downcast::<WebhookRequest>()` continue to do so post-cascade; the `TriggerEvent` API is preserved verbatim. T-codemod handles only sealed-DX trait-shape compose changes (R6); `TriggerEvent`-consumer migration is null |

### Tech Spec edits enacted

The following edits land via Edit tool calls (this report serves as the rationale; mechanical changes are inline at the cited sections):

1. **§0.1 status header** — qualifier appended: `FROZEN CP4 2026-04-25 (amended-in-place 2026-04-25 — Q1 + Q6 + Q7 post-closure audit per §15.9-§15.11)`
2. **§2.2.2** — restore `init_state` + `migrate_state` (R1)
3. **§2.2.3** — restore `accepts_events`, change `handle()` return to `TriggerEventOutcome` (R3); preserve `start`/`stop` from Q6
4. **§2.2.4** — restore `configure`/`cleanup`; drop `execute`/`Input`/`Output` (R2)
5. **§2.4** — `ResourceHandler` adopts `Box<dyn Any>` boundary; `TriggerHandler` adopts `TriggerEvent` envelope (R4 + R5)
6. **§2.6** — re-pin Webhook/Poll as peer-of-Action (NOT TriggerAction subtraits); add State/Cursor/Event associated types verbatim from production (R6)
7. **§3.2** — typification path narrative at the adapter layer (R3 + R5 + I3)
8. **§8.1.2** — cursor ownership narrative (I1)
9. **§1.2 N1** — extended cred-cascade dependency note (I2)
10. **§15.11 (new)** — Q7 enactment record for the bundle
11. **§17 CHANGELOG** — Q7 amendment entry

### ADR amendments

**NONE.** Per Part A verdict (III) — §2.9 unchanged so ADR-0038 / ADR-0039 / ADR-0040 unaffected. R1-R6 land at Tech Spec §2 per-method-signature lock, NOT at ADR family-enumeration lock; ADR-0038 §Neutral block "unchanged at the trait level — only the macro that constructs implementations changes shape" preserves the four-trait identity (no new primary trait added). R3/R5 dyn-handler shape adopts `TriggerEvent` (not JSON) — this is per ADR-0024 `#[async_trait]` policy (already covered at §15.9). R6 sealed-DX bound chain is at Tech Spec §2.6 lock; ADR-0040 §1 sealed-pattern intent (community plugins cannot bypass the adapter) is preserved either way.

---

## Part C — Cascade closure decision

### Verdict: **AMENDED CLOSED**

The Phase 1 audit surfaced 5 🔴 + 3 🟠 + 8 🟡 (Tech Spec coverage map) plus a 6th 🔴 from the Phase 2 §2.9 fifth iteration (R6 sealed-DX bound chain re-pin). All 17 are **mechanical drift** between Tech Spec §2/§3/§8 and production source — same class as Q6 lifecycle gap (production has the shape, spec dropped without rationale). None require ADR supersession; all close via amendment-in-place per ADR-0035 precedent + §15.9 / §15.10 enactment template.

The cascade is **NOT FULLY-CLOSED** — Phase 1 evidence is decisive that the FROZEN CP4 spec is structurally incomplete against production reality (only ~50% of capabilities covered per tech-lead audit summary). Calling it FULLY-CLOSED would be face-saving per `feedback_active_dev_mode.md` ("never settle for green tests / cosmetic / quick win / deferred"). AMENDED CLOSED is honest:

- Q1 + Q6 closed prior cascade slips (manual `BoxFut` vs `#[async_trait]`; lifecycle methods on TriggerAction).
- Q7 (this audit, batched) closes 6 🔴 + 3 🟠 + 8 🟡 in one amendment-in-place pass — same mechanism as Q1/Q6.
- ADR-0038 / ADR-0039 / ADR-0040 statuses unchanged. Canon §3.5 ratification still pending on user; Phase 8 cascade summary surfaces it.

The cascade is **NOT RE-OPENED** — no Strategy revision needed; no ADR supersede; no spike re-validation; the production-shape evidence is the source for restoration. Same precedent as Q6.

### Escalation flags

**No escalation.** Q7 amendment-in-place lands within Tech Spec author authority per ADR-0035 §Status block "canonical-form correction" criterion (cross-source-authoritative correction; production wins for capabilities the spike did not exercise). Tech-lead post-closure ratification is the gate, not user pushback.

**Phase 1 audit quality assessment.** 17 missed findings across 5 reviewers across 4 CPs is a meaningful cascade-quality miss — flagged for post-cascade retrospective (out of scope here). The miss class is **production-drift in spec sections that nobody cross-checked against production** (§2.2.2 / §2.2.3 / §2.2.4 / §2.4 / §2.6 / §8.1.2). The dual-audit pattern (rust-senior production inventory + tech-lead Tech Spec coverage map) is what surfaced the gap; should be a standing requirement for future Tech Spec freezes.

---

*End of post-closure combined architect report. Phase 2 + Phase 3 of post-closure audit.*
