# Post-closure rust-senior production inventory — `nebula-action`

**Scope:** Direct read of production source under `crates/action/src/` at HEAD `193af953` (post-CP4 freeze + 6 amendments). Inventory only; no fixes proposed.

**Files audited (verbatim, all lines):**
- `crates/action/src/trigger.rs` (648 LOC)
- `crates/action/src/webhook.rs` (1853 LOC)
- `crates/action/src/poll.rs` (1466 LOC)
- `crates/action/src/stateful.rs` (905 LOC)
- `crates/action/src/resource.rs` (322 LOC)
- `crates/action/src/control.rs` (947 LOC)
- `crates/action/src/stateless.rs` (603 LOC)
- `crates/action/src/handler.rs` (386 LOC)
- `crates/action/src/context.rs` (excerpts §60–134, §233–550)
- `crates/action/src/capability.rs` (`TriggerHealth` excerpt §43–145)

---

## §1 Per-trait public surface inventory

### §1.1 `StatelessAction` (stateless.rs:71–103)

```rust
#[diagnostic::on_unimplemented(
    message = "`{Self}` does not implement StatelessAction",
    note = "implement the `execute` method with matching Input/Output types"
)]
pub trait StatelessAction: Action {
    type Input: nebula_schema::HasSchema + Send + Sync;
    type Output: Send + Sync;

    fn schema() -> nebula_schema::ValidSchema
    where
        Self: Sized,
    {
        <Self::Input as nebula_schema::HasSchema>::schema()
    }

    fn execute(
        &self,
        input: Self::Input,
        ctx: &(impl ActionContext + ?Sized),
    ) -> impl Future<Output = Result<ActionResult<Self::Output>, ActionError>> + Send;
}
```

- **Supertrait:** `Action` (which transitively requires `DeclaresDependencies`, see all impl blocks across files).
- **Associated types:** `Input: HasSchema + Send + Sync` (no `DeserializeOwned` bound at trait — lifted to adapter, per memory `action_adapter_bound_leak.md`); `Output: Send + Sync`.
- **Methods:** `schema()` (default impl, `Self: Sized` gate) + `execute()` (RPITIT `async fn`, Send-bounded).
- **Bounds shape:** `&self`, `&(impl ActionContext + ?Sized)` — supports both sized and `dyn` contexts.

### §1.2 `StatefulAction` (stateful.rs:34–78)

```rust
#[diagnostic::on_unimplemented(
    message = "`{Self}` does not implement StatefulAction",
    note = "implement `init_state` and `execute` methods with matching Input/Output/State types"
)]
pub trait StatefulAction: Action {
    type Input: nebula_schema::HasSchema + Send + Sync;
    type Output: Send + Sync;
    type State: Serialize + DeserializeOwned + Clone + Send + Sync;

    fn init_state(&self) -> Self::State;

    fn migrate_state(&self, _old: Value) -> Option<Self::State> {
        None
    }

    fn execute(
        &self,
        input: Self::Input,
        state: &mut Self::State,
        ctx: &(impl ActionContext + ?Sized),
    ) -> impl Future<Output = Result<ActionResult<Self::Output>, ActionError>> + Send;
}
```

- **Associated type bound:** `State: Serialize + DeserializeOwned + Clone + Send + Sync` — lifted onto trait directly (contrast with §1.1 where Serde bounds live on adapter only).
- **`migrate_state`:** Default returns `None`. Receives raw `serde_json::Value`, returns typed `Option<Self::State>`. **This is the only state-migration hook in the whole crate.**

### §1.3 `TriggerAction` (trigger.rs:61–73)

```rust
#[diagnostic::on_unimplemented(
    message = "`{Self}` does not implement TriggerAction",
    note = "implement `start` and `stop` methods"
)]
pub trait TriggerAction: Action {
    fn start(
        &self,
        ctx: &(impl TriggerContext + ?Sized),
    ) -> impl Future<Output = Result<(), ActionError>> + Send;

    fn stop(
        &self,
        ctx: &(impl TriggerContext + ?Sized),
    ) -> impl Future<Output = Result<(), ActionError>> + Send;
}
```

- **NO associated types.** No `Input`, no `State`, no `Event`, no `Cursor`. Pure lifecycle trait.
- **Just two methods:** `start(&self, ctx)` and `stop(&self, ctx)`. No `Output`, no `handle_event` at this DX layer.
- **Context:** `TriggerContext`, distinct from `ActionContext`.

### §1.4 `ResourceAction` (resource.rs:36–52)

```rust
pub trait ResourceAction: Action {
    type Resource: Send + Sync + 'static;

    fn configure(
        &self,
        ctx: &(impl ActionContext + ?Sized),
    ) -> impl Future<Output = Result<Self::Resource, ActionError>> + Send;

    fn cleanup(
        &self,
        resource: Self::Resource,
        ctx: &(impl ActionContext + ?Sized),
    ) -> impl Future<Output = Result<(), ActionError>> + Send;
}
```

- **Associated type:** `Resource: Send + Sync + 'static` — single type for both `configure` return + `cleanup` consume. Doc-comment notes earlier `Config`/`Instance` split was abandoned (line 30–35).
- **No reconfigure / refresh / on_credential_refresh hook.** Lifecycle is fixed to two methods.
- **Context:** `ActionContext` (NOT `TriggerContext`) — resources are graph-scoped, not trigger-scoped.

### §1.5 `ControlAction` (control.rs:393–431)

```rust
pub trait ControlAction: Action {
    fn evaluate(
        &self,
        input: ControlInput,
        ctx: &(impl ActionContext + ?Sized),
    ) -> impl Future<Output = Result<ControlOutcome, ActionError>> + Send;
}
```

- **NO associated types.** Hard-coded I/O: `ControlInput` (owns `serde_json::Value`) → `ControlOutcome`.
- **Single method:** `evaluate(&self, input, ctx)`.
- **Erases to `StatelessHandler`** (NOT a separate `ControlHandler`).
- **Public + non-sealed** (line 366) — community plugin crates may impl directly.

### §1.6 `PaginatedAction` (stateful.rs:121–155)

```rust
pub trait PaginatedAction: Action {
    type Input: nebula_schema::HasSchema + Send + Sync;
    type Output: Send + Sync;
    type Cursor: Serialize + DeserializeOwned + Clone + Send + Sync;

    fn max_pages(&self) -> u32 {
        100
    }

    fn fetch_page(
        &self,
        input: &Self::Input,
        cursor: Option<&Self::Cursor>,
        ctx: &(impl ActionContext + ?Sized),
    ) -> impl Future<Output = Result<PageResult<Self::Output, Self::Cursor>, ActionError>> + Send;
}
```

- **DX over `StatefulAction`** via `impl_paginated_action!` macro (stateful.rs:170–227).
- **NOT a subtrait of `StatefulAction`** — relationship is "macro-generated impl," not "trait inheritance."
- **`Cursor` here ≠ `PollCursor`** — this is `StatefulAction`-domain pagination (one-shot stateful), distinct from poll-trigger pagination.

### §1.7 `BatchAction` (stateful.rs:267–301)

```rust
pub trait BatchAction: Action {
    type Input: nebula_schema::HasSchema + Send + Sync;
    type Item: Serialize + DeserializeOwned + Clone + Send + Sync;
    type Output: Serialize + DeserializeOwned + Clone + Send + Sync;

    fn batch_size(&self) -> usize { 50 }

    fn extract_items(&self, input: &Self::Input) -> Vec<Self::Item>;

    fn process_item(
        &self,
        item: Self::Item,
        ctx: &(impl ActionContext + ?Sized),
    ) -> impl Future<Output = Result<Self::Output, ActionError>> + Send;

    fn merge_results(&self, results: Vec<BatchItemResult<Self::Output>>) -> Self::Output;
}
```

- **DX over `StatefulAction`** via `impl_batch_action!` macro (stateful.rs:305–375).
- **`extract_items` is sync.** `process_item` is async (per-item parallelism allowed at impl). `merge_results` sync.
- **`Output` requires `Serialize + DeserializeOwned + Clone`** — bound is on the trait (because state holds `Vec<BatchItemResult<Output>>` and is engine-checkpointed).

### §1.8 `WebhookAction` (webhook.rs:578–663)

```rust
pub trait WebhookAction: Action + Send + Sync + 'static {
    type State: Clone + Send + Sync;

    fn on_activate(
        &self,
        ctx: &(impl TriggerContext + ?Sized),
    ) -> impl Future<Output = Result<Self::State, ActionError>> + Send;

    fn handle_request(
        &self,
        request: &WebhookRequest,
        state: &Self::State,
        ctx: &(impl TriggerContext + ?Sized),
    ) -> impl Future<Output = Result<WebhookResponse, ActionError>> + Send;

    fn on_deactivate(
        &self,
        _state: Self::State,
        _ctx: &(impl TriggerContext + ?Sized),
    ) -> impl Future<Output = Result<(), ActionError>> + Send {
        async { Ok(()) }
    }

    fn config(&self) -> WebhookConfig {
        WebhookConfig::default()
    }
}
```

- **CRITICAL FINDING: this trait has `type State`** — a state associated type sibling to `StatefulAction::State`, but DELIBERATELY without `Serialize`/`Deserialize` bounds (see comment lines 580–585: "If the runtime needs to persist state across restarts, that is the runtime's responsibility (post-v1)").
- **Does NOT extend `TriggerAction`.** The trait bound is `Action + Send + Sync + 'static` — webhook is a **peer DX trait** to `TriggerAction`, NOT a subtrait. This is a load-bearing finding for the orchestrator's lifecycle-decoupling concern.
- **4 methods:** `on_activate` (req), `handle_request` (req), `on_deactivate` (default no-op), `config` (default).
- **State flow:** `on_activate` → produces `State`; adapter stores in `RwLock<Option<Arc<State>>>`; `handle_request` receives `&State`; `on_deactivate` receives owned `State`.
- **`config()` returns `WebhookConfig`** — opaque non-exhaustive bag containing `SignaturePolicy`. ADR-0022 fail-closed default.

### §1.9 `PollAction` (poll.rs:800–868)

```rust
pub trait PollAction: Action + Send + Sync + 'static {
    type Cursor: Serialize + DeserializeOwned + Clone + Default + Send + Sync;
    type Event: Serialize + Send + Sync;

    fn poll_config(&self) -> PollConfig;

    fn validate(
        &self,
        _ctx: &(impl TriggerContext + ?Sized),
    ) -> impl Future<Output = Result<(), ActionError>> + Send {
        async { Ok(()) }
    }

    fn initial_cursor(
        &self,
        _ctx: &(impl TriggerContext + ?Sized),
    ) -> impl Future<Output = Result<Self::Cursor, ActionError>> + Send {
        async { Ok(Self::Cursor::default()) }
    }

    fn poll(
        &self,
        cursor: &mut PollCursor<Self::Cursor>,
        ctx: &(impl TriggerContext + ?Sized),
    ) -> impl Future<Output = Result<PollResult<Self::Event>, ActionError>> + Send;
}
```

- **CRITICAL FINDING: this trait ALSO has state, named `Cursor`.** Bound: `Serialize + DeserializeOwned + Clone + Default + Send + Sync` — strictly STRONGER than `WebhookAction::State` (adds `DeserializeOwned + Default`).
- **Does NOT extend `TriggerAction`.** Trait bound is `Action + Send + Sync + 'static`. **Peer to `TriggerAction`, just like `WebhookAction`.**
- **5 methods:** `poll_config` (sync), `validate` (default), `initial_cursor` (default), `poll` (req). NO `start`/`stop` here — those are owned by `PollTriggerAdapter`.
- **`Event`** has only `Serialize + Send + Sync` bounds — events flow OUT to the workflow as JSON (no round-trip through cursor).

---

## §2 Per-adapter capability inventory

### §2.1 `StatelessActionAdapter` (stateless.rs:331–394)

```rust
pub struct StatelessActionAdapter<A> { action: A }
```

- **Internals:** Borrows `A`. Stateless dispatch.
- **Exposes to engine:** `dyn StatelessHandler`.
- **Bounds at `impl`:** `A: StatelessAction + Send + Sync + 'static`, `A::Input: DeserializeOwned + Send + Sync`, `A::Output: Serialize + Send + Sync`. Serde bounds live HERE, NOT on trait.
- **Invariants:** None at adapter level beyond JSON (de)serialization and `try_map_output`.

### §2.2 `StatefulActionAdapter` (stateful.rs:484–627)

```rust
pub struct StatefulActionAdapter<A> { action: A }
```

- **Internals:** Borrows `A`. State serialized to/from `serde_json::Value` between iterations.
- **Exposes:** `dyn StatefulHandler`.
- **Bounds at `impl`:** `A::Input: DeserializeOwned`, `A::Output: Serialize`, `A::State: Serialize + DeserializeOwned + Clone + Send + Sync`.
- **Invariants:**
  - **Checkpoint-on-error invariant** (lines 584–617): state mutations flushed to JSON BEFORE error propagation. Even fatal errors persist state. Validation-on-input-deser is the only path that skips flush.
  - **`migrate_state` round-trip:** failed migrate-then-serialize is treated as migration failure (`.and_then(... .ok())` line 521–523).
- **State maintained:** None — state is engine-owned `serde_json::Value`, flushed between iterations.

### §2.3 `TriggerActionAdapter` (trigger.rs:404–471)

```rust
pub struct TriggerActionAdapter<A> { action: A }
```

- **Internals:** Pure delegation. No state, no sync primitives, no in-flight tracking.
- **Exposes:** `dyn TriggerHandler`.
- **Bounds:** `A: TriggerAction + Send + Sync + 'static`.
- **Invariants:** None — adapter is a thin translator from `impl TriggerContext` to `dyn TriggerContext`. **`accepts_events()` defaults to `false` from `TriggerHandler`** (trigger.rs:359–361), and `handle_event()` returns `ActionError::fatal("trigger does not accept external events")` (default impl trigger.rs:373–389). So a base `TriggerActionAdapter` is push-event-deaf.

### §2.4 `WebhookTriggerAdapter` (webhook.rs:1008–1327)

```rust
pub struct WebhookTriggerAdapter<A: WebhookAction> {
    action: A,
    config: WebhookConfig,
    state: RwLock<Option<Arc<A::State>>>,
    in_flight: Arc<AtomicU32>,
    idle_notify: Arc<Notify>,
}
```

- **Internals — STATEFUL adapter:**
  - Reads `WebhookAction::config()` once at construction, caches in field (line 1035).
  - `state: RwLock<Option<Arc<State>>>` — parking_lot, NOT tokio. Cleared on `stop`, set on `start`.
  - `in_flight: Arc<AtomicU32>` — tracks outstanding `handle_event` calls.
  - `idle_notify: Arc<Notify>` — wakes `stop()` when in_flight transitions to 0.
- **Exposes:** `dyn TriggerHandler` (with `accepts_events() = true`, line 1183–1185).
- **Bounds:** `A: WebhookAction + Send + Sync + 'static`, `A::State: Send + Sync`.
- **Invariants enforced:**
  - **Double-start rejection** (lines 1097–1100, 1112–1133): `Fatal` if state already present. Ladder: read-check → `on_activate` → write-lock recheck → rollback `on_deactivate` if race lost.
  - **In-flight tracking RAII guard** (lines 1059–1072): `InFlightGuard::drop` decrements + `notify_waiters` on transition-to-zero.
  - **Cancellation-safe dispatch** (lines 1266–1298): `tokio::select!` between `ctx.cancellation()` and handler — sends 503 via response_tx on cancel, 500 on handler error, before propagating.
  - **Adapter NEVER holds `parking_lot::RwLock` guard across `.await`** (comment lines 1238–1240, 1107–1110) — guard is dropped at end of statement.
  - **Downcast invariant** (lines 1219–1228): mismatched payload type is engine-routing bug → `ActionError::fatal`.
  - **`config` NOT exposed via `dyn TriggerHandler`** (lines 1011–1015) — only through concrete `WebhookTriggerAdapter::config()` accessor.
- **Resource ownership:** External webhook registrations (GitHub/Slack/Stripe hooks) — the comment at 1095–1099 explicitly warns about leak risk.

### §2.5 `PollTriggerAdapter` (poll.rs:1051–1466)

```rust
pub struct PollTriggerAdapter<A: PollAction> {
    action: A,
    started: AtomicBool,
    poll_warn: WarnThrottle,
    serialize_warn: WarnThrottle,
    emit_warn: WarnThrottle,
}
```

- **Internals — STATEFUL adapter:**
  - `started: AtomicBool` — sentinel for double-start rejection. `compare_exchange(false, true, AcqRel, Acquire)`.
  - `StartedGuard` RAII (poll.rs:900–906): clears `started` flag when `start()` exits — **defused pattern, NOT `mem::forget`** so `stop() → start()` works.
  - **Three `WarnThrottle` instances** (poll.rs:876–898): rate-limited warn-level logs for poll/serialize/emit failures with 30s cooldown.
  - **Cursor lives on the stack of `start()`'s loop** (poll.rs:1328) — `let mut cursor = self.action.initial_cursor(ctx).await?;`. Adapter does NOT store cursor as a field.
- **Exposes:** `dyn TriggerHandler` (with default `accepts_events = false`).
- **Bounds:** `A: PollAction + Send + Sync + 'static`, `A::Cursor: Send + Sync`, `A::Event: Send + Sync`.
- **Invariants enforced:**
  - **Double-start via atomic CAS** (poll.rs:1310–1318) — different mechanism vs. webhook's `RwLock<Option<...>>`. Failure: `Fatal("poll trigger already started; call stop() and await the task before start() again")`.
  - **Cancel-safe loop** (poll.rs:1336–1419): pre-poll check, `tokio::time::timeout` for poll deadline, `tokio::select!` for sleep vs cancel. **`ctx.cancellation().is_cancelled()` is the cheap fast path** (line 1341).
  - **Cursor rollback policy** (poll.rs:1042–1057 doc): Idle = keep, Ready = advance, Partial = checkpoint, Err = pre-poll.
  - **Identity seed for jitter** (poll.rs:945–967, 1327): FNV-1a over `(action_key, scope.workflow_id, scope.node_key|trigger_id)`. Prevents thundering herd in fleets.
  - **`stop()` is fire-and-forget cancel** (poll.rs:1454–1456): `ctx.cancellation().cancel()` then immediate `Ok(())`. Doc explicitly notes "DOES NOT WAIT" — caller must await spawned task before second `start()`.
  - **Dispatch failure policy enforced at adapter** (poll.rs:1208–1240): three branches for `EmitFailurePolicy`. `StopTrigger` raises fatal from inside resolve_cycle.
- **State maintained:** Cursor lives in stack frame; `started` flag is the only field-level state. **No persistent storage hooks.**

### §2.6 `ResourceActionAdapter` (resource.rs:122–204)

```rust
pub struct ResourceActionAdapter<A> { action: A }
```

- **Internals:** Pure delegation + `Box<dyn Any + Send + Sync>` for resource erasure.
- **Exposes:** `dyn ResourceHandler`.
- **Bounds:** `A: ResourceAction + Send + Sync + 'static`.
- **Invariants enforced:**
  - **Downcast invariant** (lines 195–200): `ActionError::fatal` if engine routes wrong-typed `Box<dyn Any>` to this adapter. Comment notes "engine bug, not user footgun."
  - **`_config: Value` parameter on `configure` is reserved** (lines 150–151) — typed action gets configuration from `ctx`, not from JSON.

### §2.7 `ControlActionAdapter` (control.rs:456–510)

```rust
pub struct ControlActionAdapter<A: ControlAction> {
    action: A,
    cached_metadata: Arc<ActionMetadata>,
}
```

- **Internals — METADATA-MUTATING adapter:**
  - Clones `action.metadata()` at construction, **stamps `category` field** based on output port count (line 469–474).
  - 0 outputs → `ActionCategory::Terminal`; 1+ → `ActionCategory::Control` (control.rs:529–535).
  - Caches in `Arc<ActionMetadata>` so subsequent `metadata()` calls are O(1).
- **Exposes:** `dyn StatelessHandler` (NOT a separate `ControlHandler` trait).
- **Bounds:** `A: ControlAction + Send + Sync + 'static`.
- **Invariants enforced:** Category stamping cannot be forgotten by author.
- **Conversion:** `ControlOutcome → ActionResult<Value>` via `impl From` (control.rs:329–354): Branch→Branch, Route→MultiOutput, Pass→Success, Drop→Drop, Terminate→Terminate.

---

## §3 Cross-trait composition patterns

| Layer | Trait | Storage in `ActionHandler` | DX trait erases to |
|---|---|---|---|
| Primary (engine) | `StatelessHandler` | `Arc<dyn StatelessHandler>` | — |
| Primary (engine) | `StatefulHandler` | `Arc<dyn StatefulHandler>` | — |
| Primary (engine) | `TriggerHandler` | `Arc<dyn TriggerHandler>` | — |
| Primary (engine) | `ResourceHandler` | `Arc<dyn ResourceHandler>` | — |
| DX (author) | `StatelessAction` | via `StatelessActionAdapter` | `StatelessHandler` |
| DX (author) | `StatefulAction` | via `StatefulActionAdapter` | `StatefulHandler` |
| DX (author) | `TriggerAction` | via `TriggerActionAdapter` | `TriggerHandler` |
| DX (author) | `ResourceAction` | via `ResourceActionAdapter` | `ResourceHandler` |
| DX (author) | `WebhookAction` | via `WebhookTriggerAdapter` | `TriggerHandler` (NOT `WebhookHandler` — there is none) |
| DX (author) | `PollAction` | via `PollTriggerAdapter` | `TriggerHandler` (NOT `PollHandler` — there is none) |
| DX (author) | `ControlAction` | via `ControlActionAdapter` | `StatelessHandler` |
| DX (author) | `PaginatedAction` | via `impl_paginated_action!` macro | `StatefulAction` (then `StatefulActionAdapter`) |
| DX (author) | `BatchAction` | via `impl_batch_action!` macro | `StatefulAction` (then `StatefulActionAdapter`) |

**Pattern observations:**

1. **Two distinct DX-over-primary mechanisms:**
   - **Adapter wrap** (Webhook, Poll, Control, Stateless, Stateful, Resource, Trigger): typed DX trait + adapter type that impls the dyn-handler trait directly. Adapter holds state if needed.
   - **Macro-generated impl** (Paginated, Batch): DX trait + macro that emits `impl StatefulAction for $ty`. No new adapter. The author's type literally implements `StatefulAction` after macro expansion. State machinery lives in `PaginationState<C>` / `BatchState<I, T>` carried via `StatefulAction::State`.

2. **`WebhookAction` and `PollAction` are PEERS of `TriggerAction`, not subtraits.** Verified: `pub trait WebhookAction: Action + Send + Sync + 'static` (webhook.rs:578) and `pub trait PollAction: Action + Send + Sync + 'static` (poll.rs:800). Neither has `: TriggerAction` in its bound. They share only the `TriggerHandler` dyn boundary at the engine layer. **This is the lifecycle-decoupling layer the orchestrator was asking about** — the answer is that the DX traits don't share `start/stop` because their adapters absorb that complexity.

3. **`ActionHandler` enum (handler.rs:41–50)** is the engine-level dispatcher. `#[non_exhaustive]`, 4 variants. **No `Webhook(...)` or `Poll(...)` variant** — both flow through `Trigger(Arc<dyn TriggerHandler>)`.

4. **No sealed-pattern enforcement.** `WebhookAction`, `PollAction`, `TriggerAction`, `ControlAction` are all public, non-sealed, externally implementable.

5. **Macro emission: `#[derive(Action)]` does NOT cover the DX traits.** From memory and from the absence of macro-generated `impl WebhookAction`/`impl PollAction` patterns: the proc-macro covers `Action` (metadata) only. The DX traits are written by hand. `impl_paginated_action!` and `impl_batch_action!` are decl-macros invoked separately at the call site after `impl PaginatedAction for X {}` is written.

---

## §4 Lifecycle hooks beyond start/stop

| Trait | Cancellation behavior | Cleanup hook | Reconnect / refresh | Cluster mode |
|---|---|---|---|---|
| `StatelessAction` | Runtime races future vs `ctx.cancellation()` (stateless.rs:42–46). Author writes nothing. | None | None | None |
| `StatefulAction` | Same as stateless (engine drives `tokio::select!`). `StatefulHandler` doc explicitly says "runtime drops the execute future" mid-await (stateful.rs:444–456). | None — engine does nothing on iter end. | None | None |
| `TriggerAction` (base) | Author-driven via `ctx.cancellation().cancelled()` in `start()` body. | `stop()` is the only cleanup hook. | None | None |
| `ResourceAction` | Engine cancels mid-await; no specific hook. | `cleanup(self.resource, ctx)` is paired with `configure`. | None — no `reconfigure` or `on_credential_refresh`. | None |
| `ControlAction` | Engine races `evaluate` vs cancel (control.rs:391–392 doc). | None | None | None |
| `PaginatedAction` | Inherits StatefulAction. | None | None | None |
| `BatchAction` | Inherits StatefulAction. Per-item failure does NOT abort batch (only `is_fatal` aborts; stateful.rs:347–356). | None | None | None |
| `WebhookAction` | `WebhookTriggerAdapter` enforces 503 + `ActionError::retryable` on cancel mid-request (webhook.rs:1268–1278). `on_deactivate` waits for in-flight to drain. | `on_deactivate(state, ctx)` paired with `on_activate`. | **No explicit reconnect hook.** No `on_credential_refresh`. | None — no `IdempotencyKey`, no `on_leader_*`, no `dedup_window` at this layer. |
| `PollAction` | `PollTriggerAdapter` runs `tokio::select!` over cancel + sleep (poll.rs:1415–1418). Per-cycle `tokio::time::timeout` (poll.rs:1354–1365). | `stop()` triggers cancel; cursor evaporates with task. | None — no reconnect hook. | None — no leader gate, no global dedup. (`DeduplicatingCursor` is in-process only.) |

**Aggregated findings:**

- **No `on_credential_refresh` hook on any trait.** A credential rotation requires the trigger to be torn down and re-activated.
- **No cluster-mode hooks.** No `IdempotencyKey`, no `on_leader_elected`, no `dedup_window`. Webhook/poll dedup is local to the adapter (webhook: in-flight counter; poll: `DeduplicatingCursor`).
- **Cleanup story is asymmetric:** `WebhookAction::on_deactivate(state)` and `ResourceAction::cleanup(resource)` consume their state. `StatefulAction` and `PollAction` have no end-of-life cleanup hook — state and cursor disappear with the task.
- **Cancellation contract differs:**
  - Stateless / Stateful / Control: engine drives, runtime drops the future at `.await`.
  - Webhook (event handler): adapter enforces 503-on-cancel before propagating retryable.
  - Poll: adapter drives the loop and observes the token.
  - Resource: engine drives `configure`/`cleanup`; no special cancel handling at adapter.

---

## §5 PollCursor / DeduplicatingCursor State analog analysis

**Question (from orchestrator):** *Is `PollCursor` a `StatefulAction::State` analog? Should `TriggerAction` have an explicit `State` associated type?*

### §5.1 `PollCursor<C>` — what it actually is (poll.rs:439–477)

```rust
pub struct PollCursor<C> {
    current: C,
    checkpoint: C,
}

impl<C: Clone> PollCursor<C> {
    pub fn new(cursor: C) -> Self { ... }
    pub fn checkpoint(&mut self) { self.checkpoint = self.current.clone(); }
    pub(crate) fn rollback(&mut self) { self.current = self.checkpoint.clone(); }
    pub(crate) fn into_current(self) -> C { self.current }
}

impl<C> Deref for PollCursor<C> { type Target = C; ... }
impl<C> DerefMut for PollCursor<C> { ... }
```

**`PollCursor` is NOT the cursor itself.** It is a **per-cycle wrapper** that lives only inside the body of one `poll()` call. It carries:
- The actual cursor (`current: C`)
- A snapshot of the cursor as of the last successfully processed page (`checkpoint: C`)
- `Deref`/`DerefMut` so the action body can read/write the cursor transparently.

The wrapper is constructed fresh each cycle by `PollTriggerAdapter::start()` at poll.rs:1351–1352:
```rust
let pre_poll = cursor.clone();
let mut poll_cursor = PollCursor::new(cursor);
```

After the cycle, the adapter unwraps via `into_current()` (poll.rs:1090, 1110, 1196).

### §5.2 The actual cursor `C` lives in `PollTriggerAdapter::start()` stack frame

Look at poll.rs:1328:
```rust
let mut cursor = self.action.initial_cursor(ctx).await?;
```

`cursor: A::Cursor` is a **local variable in the `start()` async block.** It is **NOT a field of `PollTriggerAdapter`** (compare with `WebhookTriggerAdapter::state: RwLock<Option<Arc<A::State>>>` field).

**Consequence:** When `start()` returns (cancellation or fatal), the cursor is dropped with the stack frame. There is NO place in `PollTriggerAdapter` that holds the cursor across an exit-and-restart cycle. The doc at poll.rs:732–761 acknowledges this loudly: "Cursor state is in-memory only" + "On process restart, `initial_cursor` is called again."

### §5.3 Is `PollCursor::C` the moral equivalent of `StatefulAction::State`?

**Yes, semantically — but with markedly different machinery.**

| Aspect | `StatefulAction::State` | `PollAction::Cursor` |
|---|---|---|
| Trait bound | `Serialize + DeserializeOwned + Clone + Send + Sync` | `Serialize + DeserializeOwned + Clone + Default + Send + Sync` (adds `Default`) |
| Persistence | Engine-checkpointed as `serde_json::Value` between every iteration (stateful.rs:594–617) | **In-memory only.** Lives on `start()`'s stack. No engine persistence. |
| Migration hook | `migrate_state(Value) -> Option<Self::State>` (stateful.rs:64–66) | **NONE.** No equivalent. |
| Rollback model | Engine-driven: `Retryable` from action → engine retries with the just-checkpointed state. Mutations before `Err` ARE persisted. | Adapter-driven: 4-way decision tree based on `PollOutcome` variant + `EmitFailurePolicy` (poll.rs:1042–1057). Pre-poll snapshot lives in stack. |
| Snapshot before mutation | Engine clones state JSON before iter (implicit via deserialize) | Adapter clones via `pre_poll = cursor.clone()` (poll.rs:1351) AND wraps in `PollCursor::new` which seeds `checkpoint = current.clone()` |
| Author API | `&mut Self::State` parameter on `execute` | `&mut PollCursor<Self::Cursor>` parameter on `poll` (with Deref/DerefMut to inner) |
| Restart semantics | Engine re-deserializes from last checkpoint on next iter | `initial_cursor()` re-called from scratch on every process start |

### §5.4 `DeduplicatingCursor<K, C>` (poll.rs:553–719)

```rust
pub struct DeduplicatingCursor<K, C> {
    pub inner: C,
    order: VecDeque<K>,
    lookup: HashSet<K>,
    max_seen: usize,
}
```

- **Custom `Serialize`/`Deserialize`** (poll.rs:569–620): on-the-wire shape is `{ inner, seen: VecDeque<K>, max_seen }`. Deserialization rebuilds the `HashSet` from `order`.
- **Bounded FIFO**: `max_seen` cap enforced via `try_insert` (poll.rs:705–718).
- **Treated as a normal `PollAction::Cursor`** — the type `DeduplicatingCursor<K, C>` plugs into `type Cursor = ...` and benefits from cursor checkpoint semantics.
- **Has `is_new`, `mark_seen`, `filter_new`, `seen_count`, `clear_seen`, `with_max_seen` accessors** for action authors.

### §5.5 Should `TriggerAction` have an explicit `State` associated type?

**Inventory only — not proposing answers, just enumerating evidence:**

- **Three different "trigger state" shapes already exist in production:**
  1. `WebhookAction::State` (no Serde bounds; engine does not persist; adapter holds in `RwLock<Option<Arc<...>>>`).
  2. `PollAction::Cursor` (Serde-bound; lives in `start()` stack; per-cycle `PollCursor` wrapper; no engine persistence).
  3. Base `TriggerAction` (NO state at all; just `start`/`stop`).
- **`PollCursor` is NOT itself the state** — it is a per-cycle wrapper. The state is the bare `C`. Conflating these is a real risk for designers reading the cascade.
- **Migration hook only exists on `StatefulAction`.** Webhook and Poll have none. Webhook deliberately rejects persistence; Poll explicitly notes it as future work (poll.rs:733–761).
- **`Default` bound difference between `PollAction::Cursor` and `WebhookAction::State`** is load-bearing: `PollAction::initial_cursor` has a default impl that returns `Self::Cursor::default()` — that default impl REQUIRES `Default`. Webhook has no equivalent default-state-from-thin-air path because `on_activate` is required.

### §5.6 What was already noted in CASCADE_LOG

The Tech Spec freeze (CP4) and post-freeze amendments did not surface these three different state shapes as a unified abstraction — nor explicitly call them divergent. The orchestrator's framing ("PollCursor on PollAction = State analog") is correct in spirit but understates the asymmetry: there are **three** trigger-family-state shapes (webhook, poll cursor, none), each with different persistence + migration + rollback discipline. Whether this is a coverage gap in §2/§3 of the Tech Spec is for tech-lead to evaluate.

---

## §6 Specific concerns from orchestrator

### §6.1 start/stop lifecycle decoupling — Tech Spec layered explanation

**Production reality:**

- **`TriggerAction::start/stop`** (trigger.rs:61–73) — DX-layer methods. Just two functions, no state, no events.
- **`TriggerHandler::start/stop`** (trigger.rs:328–353) — dyn-compat-layer methods. Pin<Box<dyn Future>> at the engine boundary.
- **Three production paths from DX to dyn:**
  1. `TriggerAction → TriggerActionAdapter → TriggerHandler` (trigger.rs:404–471): pure delegation.
  2. `WebhookAction → WebhookTriggerAdapter → TriggerHandler` (webhook.rs:1008–1319): adapter HAS NO `start/stop` from `WebhookAction` to delegate to. **`WebhookAction` has `on_activate/on_deactivate` instead.** The adapter translates: `start()` calls `on_activate` and stores `State`; `stop()` waits for in-flight, takes State, calls `on_deactivate`.
  3. `PollAction → PollTriggerAdapter → TriggerHandler` (poll.rs:1290–1458): adapter ALSO has no `start/stop` from `PollAction` to delegate to. **`PollAction` has only `poll()`.** The adapter's `start()` runs the loop inline; `stop()` cancels the token.
- **Webhook and Poll DX traits do NOT inherit `TriggerAction`.** They are peer DX traits, not subtraits. Q6's "added start/stop to TriggerAction" amendment refers ONLY to the base `TriggerAction` trait — webhook/poll authors continue to write `on_activate`/`on_deactivate` (webhook) or `poll`/`poll_config` (poll) and never see `start`/`stop`.

**The Tech Spec layered model that production embodies:**
```
DX layer:        [TriggerAction]   [WebhookAction]   [PollAction]   [ControlAction]
                  start, stop      on_activate,      poll,          evaluate
                                   handle_request,   poll_config
                                   on_deactivate

Adapter layer:  TriggerActionAdapter   WebhookTriggerAdapter   PollTriggerAdapter   ControlActionAdapter
                  pure delegate         RwLock state            atomic + warns        metadata stamp
                                       in-flight counter        timeout + jitter      desugar Outcome

Dyn layer:       TriggerHandler                                         StatelessHandler
                  start, stop, accepts_events, handle_event              metadata, execute

Engine layer:    ActionHandler::Trigger(Arc<dyn TriggerHandler>)         ActionHandler::Stateless(Arc<dyn ...>)
```

### §6.2 start() Input requirements — production examples

**`TriggerAction::start` does NOT take any user-supplied input.** Signature is `fn start(&self, ctx: &(impl TriggerContext + ?Sized))` — only `&self` and `ctx`.

**Where author-supplied configuration actually lives in production:**
- **Struct fields on the action type.** Example `struct GitHubWebhook { secret: Vec<u8> }` (webhook.rs:557): the secret is a struct field, supplied by the author when the action is constructed (typically by `nebula-cli` at deploy time from the workflow definition).
- **`ctx.credentials()` lookup** at `start`/`on_activate` time (e.g., `verify_hmac_sha256(req, &self.secret, "X-Hub-Signature-256")` line 566 — secret is on `self`).
- **`ctx.webhook_endpoint()`** — `WebhookEndpointProvider` capability, queried at `on_activate` to get the public URL the trigger should register with the upstream provider (webhook.rs:932–984).
- **`PollAction::initial_cursor(ctx)`** receives `ctx` — typical pattern: read "now" timestamp via `ctx.clock()` to seed cursor at present moment instead of beginning of time.

**§2.9 "REJECT" rationale held up because it concerned `handle()`-style parameter input.** The orchestrator's concern is real but the answer in production is: **`start()` flows config from struct fields; `handle()`-style input is not a `start` concept; per-event payload is `WebhookRequest` carried in `TriggerEvent`.**

### §6.3 TriggerHandler decoupling — production two-layer pattern

The two layers in production are:
- **`TriggerHandler` is the dyn-compat trait** (trigger.rs:276–390) that the engine sees. Methods: `metadata`, `start`, `stop`, `accepts_events` (default false), `handle_event` (default fatal-error).
- **DX-layer traits** (`TriggerAction`, `WebhookAction`, `PollAction`) are author-facing.
- **Adapters are the only impl path** between DX and dyn — the doc (trigger.rs:282–303) classifies adapters into "Setup-and-return" (Shape 1, webhook) and "Run-until-cancelled" (Shape 2, poll). The split was named at CP3 and is preserved in the freeze.

`accepts_events` as the gate between push-driven and pull-driven (webhook returns `true`, poll keeps default `false`) is the load-bearing decoupling: a poll trigger never has `handle_event` called by the engine, so its pure-loop shape is honest.

### §6.4 TriggerEvent type-erased payload — migration to typed Source::Event

**Current production:**

```rust
pub struct TriggerEvent {
    id: Option<String>,
    received_at: SystemTime,
    payload: Box<dyn Any + Send + Sync>,
    payload_type: TypeId,
    payload_type_name: &'static str,
}
```

The doc-comment (trigger.rs:86–96) explicitly defends the type-erasure: "Earlier versions of this type were an HTTP request in disguise..." The TypeId + name are captured at construction so the adapter can produce a meaningful diagnostic on mismatch.

The downcast surface (trigger.rs:182–202) returns `Result<(Option<String>, SystemTime, T), Self>` — gives the failed envelope back to the caller for diagnostics.

**Whether a typed `Source::Event` migration would simplify this is a design-question, not an inventory finding.** Production currently relies on the type-erased shape so `TriggerHandler` can stay dyn-compatible.

### §6.5 StatefulAction state migration — versioning hooks

Sole hook: `StatefulAction::migrate_state(&self, old: Value) -> Option<Self::State>` (stateful.rs:64–66).

- Default impl returns `None` → engine surfaces deserialization error as `ActionError::Validation` (stateful.rs:519–524).
- Adapter calls `migrate_state` when `serde_json::from_value::<A::State>(state.clone())` fails (stateful.rs:573–582).
- Migration is one-shot per-iteration: there's no version number, no migration chain, no upgrade-path hint. Author is responsible for fanning out their own versioning logic inside `migrate_state`.

**No equivalent hook on `WebhookAction::State` or `PollAction::Cursor`.** Webhook state is ephemeral; poll cursor is also in-memory only. **This is the asymmetry.**

### §6.6 ResourceAction pool integration

`ResourceAction` is the slimmest of the lifecycle traits:
- `configure(&self, ctx) -> Future<Resource>`
- `cleanup(&self, resource, ctx) -> Future<()>`

The doc (resource.rs:30–35) explicitly notes the rejected `Config`/`Instance` split. Single `Resource` type round-trips through the boxed `dyn Any` and gets downcast on `cleanup`.

**No pool integration at trait layer.** A `ResourceAction` that wants connection pooling implements its `Resource` type as a pool handle (e.g., `Arc<Pool>`), and `cleanup` triggers pool-shutdown. The trait does not know about pool concepts.

**No `reconfigure` hook**, no credential refresh, no health check. The resource is treated as opaque from `configure` to `cleanup`.

---

## §7 Inventory summary table

| Capability | Production has? | Tech Spec covers? | Phase 0/1 surfaced? | Severity if gap |
|---|---|---|---|---|
| Two-layer DX/Handler split (DX trait + Adapter + Handler trait) | YES (all 7 traits) | Per CP1/CP2 docs, yes (§2/§4) | Yes | — |
| `TriggerAction` base trait (start/stop only) | YES (post-Q6 amendment) | YES (Q6 amendment) | Q6 added | — |
| `WebhookAction` peer DX trait (NOT subtrait of TriggerAction) | YES (line 578) | Per spec author-claim, yes | YES | LOW — already known |
| `PollAction` peer DX trait (NOT subtrait of TriggerAction) | YES (line 800) | Per spec author-claim, yes | YES | LOW — already known |
| `WebhookAction::State` (Clone+Send+Sync, no serde) | YES | Verify in §3 | UNKNOWN | MEDIUM if missed |
| `PollAction::Cursor` (Serialize+DeserializeOwned+Clone+Default+Send+Sync) | YES | Verify in §3 | UNKNOWN | MEDIUM if missed |
| `PollCursor<C>` per-cycle wrapper (with checkpoint) | YES | UNKNOWN — orchestrator flagged | NO | MEDIUM — the `checkpoint`/`rollback` semantics are load-bearing for cursor authors |
| `DeduplicatingCursor<K, C>` | YES | UNKNOWN | UNKNOWN | LOW — leaf utility |
| `StatefulAction::migrate_state` hook | YES | Verify in §3 | UNKNOWN | MEDIUM if missed |
| Equivalent migration hook on `WebhookAction`/`PollAction` | **NO** | NO | NO | LOW (deliberately not present per webhook docs) |
| `WebhookTriggerAdapter` double-start rejection + rollback dance | YES | UNKNOWN — implementation detail | UNKNOWN | LOW |
| `WebhookTriggerAdapter` in-flight counter + Notify (NOT yield_now) | YES (M1 fix doc) | UNKNOWN | UNKNOWN | LOW — adapter-internal |
| `PollTriggerAdapter` AtomicBool + StartedGuard (NOT mem::forget) | YES | UNKNOWN | UNKNOWN | LOW |
| `PollTriggerAdapter` per-trigger jitter seed via FNV-1a over (action_key, scope) | YES (poll.rs:945) | UNKNOWN | UNKNOWN | LOW |
| `PollConfig::validate_and_clamp` warn-and-degrade | YES | UNKNOWN | UNKNOWN | LOW |
| `EmitFailurePolicy` (DropAndContinue/RetryBatch/StopTrigger) | YES | UNKNOWN | UNKNOWN | LOW |
| `WarnThrottle` (30s cooldown) for noisy logs | YES (poll only) | UNKNOWN | UNKNOWN | LOW |
| `ResourceAction::Config`/`Instance` split | **NO — removed** | UNKNOWN | UNKNOWN — was earlier | NONE |
| `TransactionalAction` three-phase saga | **NO — removed 2026-04-10 (M1)** | should not cover | UNKNOWN | NONE |
| `on_credential_refresh` hook on any trait | **NO** | UNKNOWN | UNKNOWN | UNKNOWN — depends on whether cred rotation is a v1 promise |
| Cluster-mode hooks (IdempotencyKey, on_leader_*, dedup_window) | **NO** | should not cover (post-v1) | UNKNOWN | NONE |
| `ControlAction` desugars to `StatelessHandler` (no `ControlHandler`) | YES | Verify in §3 | UNKNOWN | LOW |
| `ControlActionAdapter` auto-stamps `ActionCategory` from output count | YES | UNKNOWN | UNKNOWN | LOW |
| `WebhookEndpointProvider` capability (lives on `TriggerContext` via `HasWebhookEndpoint`) | YES | UNKNOWN | UNKNOWN | MEDIUM — load-bearing for webhook URL propagation |
| `accepts_events`/`handle_event` default behaviors on `TriggerHandler` | YES (default false / default fatal) | UNKNOWN | UNKNOWN | LOW |
| `PollAction::validate(ctx)` pre-loop hook | YES (default no-op, line 822–827) | UNKNOWN | UNKNOWN | LOW |
| `PollAction::initial_cursor(ctx)` async customization (not just `Default`) | YES (line 837–842) | UNKNOWN | UNKNOWN | LOW |
| `WebhookAction::config()` returning `WebhookConfig` (signature policy at trait surface) | YES (line 660–662) | YES (ADR-0022) | YES | — |
| `WebhookConfig` `#[non_exhaustive]` for future expansion | YES (line 686) | YES | YES | — |
| HMAC primitives (`verify_hmac_sha256`, `_base64`, `_with_timestamp`, `hmac_sha256_compute`, `verify_tag_constant_time`) | YES | UNKNOWN | UNKNOWN | LOW — leaf utility |
| Body-cap, header-cap, JSON depth-cap on `WebhookRequest` | YES (lines 91–104, 326–349) | YES | YES | — |
| `WebhookResponse::Accept`/`Respond` split (HTTP response orthogonal to workflow emit) | YES | UNKNOWN | UNKNOWN | MEDIUM — load-bearing for Slack URL verification class |
| `TriggerEventOutcome` (Skip/Emit/EmitMany) | YES | YES | YES | — |
| `TriggerEvent` typed-erasure boundary with TypeId/type_name diagnostics | YES | YES | YES | — |

**Legend for "Tech Spec covers?":** I have not re-audited the frozen Tech Spec against this inventory; that's tech-lead's coverage-map task per orchestrator's split. UNKNOWN means "not verifiable from this code-only inventory pass."
