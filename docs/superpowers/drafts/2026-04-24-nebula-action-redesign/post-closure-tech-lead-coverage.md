# Post-closure systematic audit — Phase 1 of 3 — Tech Spec coverage map

**Date:** 2026-04-25
**Author:** tech-lead (post-closure audit, sub-agent dispatch)
**Scope:** cross-reference production capabilities (`crates/action/src/**`) to Tech Spec §2-§13 coverage; categorize each with severity tag.
**Inputs read:** Tech Spec FROZEN CP4 (`docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md` lines 1-1900, 2400-2750, key sections via grep); Phase 0 audit (`01a-code-audit.md` full); Phase 1 pain enumeration (`02-pain-enumeration.md` full); Q6 lifecycle gap fix (`15-q6-lifecycle-gap-fix.md`); production source: `trigger.rs` full, `resource.rs` first 120, `stateful.rs` first 150, `poll.rs` first 200 + grep PollCursor matches.
**Cross-ref attempted to:** `post-closure-rust-senior-inventory.md` — file does NOT exist at audit time (5 polls); proceeded without cross-ref per orchestrator fallback instruction.

**Severity legend:** 🔴 REGRESSION (capability lost without rationale; impl correctness affected) · 🟠 ARCHITECTURAL INVERSION (production pattern flipped without rationale) · 🟡 DOCUMENTATION GAP (capability preserved through decoupling/adapter, but Tech Spec doesn't say where it lives) · 🟢 INTENTIONAL REMOVAL (dropped with rationale) · ✅ COVERED.

---

## §1 Coverage map by trait

### §1.1 `Action` (base supertrait)

| Capability | Production location | Tech Spec coverage | Status | Severity notes |
|---|---|---|---|---|
| `metadata(&self) -> &ActionMetadata` | `action.rs:27` | §2.1 | ✅ | Identity preserved |
| `#[diagnostic::on_unimplemented]` ("derive it: #[derive(Action)]") | `action.rs` | None — replaced by `#[action]` attribute macro | 🟢 | ADR-0038 §Decision item 1; rationale documented |
| Direct user impl (today permissible) | `action.rs:27` | §2.1 narrative — "User code does NOT implement `Action` directly — the `#[action]` macro emits a concrete impl" | 🟢 | Locked behind macro; intentional |
| `ActionSlots` supertrait composition | NONE today (`action.rs` does not require `ActionSlots`) | §2.1 — `Action: ActionSlots + Send + Sync + 'static` | ✅ | New supertrait chain; ADR-0039 §2 |

### §1.2 `StatelessAction`

| Capability | Production location | Tech Spec coverage | Status | Severity notes |
|---|---|---|---|---|
| `type Input: HasSchema` | `stateless.rs` | §2.2.1 lift adds `DeserializeOwned + Send + 'static` | ✅ | CR9 close |
| `type Output` (untyped Send + 'static) | `stateless.rs` | §2.2.1 lifts `Serialize + Send + 'static` | ✅ | Adapter-leak-fix |
| `execute<'a>(&'a self, ctx, input) -> impl Future` | `stateless.rs:597` | §2.2.1 RPITIT verbatim | ✅ | Spike-locked |
| `assert_*!` test macros (11 macros: success, branch, continue, break, skip, wait, retry, retryable, fatal, validation_error, cancelled) | `macros.rs:204` | None — Tech Spec §5 + §6.4 list test contract but does NOT enumerate the assertion macros as preserved-DX surface | 🟡 | Author DX surface widely used in tests; not surfaced in §9 prelude or §16 DoD; preserved in code, but Tech Spec silent on whether they survive cascade |

### §1.3 `StatefulAction`

| Capability | Production location | Tech Spec coverage | Status | Severity notes |
|---|---|---|---|---|
| `type State: Serialize + DeserializeOwned + Clone + Send + Sync + 'static` | `stateful.rs:50` | §2.2.2 + §8.1.1 | ✅ | Bound chain lifted onto trait per CP1 |
| `init_state(&self) -> Self::State` | `stateful.rs:56` | **NOT NAMED in Tech Spec §2.2.2** (only `execute(.., &mut state, ..)` shown) | 🔴 | **Lifecycle method silently dropped from spec.** §2.2.2 shows only `execute` — `init_state()` is required for the engine to construct the first state. §8.1.1 narrative refers to "JSON serialization (`to_value(&typed_state)`)" but never names `init_state`. Same class of slip as Q6 trigger lifecycle — production has lifecycle method, spec dropped without rationale. |
| `migrate_state(&self, _old: Value) -> Option<Self::State>` | `stateful.rs:64` | §8.1.1 narrative cites it ("consulted only when `from_value` fails") | 🟡 | Cited in narrative, **not in §2.2.2 trait shape** — a reader of §2.2 alone cannot derive the migration contract. Implementer would have to read §8 to discover migrate_state exists. Documentation gap. |
| `execute(.., &mut state, ..)` per-iteration shape | `stateful.rs:72` | §2.2.2 verbatim | ✅ | RPITIT lift |

### §1.4 `TriggerAction`

| Capability | Production location | Tech Spec coverage | Status | Severity notes |
|---|---|---|---|---|
| `start(&self, ctx) -> Result<(), ActionError>` | `trigger.rs:63` | §2.2.3 (POST Q6 amendment-in-place 2026-04-25) | ✅ | Closed by Q6 |
| `stop(&self, ctx) -> Result<(), ActionError>` | `trigger.rs:69` | §2.2.3 (POST Q6 amendment-in-place 2026-04-25) | ✅ | Closed by Q6 |
| Type-erased `TriggerEvent` envelope (`Box<dyn Any + Send + Sync>` payload, runtime downcast in adapter) | `trigger.rs:97-213` | §2.2.3 typed `<Self::Source as TriggerSource>::Event` projection — type-erasure absent | 🟠 | **Architectural inversion at the dyn-handler boundary**. Production decouples action-trait from transport via `Box<dyn Any>` envelope; adapter downcasts to action-specific event type. Tech Spec §2.2.3 + §2.4 `TriggerHandler::handle(ctx, event: serde_json::Value)` collapse this to JSON-typed event at the dyn boundary AND typed `Source::Event` at the user-typed boundary. The "how does the engine bridge JSON-Value to typed `<Source as TriggerSource>::Event`" mechanism is **NOT specified in §2.4 or §11**. Q6 doc (15-q6-lifecycle-gap-fix.md line 96) claims "no regression — production already supports event-driven dispatch through the type-erased envelope; cascade typifies it" — but Tech Spec proper does not echo this rationale or specify the typification boundary. |
| `TriggerEventOutcome::{Skip, Emit(Value), EmitMany(Vec<Value>)}` (return shape from event handling) | `trigger.rs:215-264` | §2.2.3 `handle()` returns `Result<(), Self::Error>` — outcome shape collapsed to fire-and-forget unit | 🔴 | **REGRESSION — outcome multiplicity lost**. Production `TriggerEventOutcome::Emit(Value)` and `EmitMany(Vec<Value>)` allow a single transport event (one webhook delivery) to fan-out to N workflow executions. §2.2.3 `handle() -> Result<(), Error>` has no fan-out path. Tech Spec narrative (§2.2.3 line 251-252 + §8.1.2 + Strategy §3.1 component 7) refers to "engine receives events on a channel" — but the action's per-event multiplicity (1 event → N executions OR Skip) has no surface in the typed contract. Engine-cascade scope (§1.2 N4) per §2.2.3, but the **action-author surface for fan-out is undeclared** in Tech Spec. |
| `TriggerHandler::accepts_events()` predicate (engine asks before pushing events) | `trigger.rs:359` | NOT covered | 🟡 | Production gates engine push-event path; Tech Spec doesn't surface whether engine still asks. Likely engine-internal contract; could be inferred lost or moved. |
| `#[diagnostic::on_unimplemented]` ("implement `start` and `stop` methods") | `trigger.rs:57-60` | NOT covered | 🟡 | DX diagnostic dropped; minor |

### §1.5 `ResourceAction`

| Capability | Production location | Tech Spec coverage | Status | Severity notes |
|---|---|---|---|---|
| `type Resource: Send + Sync + 'static` | `resource.rs:38` | §2.2.4 with `Resource: Resource` (new Resource trait with `type Credential: Credential`) | 🟠 | **Architectural inversion**. Production `ResourceAction::Resource: Send + Sync + 'static` (single bound). Tech Spec §2.2.4 narrows to `Resource: Resource` requiring resource type to implement a NEW `Resource` trait (with `type Credential: Credential`). This forces every existing resource type (`HttpClient`, `PostgresPool`, etc.) to acquire a `Credential` associated type binding. **Today no production resource type implements such a `Resource` trait** (the trait is part of `nebula-credential` per credential Tech Spec §15.7). N1 marks `Resource::on_credential_refresh full integration` OUT-of-scope, but §2.2.4's `Resource: Resource` bound is action-cascade-scope and creates a hard dependency on the credential cascade landing first. Tech Spec acknowledges this only obliquely ("resource-side scope is N1 / OUT (Strategy §3.4 line 173)"). |
| `configure(&self, ctx) -> Future<Self::Resource>` lifecycle | `resource.rs:41` | §2.2.4 collapses to `execute(&self, ctx, resource: &Self::Resource, input)` — **NO `configure` method** | 🔴 | **REGRESSION — `configure` lifecycle dropped**. Production resources are **graph-scoped DI**: engine calls `configure` once per scope before downstream nodes; resource is owned by the engine, borrowed via `&Self::Resource` to children. Tech Spec §2.2.4 has only `execute(.., resource: &Self::Resource, ..)` per dispatch — there is no method that **creates** the resource. Same class of slip as Q6 trigger / StatefulAction `init_state`. |
| `cleanup(resource, ctx) -> Future<()>` lifecycle | `resource.rs:47` | §2.2.4 — NO `cleanup` method | 🔴 | **REGRESSION — `cleanup` dropped**. Resources today release pool connections, close DB handles, etc. via `cleanup` when scope ends. Tech Spec offers no equivalent. |
| `ResourceHandler::ResourceConfigureFuture<'a>` / `ResourceCleanupFuture<'a>` dyn boundary | `resource.rs:59-64` | §2.4 `ResourceHandler::execute(ctx, resource_id, input)` only | 🔴 | **REGRESSION at dyn surface**. Tech Spec §2.4 dyn-erased `ResourceHandler` has only `execute` — no configure/cleanup. The dyn-erasure boundary that today enables `Box<dyn Any + Send + Sync>` resource handoff (`resource.rs:60`) is silently dropped; replaced with `resource_id: ResourceId` parameter. Resource lifecycle pattern fundamentally changes from "engine owns, action borrows scoped" to "engine resolves by id, action receives borrow" — but **the new pattern is not specified anywhere** in §2 / §3 / §11. |
| Single-`type Resource` (Config/Instance unification) | `resource.rs:29-35` | §2.2.4 preserves single-`Resource` | ✅ | 🟢 sensible; Phase 0 §6 confirmed sound |

### §1.6 DX traits — `ControlAction` / `PaginatedAction` / `BatchAction` / `WebhookAction` / `PollAction`

| Capability | Production location | Tech Spec coverage | Status | Severity notes |
|---|---|---|---|---|
| 5 DX trait identifiers | `control.rs:945`, `stateful.rs` (Paginated/Batch), `webhook.rs:1852`, `poll.rs:1466` | §2.6 + §9.2 sealed pattern per ADR-0040 | ✅ | Sealing intentional |
| `ControlInput` / `ControlOutcome` types | `control.rs` | NOT enumerated in §9.3 added/removed/reshuffled | 🟡 | Sealed-DX migration target says `#[action(control_flow = ...)]` zone replaces direct impl, but the **input/outcome value types** (the data the engine threads) are not surfaced. Reader can't tell if they survive, are renamed, or vanish. |
| `PageResult<T, C>`, `PaginationState<C>` types | `stateful.rs:84-98` | NOT covered | 🟡 | DX type surface lost in Tech Spec; community plugins authoring `PaginatedAction` cannot tell from spec whether `PageResult` survives or is replaced. |
| `WebhookConfig`, `SignaturePolicy`, `RequiredPolicy`, `SignatureScheme`, HMAC primitives (`verify_*`, `hmac_sha256_compute`) | `webhook.rs` | NOT covered | 🟡 | Webhook hardening cascade marked OUT (§1.2 N7), but the **action-author surface** (`SignaturePolicy::Custom(Arc<dyn Fn>)` and friends) is silent in Tech Spec. The `feedback_active_dev_mode.md` discipline of "if deferring, name where it lives" is met for the security floor but not for the type surface. |
| `PollConfig`, `PollCursor<C>`, `DeduplicatingCursor`, `PollResult`, `EmitFailurePolicy`, `POLL_INTERVAL_FLOOR` | `poll.rs:1466` | §8.1.2 ("PollAction-shaped triggers track cursor position via the underlying `TriggerAction::handle` fire-and-forget event surface; cursor itself is engine-managed") | 🟠 | **Architectural inversion**. Production: cursor lives in the action's state (`PollCursor::checkpoint`, `DeduplicatingCursor::seen` set, in-memory only per restart). Tech Spec §8.1.2: "cursor lives at engine, not action body." This is a fundamental architectural shift — moves cursor ownership across the action/engine boundary. The PollCursor poll-loop semantics (`PollResult::partial`, `max_pages_hint`, backoff/jitter at `PollTriggerAdapter`) require runtime state somewhere. Tech Spec defers shape lock to "CP3 §7" but CP3 §7 closed at FROZEN CP4 without specifying. |
| `impl_paginated_action!` / `impl_batch_action!` macros (DX activation) | `stateful.rs:170` (impl_paginated) | NOT covered — replaced by `#[action(paginated(cursor = ...))]` zone per §2.6 community migration text | 🟢 | Migration target named ("replace direct impl with primary trait + zone") but **specific macro-zone syntax** is "CP3 §7 housekeeping, flag form is CP3-CP4 placeholder" per §15.8 row entry. Honest deferral with named owner. |

### §1.7 Sealed-pattern composition

| Capability | Production location | Tech Spec coverage | Status | Severity notes |
|---|---|---|---|---|
| Five sealed traits via `mod sealed_dx` | NEW (none in production) | §2.6 + §9.2 | ✅ | New surface; intentional per ADR-0040 |

### §1.8 `ActionHandler` enum dispatch

| Capability | Production location | Tech Spec coverage | Status | Severity notes |
|---|---|---|---|---|
| 4-variant enum (Stateless / Stateful / Trigger / Resource) | `handler.rs:39-50` | §2.5 verbatim | ✅ | Preserved |
| `is_*()` predicates | `handler.rs` | NOT named explicitly | 🟡 | Probably preserved by virtue of "current shape preserved"; reader can't confirm |

### §1.9 `ActionResult` / `ActionError`

| Capability | Production location | Tech Spec coverage | Status | Severity notes |
|---|---|---|---|---|
| 10-variant `ActionResult<T>` (Success/Skip/Drop/Continue/Break/Branch/Route/MultiOutput/Wait/Retry/Terminate) | `result.rs:55-224` | §2.7.2 verbatim with new `unstable-terminate-scheduler` flag | ✅ | Complete |
| `ActionError` (RetryHintCode, ValidationReason, two-axis Classify) | `error.rs:1016` | §2.8 confirms preservation | ✅ | "Reference-quality" per rust-senior 02c |
| `From<CredentialAccessError>` / `From<CoreError>` impls | `error.rs:305-333` | NOT named | 🟡 | Probably preserved; reader can't confirm |
| `CredentialRefreshFailed` variant | `error.rs:254-265` | NOT named | 🟡 | Critical for credential cascade; absence in §2.8 enumeration is risky |

---

## §2 Coverage map by adapter

### §2.1 `StatelessActionAdapter<A>`

| Capability | Production location | Tech Spec coverage | Status | Severity notes |
|---|---|---|---|---|
| JSON deserialize input | `stateless.rs:356-383` | §6.1 (depth cap 128 added) + §11.1 narrative | ✅ | Hardened |
| `try_map_output` to serialize output | `stateless.rs` | §11.1 narrative ("Serialize typed output") | ✅ | Preserved |

### §2.2 `StatefulActionAdapter<A>`

| Capability | Production location | Tech Spec coverage | Status | Severity notes |
|---|---|---|---|---|
| Bridge JSON state ↔ typed state | `stateful.rs:561, 573` | §6.1.1 + §8.1.1 | ✅ | Hardened |
| State migration hook (`migrate_state`) | `stateful.rs` | §8.1.1 narrative cite | 🟡 | Per §1.3 above — not in §2.2 |

### §2.3 `TriggerActionAdapter<A>`

| Capability | Production location | Tech Spec coverage | Status | Severity notes |
|---|---|---|---|---|
| Pure delegation (start/stop pass-through) | `trigger.rs:404-471` | §2.2.3 amended + §11.1 | ✅ | Closed |
| Type-erased payload downcast (`Box<dyn Any>` → typed transport event) | `trigger.rs:182-203` | NOT specified | 🔴 | **REGRESSION at adapter layer**. Production adapter receives `TriggerEvent { payload: Box<dyn Any> }`, downcasts to `WebhookRequest` / `KafkaMessage` / etc.; downcast-mismatch returns `ActionError::Fatal`. Tech Spec §2.4 dyn `TriggerHandler::handle` receives `serde_json::Value` — implies JSON deser at engine boundary, but the **adapter's downcast → typed `<Source as TriggerSource>::Event` round-trip** is invisible. Where does deser fail? What is the typed-vs-untyped audit point? Spec silent. |

### §2.4 `ResourceActionAdapter<A>`

| Capability | Production location | Tech Spec coverage | Status | Severity notes |
|---|---|---|---|---|
| Box `A::Resource` to `Box<dyn Any + Send + Sync>` for engine handoff | `resource.rs:69-102, 186-196` | §2.4 `ResourceHandler::execute(ctx, resource_id, input)` — `Box<dyn Any>` boundary disappears | 🔴 | See §1.5 row 4 above |
| Downcast on cleanup with fatal mismatch | `resource.rs` | NOT covered (no cleanup in spec) | 🔴 | Gone with `cleanup` dropping |

### §2.5 `ControlActionAdapter` / `WebhookTriggerAdapter` / `PollTriggerAdapter`

| Capability | Production location | Tech Spec coverage | Status | Severity notes |
|---|---|---|---|---|
| Wrap-to-StatelessHandler / TriggerHandler erasure | `control.rs`, `webhook.rs:585+`, `poll.rs:1078+` | §11.1 + §2.6 sealed-DX adapter pattern | ✅ | Pattern preserved at trait level |
| Webhook double-start rejection (`RwLock<Option<Arc<State>>>`) | `webhook.rs` | §2.2.3 narrative ("Calling `start` twice without an intervening `stop` returns Fatal") | ✅ | Lifted into trait-level invariant |
| PollTriggerAdapter cancel-safe loop with `POLL_INTERVAL_FLOOR=100ms` | `poll.rs` | NOT covered in spec | 🟡 | Per §1.6 row 7 above; floor constant not surfaced |

---

## §3 Specific concern resolutions

### §3.1 start/stop lifecycle decoupling — Tech Spec status post-Q6

**RESOLVED (✅ COVERED).** §2.2.3 amended-in-place 2026-04-25 per §15.10 enactment. `start()` + `stop()` now appear in trait shape with Option (i) rationale. Spike scope narrowed (lifecycle derived from production PASS evidence, not spike-shape change). T7 codemod added to §10.2. Migration coverage adequate.

### §3.2 `start()` Input axis — does Tech Spec acknowledge?

**ACKNOWLEDGED (✅ COVERED) but with refinement gap.** §15.10.2 explicitly REJECTS bundling `start(input: Self::Input)` with Q6 (B1 + B4 sufficient blockers). §2.9.1a/b/c name Configuration vs Runtime Input axis distinction. The user's framing ("RSS url, Kafka channel = configuration; trigger emits events = output") is acknowledged at §2.9.1b. **However, the Configuration carrier paradigm — `&self` fields populated at registration time + `parameters = T` schema zone — is named in narrative but the Tech Spec §2.2.3 trait shape does NOT show example fields or call out the `&self` configuration carrier explicitly.** A reader ports `WebhookAction { webhook_url: String, secret: SecretString }` from production by inferring "fields outside zones pass through" from §4.2 → would benefit from §2.2.3 narrative example. 🟡 documentation gap.

### §3.3 `TriggerHandler` decoupling — Tech Spec collapsed two-layer to one?

**🔴 REGRESSION/INVERSION.** Production has TWO decoupled boundaries:
1. **Trait layer** (`TriggerAction`) — typed for plugin author.
2. **Handler layer** (`TriggerHandler`) — type-erased `TriggerEvent` envelope (`Box<dyn Any>` payload) at the dyn-handler boundary, with adapter-side downcast.

Tech Spec §2.4 collapses dyn-handler to JSON-Value boundary (`fn handle(ctx, event: serde_json::Value)`) AND §2.2.3 trait keeps typed `Source::Event` projection. The **typification path between the two layers** (how does engine source events of type T from `Source: TriggerSource` and feed them as `serde_json::Value` to the dyn handler, then re-typify to `<Source as TriggerSource>::Event` at the typed body?) is **not specified anywhere in §2/§3/§11**.

Phase 0 audit line 125 already flagged this: "this is a deliberate **decoupling** post-v2-spec, not a code bug, but docs should admit it." Tech Spec at FROZEN CP4 does NOT admit it — the Q6 amendment-in-place doc (`15-q6-lifecycle-gap-fix.md`) line 96 admits it parenthetically but Tech Spec proper does not.

### §3.4 `TriggerEvent` type-erased migration to typed `Source::Event` — covered?

**🔴 NOT COVERED.** Per §3.3 — the migration mechanism is undeclared. Q6 line 96 claims "no regression — production already supports event-driven dispatch through the type-erased envelope; cascade typifies it" but Tech Spec §2 / §3 / §10 / §11 contain no:
- T-codemod for `TriggerEvent` consumers
- Specification of where `Box<dyn Any> → <Source as TriggerSource>::Event` downcast lives
- Migration step in §10.4 for plugin authors who today consume `TriggerEvent::downcast::<WebhookRequest>()`
- Sunset story for `TriggerEvent` / `TriggerEventOutcome` types

### §3.5 `StatefulAction` state migration — covered?

**🟡 PARTIALLY COVERED.** §8.1.1 names `migrate_state` in narrative form. **§2.2.2 trait shape does NOT include `init_state()` or `migrate_state()`.** A reader of §2.2.2 alone cannot derive the lifecycle. This is exactly the same class of slip Q6 caught (production has lifecycle method, spec dropped without rationale). Covered partially — the `init_state` method is **completely missing** from Tech Spec; `migrate_state` is mentioned in §8 narrative but not in §2 trait. Should be 🔴 for `init_state` + 🟡 for `migrate_state`.

### §3.6 `ResourceAction` pool integration — covered?

**🔴 NOT COVERED.** §2.2.4 has no `configure` / `cleanup` lifecycle methods. Tech Spec offers `execute(&self, ctx, resource: &Self::Resource, input)` only — implies the engine somehow obtains `Self::Resource` and lends it. **No specification of where the resource is built, when, or how cleanup happens.** N1 (Non-goal) defers `Resource::on_credential_refresh full integration` but does NOT defer the basic configure/cleanup lifecycle — those should be in scope. Q6-equivalent slip class.

---

## §4 PollCursor / State-on-trigger-family analysis

User's specific concern: **does `TriggerAction` need a `type State` analogous to `StatefulAction`?**

### §4.1 Does Tech Spec recognize PollCursor as State-equivalent?

**NO.** §8.1.2 explicitly declares "cursor lives at engine, not action body" — i.e., Tech Spec moves cursor management OUT of the action surface entirely. Production `PollCursor<C>` lives in the `PollAction` sealed-DX surface (`poll.rs:439`), is in-memory per process, and is read/checkpointed by the action body via `&mut PollCursor<Self::Cursor>` in `poll(.., cursor, ..)`.

**Tech Spec §8.1.2 commits ONE sentence ("cursor itself is engine-managed (per Strategy §3.1 component 7 — cluster-mode dedup window, idempotency key)") but does NOT specify the engine-side persistence shape, the transition contract, or migration for existing PollAction implementors who today read `cursor.checkpoint(...)` directly.** 🟠 Architectural inversion at scope; documentation 🟡 at detail level.

### §4.2 Should TriggerAction have `type State` analogous to StatefulAction?

**Position (tech-lead solo):** **NO at primary trait level; YES at PollAction sealed-DX level — which is a property the cascade has already locked but undocumented in Tech Spec §2.2.3.**

Rationale:
1. **Triggers vs Stateful actions have different state semantics.** StatefulAction state is per-execution iterative state (paginate-state, loop-counter); persisted via `ExecutionRepo` per canon §11.3 idempotency. Trigger cursor state is per-trigger-instance (one trigger has one cursor across its lifetime), persisted across process restarts at engine cluster-mode coordination layer. Different lifecycle, different persistence target.
2. **Hoisting `type State` to TriggerAction breaks Source/Event typing.** TriggerAction already carries `type Source: TriggerSource` per spike Probe 2; adding `type State` would force a 4-associated-type signature (`Source`, `Error`, `State`, projected `Event`) — heavier than StatefulAction (3 associated types: `Input`, `Output`, `State`). The shape doesn't honestly read as "trigger" any more.
3. **Cursor state is PollAction-specific.** Webhook triggers don't have cursors. Lifting `type State` to TriggerAction forces every trigger to declare `type State = ()` for non-poll cases — same noise §2.9 REJECTed for hoisting `type Output`.
4. **The right level is PollAction.** PollAction is sealed-DX, wraps TriggerAction, and is the natural carrier for `type Cursor` (what production has). Tech Spec §2.6 / §9.2 lock the sealing but **do NOT enumerate `PollAction::type Cursor` or where cursor flows.**

### §4.3 Migration impact if YES?

**N/A** since recommendation is NO. But documentation impact of locking the existing arrangement explicitly:
- Tech Spec §2.6 should add: `pub trait PollAction: sealed_dx::PollActionSealed + TriggerAction { type Cursor: Serialize + DeserializeOwned + Clone + Send + Sync + 'static; ... }`.
- §8.1.2 should expand the one-sentence treatment to specify cursor's lifecycle (engine-persisted at cluster-mode layer, action body reads via `&mut`-equivalent borrow).
- §10.2 needs a T-codemod for cursor-bearing `PollAction` impls if the cursor-API shape changes.

---

## §5 Severity-tagged findings list

### 🔴 REGRESSIONs

1. **`StatefulAction::init_state(&self) -> Self::State` dropped from §2.2.2** — required engine-driven state construction missing from trait shape. Same class slip as Q6 trigger lifecycle. Action-author surface incomplete.
2. **`ResourceAction::configure / cleanup` lifecycle dropped from §2.2.4** — graph-scoped DI lifecycle methods absent from trait shape; replaced by undeclared `engine resolves resource by id` paradigm with no spec text. Implementer cannot write a working ResourceAction from §2 alone.
3. **`TriggerEventOutcome::{Skip, Emit, EmitMany}` outcome multiplicity lost** — §2.2.3 `handle() -> Result<(), Error>` collapses fan-out path; production action authors who emit N executions per single transport event have no equivalent in spec.
4. **`ResourceHandler` dyn-erasure boundary changes silently** — production `Box<dyn Any + Send + Sync>` resource handoff at `ResourceHandler::configure` is dropped; new `resource_id: ResourceId` paradigm specified in §2.4 with no migration path or rationale.
5. **`TriggerHandler::handle_event` type-erased `Box<dyn Any>` payload boundary** — adapter-side downcast → typed-event dispatch path is not specified in §2.4 or §11; the typed-vs-untyped audit point is invisible.

### 🟠 ARCHITECTURAL INVERSIONs

1. **Cursor ownership shifts from action to engine** (§8.1.2) — PollCursor moves across the action/engine boundary in one sentence; no migration shape for existing `PollAction` impls.
2. **`ResourceAction::Resource: Resource` (new bound)** — forces every existing resource type to acquire a `Credential` associated type binding via the new `Resource` trait; creates hard dependency on credential cascade landing first; only obliquely acknowledged via N1.
3. **TriggerAction event-dispatch typification path** (§3.3 above) — production's two-layer decoupling (typed action / type-erased dyn) is collapsed without spec text on how the boundary works post-cascade.

### 🟡 DOCUMENTATION GAPs

1. **Test assertion macros** (`assert_success!`, `assert_continue!`, `assert_branch!`, etc. — 11 macros at `macros.rs:204`) — preserved in code, not surfaced in §9 prelude or §16 DoD.
2. **`StatefulAction::migrate_state` cited in §8 but not in §2.2.2 trait shape** — implementer-guidance gap.
3. **`ControlInput`, `ControlOutcome`, `PageResult`, `PaginationState`, webhook config types, `PollConfig`/`POLL_INTERVAL_FLOOR`, `EmitFailurePolicy`** — DX type surfaces lost in Tech Spec inventory; readers cannot tell which survive.
4. **`ActionError::From<CredentialAccessError>` / `From<CoreError>` / `CredentialRefreshFailed` variant** — not named in §2.8 enumeration.
5. **TriggerHandler `accepts_events()` engine-gating predicate** — silent.
6. **`#[diagnostic::on_unimplemented]` attributes** on TriggerAction / StatefulAction — DX diagnostic surface absent from spec.
7. **`Configuration` carrier paradigm narrative** — §2.9.1a/b/c name it but §2.2.3 has no example showing `pub webhook_url: String` field pattern.
8. **Type-erased `TriggerEvent` migration to typed `Source::Event`** — Q6 doc admits cascade "typifies it" but Tech Spec proper has no T-codemod, no §10.4 step, no spec text.

### 🟢 INTENTIONAL REMOVALs (verified rationale)

1. **`#[derive(Action)]` macro** — replaced by `#[action]` attribute macro per ADR-0038 §Decision item 1; rationale documented; codemod T1 covers.
2. **`CredentialContextExt::credential<S>()` no-key heuristic** — hard-removed per §6.2 + security-lead VETO; codemod T2.
3. **`Action` direct user impl** — locked behind macro; rationale at §2.1.
4. **Resource `Config`/`Instance` split** — production already removed; preserved.

### ✅ COVERED capabilities (count: ~25)

- Four primary trait identities (Stateless / Stateful / Trigger / Resource) + RPITIT shape
- 4-variant ActionHandler dispatch enum
- 10-variant ActionResult including Retry + Terminate (gated)
- ActionError taxonomy core
- StatefulAction State bound chain
- StatelessAction Input/Output bound lift
- §6 security floor (JSON depth, hard-removal credential, redacted_display, cancellation-zeroize)
- Macro test harness §5 (probes 1-7)
- Codemod runbook §10 (T1-T7) + reverse-deps inventory
- §8.1.1 state JSON persistence narrative
- Sealed-DX pattern at trait level
- Adapter pattern at JSON-erasure boundary (StatelessHandler/StatefulHandler at JSON in/out)
- 9.5 Cross-tenant Terminate boundary
- ADR-0035/0036/0037/0038 composition
- §2.7 wire-end-to-end Retry+Terminate
- Q1 `*Handler` `#[async_trait]` per §15.9
- Q2 / Q3 §2.9 REJECT rationale tightening (4 axes)
- Q6 lifecycle gap (Tech Spec §2.2.3 amended-in-place)

---

## §6 Recommended actions

For each 🔴 / 🟠 / 🟡 finding (Phase 3 scope per orchestrator — not amendment text here):

### 🔴 actions

| Finding | Tech Spec amendment-in-place scope | ADR amendment? | §15 entry needed? | Escalation? |
|---|---|---|---|---|
| 🔴 R1 `init_state` dropped | Add `init_state(&self) -> Self::State` to §2.2.2 trait shape | NO (ADR-0038 §Neutral 2 phrasing covers) | YES — §15.11 enactment record | NO — amendment-in-place precedent (ADR-0035 / Q6) |
| 🔴 R2 ResourceAction `configure`/`cleanup` dropped | Restore `configure(&self, ctx) -> Future<Self::Resource>` + `cleanup(resource, ctx) -> Future<()>` to §2.2.4 OR explicitly document the new "engine resolves" paradigm with full lifecycle spec | Possible ADR — depends on whether spec changes paradigm or restores lifecycle | YES | **POSSIBLY** — paradigm shift would benefit from architect co-decision; if restoration, mechanical |
| 🔴 R3 `TriggerEventOutcome` lost | Add Skip/Emit/EmitMany return shape on `TriggerAction::handle` OR document where fan-out lives engine-side | NO if fan-out is engine-cascade scope; YES if action surface needs the multiplicity | YES | NO if engine-cascade defers |
| 🔴 R4 `ResourceHandler` dyn boundary | Specify `ResourceHandler::configure / cleanup` shape in §2.4 OR explicitly document the resource_id paradigm with full migration | depends on R2 resolution | YES | follows R2 |
| 🔴 R5 TriggerHandler type-erased payload | §2.4 `TriggerHandler::handle` body specifies the `Value → <Source as TriggerSource>::Event` typification location (engine adapter? action adapter?) | NO | YES | NO |

### 🟠 actions

| Finding | Tech Spec amendment-in-place scope | Notes |
|---|---|---|
| 🟠 I1 PollCursor ownership shift | §8.1.2 expand from one-sentence to explicit lifecycle (engine-persisted or in-memory; how PollAction body accesses cursor); add §10 T-codemod | Migration story today is missing |
| 🟠 I2 ResourceAction `Resource: Resource` bound dependency on credential cascade | §1.2 add explicit N (negative) line + §2.2.4 narrative + §16.5 cascade-final precondition update | Acknowledge dependency explicitly |
| 🟠 I3 TriggerAction typification path | §3.3 add narrative section "Trigger event-dispatch typification" specifying the boundary | Closes Q6 secondary gap |

### 🟡 actions

Bundle into a single Tech Spec amendment-in-place pass:
- §2.8 enumerate `ActionError` From-impls + variants explicitly
- §9.3 added-list expand to include all DX type families (`ControlInput`, `ControlOutcome`, `PageResult`, `PaginationState`, webhook types, poll types) with rename/preserve disposition
- §16 DoD add row "test assertion macros (11 macros) preserved at `macros.rs`"
- §2.2.3 add example `&self` configuration field to bind §2.9.1a paradigm narrative to spec text
- §2.2.2 add `migrate_state` to trait shape (closes §1.3 row 2)

---

## §7 Coverage summary

- **Total capabilities catalogued:** ~50 (across 9 traits + adapters + ActionResult + ActionError + DX surfaces)
- **Tech Spec coverage %:** ~50% (25 ✅ / 50 catalogued; remainder split across 5 🔴 + 3 🟠 + 8 🟡 + 4 🟢)
- **🔴 count:** 5 (init_state, ResourceAction configure/cleanup, TriggerEventOutcome multiplicity, ResourceHandler dyn boundary, TriggerHandler type-erased payload)
- **🟠 count:** 3 (PollCursor ownership shift, ResourceAction:Resource dependency, TriggerAction typification path)
- **🟡 count:** 8 (test macros, migrate_state in trait, DX type families, ActionError detail, accepts_events, on_unimplemented, &self field example, TriggerEvent migration text)
- **🟢 count:** 4 (intentional removals with rationale)
- **✅ count:** ~25

**Recommendation: AMENDED-CLOSED** with same amendment-in-place precedent that closed Q1/Q2/Q3/Q6.

The 🔴 set is **mechanical lifecycle slips of the same class as Q6** (production has method, spec dropped without rationale). The Tech Spec's amendment-in-place precedent (ADR-0035 §Status block "canonical-form correction" criterion + §15.9 / §15.5 / §15.10 enactment records) is the right tool: enact 5 amendments-in-place adding `init_state` to §2.2.2, restoring `configure`/`cleanup` to §2.2.4 (or specifying the new paradigm with full lifecycle), specifying the `TriggerEventOutcome` fate, and documenting the type-erased → typed boundary.

The 🟠 set is **architectural inversions that need explicit narrative** rather than re-derivation; they don't require new ADRs, just expansion of §8.1.2 and §2.2.4 N-row text.

The 🟡 set is **bundled documentation work** — one cross-section pass adding ~8 short clarifications to existing sections.

**This is NOT RE-OPENED territory.** No load-bearing decision flips; no ADR supersedes; no spike re-validation needed (production-shape evidence is the source for restoration). Same precedent as Q6 — mechanical drift fix via amendment-in-place.

The cascade should NOT be considered FULLY-CLOSED until at minimum the 5 🔴 amendments-in-place land. The 3 🟠 + 8 🟡 are AMENDED-CLOSED-acceptable as deferred-with-trigger §15 items if owner + sunset window are named.

---

*End of post-closure tech-lead coverage map. Phase 1 of 3.*
