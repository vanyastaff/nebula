//! Final shape v3 — post-amendment trait shapes (Q1 + Q6 + Q7 + Q8).
//!
//! Validated by spike iter-3 against Tech Spec FROZEN CP4 + 4 amendment
//! rounds. The original spike (commit `c8aef6a0`) PASS verdict for
//! shape-only validation stands; iter-3 = update + compose verification.
//!
//! Pre-amendment shape: see `final_shape_v2.rs` (Phase 4 spike commit
//! `c8aef6a0`). v2 stays untouched — historical artefact.
//!
//! Iter-3 compose validation: standalone crate
//! `scratch/spike-iter-3-shape/` on isolated worktree branch
//! `worktree-agent-a3ec73dbf722f0095`, commit `10b24616`. Eight compose
//! probes; clean `cargo check` under workspace toolchain pin 1.95.0.
//!
//! Amendment provenance:
//!
//! - **Q1** per Tech Spec §15.9 enactment: §2.4 *Handler companion traits
//!   adopt `#[async_trait::async_trait]` per ADR-0024 §Decision items 1+4;
//!   §2.3 `BoxFut` alias survives but scope narrowed to `SlotBinding::resolve_fn`
//!   HRTB only.
//! - **Q6** per Tech Spec §15.10 enactment: §2.2.3 `TriggerAction` gains
//!   `start` / `stop` lifecycle methods adjacent to `handle()`.
//! - **Q7** per Tech Spec §15.11 enactment, six restorations:
//!   - R1: §2.2.2 `StatefulAction` restores `init_state` + `migrate_state`.
//!   - R2: §2.2.4 `ResourceAction` restores `configure` / `cleanup` paradigm;
//!     spurious `execute` / `Input` / `Output` dropped.
//!   - R3: §2.2.3 `TriggerAction::handle` returns `TriggerEventOutcome::{Skip,
//!     Emit, EmitMany}` multiplicity; `accepts_events()` predicate added.
//!   - R4: §2.4 `ResourceHandler` uses `Box<dyn Any + Send + Sync>` resource
//!     handoff; spurious `execute` dropped.
//!   - R5: §2.4 `TriggerHandler` adopts `TriggerEvent` envelope (type-erased
//!     `Box<dyn Any>` payload + `TypeId` diagnostic); §3.5 typification path.
//!   - R6: §2.6 `WebhookAction` / `PollAction` bound to `Action` (NOT
//!     `TriggerAction`) — sealed-DX peer framing per production reality.
//! - **Q8** per Tech Spec §15.12 enactment, five amendments (3 affect shape):
//!   - F2: §2.2.3 `TriggerAction::idempotency_key()` hook default-opt-in.
//!   - F9: §3.6.1 `ActionMetadata::max_concurrent: Option<NonZeroU32>`.
//!   - F12: §3.6.2 `NodeDefinition::action_version: SemVer`.
//!   - F13: §3.7 four engine cluster-mode trait placeholders (doc-only):
//!     `CursorPersistence`, `LeaderElection`, `ExternalSubscriptionLedger`,
//!     `ScheduleLedger`.

// ============================================================
// 1. CredentialRef<C> — Credential Tech Spec §3.5 typed handle
//    (UNCHANGED from v2)
// ============================================================

use std::marker::PhantomData;

#[derive(Clone, Eq, PartialEq, Hash)]
pub struct CredentialRef<C: ?Sized> {
    pub key: CredentialKey,
    _t: PhantomData<fn() -> C>,
}

impl<C: ?Sized> CredentialRef<C> {
    pub fn from_key(key: CredentialKey) -> Self {
        Self { key, _t: PhantomData }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct CredentialKey(pub std::sync::Arc<str>);

// ============================================================
// 2. SlotBinding — Credential Tech Spec §3.4 line 869 verbatim
//    (UNCHANGED from v2)
// ============================================================

use std::pin::Pin;

/// Per Tech Spec §2.3 (Q1 amendment) — `BoxFut` alias survives, scope
/// narrowed to `SlotBinding::resolve_fn` HRTB only. The four `*Handler`
/// per-method async returns moved to `#[async_trait]` per ADR-0024 §Decision
/// items 1+4 (see §2.4 below). The two shapes are structurally distinct:
/// `#[async_trait]` rewrites `async fn` in trait into `Pin<Box<dyn Future>>`
/// returns at the macro layer; HRTB fn-pointer signatures (this alias's use
/// site) are not method bodies and are not rewritten by the macro.
pub type BoxFuture<'a, T> = Pin<Box<dyn core::future::Future<Output = T> + Send + 'a>>;

/// HRTB fn-pointer — load-bearing shape per credential Tech Spec §3.4 line 869.
/// Single 'ctx lifetime per 02c §6 modernization.
/// Cannot be `async fn` pointer (no such syntax on 1.95).
pub type ResolveFn = for<'ctx> fn(
    ctx: &'ctx CredentialContext<'ctx>,
    key: &'ctx SlotKey,
) -> BoxFuture<'ctx, Result<ResolvedSlot, ResolveError>>;

/// Macro-emitted as `&'static [SlotBinding]` per action.
/// Must be `Copy + 'static` for static-slice storage.
#[derive(Clone, Copy, Debug)]
pub struct SlotBinding {
    pub field_name: &'static str,
    pub slot_type: SlotType,
    pub resolve_fn: ResolveFn,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct SlotKey {
    pub credential_key: String,
    pub field_name: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SlotType {
    DirectType,
    ServiceCapability { capability: Capability },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Capability {
    Bearer,
    Basic,
    OAuth2,
}

// Placeholders — defined in surrounding crate.
pub struct CredentialContext<'a>(PhantomData<&'a ()>);
pub enum ResolvedSlot {
    Bearer { /* SecretString */ },
    Basic { /* ... */ },
    OAuth2 { /* ... */ },
}
#[derive(Debug)]
pub enum ResolveError {
    NotFound { key: String },
    WrongType { key: String, expected: &'static str },
    StateLoad { key: String, reason: String },
}

// ============================================================
// 3. SchemeGuard<'a, C> — credential Tech Spec §15.7 line 3394-3429
//    (UNCHANGED from v2 — credential-cascade artefact)
// ============================================================

use std::ops::Deref;
use zeroize::Zeroize;

pub trait Credential: Send + Sync + 'static {
    type State: Send + Sync + 'static;
    type Scheme: Send + Sync + 'static;
    const KEY: &'static str;
    const DYNAMIC: bool = false;
    const LEASE_TTL: Option<std::time::Duration> = None;
    fn project(state: &Self::State) -> Self::Scheme;
}

/// !Clone, ZeroizeOnDrop, Deref<Target = Scheme>.
/// Lifetime 'a is pinned by `engine_construct(scheme, &'a ctx)` —
/// see iter-3 refinement (§15.7 line 3503-3516).
pub struct SchemeGuard<'a, C: Credential>
where
    C::Scheme: Zeroize,
{
    scheme: C::Scheme,
    _lifetime: PhantomData<&'a ()>,
}

impl<'a, C: Credential> SchemeGuard<'a, C>
where
    C::Scheme: Zeroize,
{
    pub(crate) fn engine_construct(
        scheme: C::Scheme,
        _pin: &'a CredentialContext<'a>,
    ) -> Self {
        Self { scheme, _lifetime: PhantomData }
    }
}

impl<'a, C: Credential> Deref for SchemeGuard<'a, C>
where
    C::Scheme: Zeroize,
{
    type Target = C::Scheme;
    fn deref(&self) -> &Self::Target {
        &self.scheme
    }
}

impl<'a, C: Credential> Drop for SchemeGuard<'a, C>
where
    C::Scheme: Zeroize,
{
    fn drop(&mut self) {
        self.scheme.zeroize();
    }
}

// ============================================================
// 4. SchemeFactory<C> — credential Tech Spec §15.7 line 3438-3447
//    (UNCHANGED from v2 — credential-cascade artefact)
// ============================================================

use std::sync::Arc;

#[derive(Debug)]
pub enum AcquireError {
    ResolveFailed(String),
    RefreshExhausted,
}

pub struct SchemeFactory<C: Credential>
where
    C::Scheme: Zeroize,
{
    inner: Arc<dyn Fn() -> BoxFuture<'static, Result<C::Scheme, AcquireError>> + Send + Sync>,
}

impl<C: Credential> Clone for SchemeFactory<C>
where
    C::Scheme: Zeroize,
{
    fn clone(&self) -> Self {
        Self { inner: Arc::clone(&self.inner) }
    }
}

impl<C: Credential> SchemeFactory<C>
where
    C::Scheme: Zeroize,
{
    pub async fn acquire<'a>(
        &'a self,
        ctx: &'a CredentialContext<'a>,
    ) -> Result<SchemeGuard<'a, C>, AcquireError> {
        let scheme = (self.inner)().await?;
        Ok(SchemeGuard::engine_construct(scheme, ctx))
    }
}

// ============================================================
// 5. Action trait family — RPITIT typed surface
// ============================================================

use std::any::{Any, TypeId};
use std::future::Future;
use std::num::NonZeroU32;
use std::time::SystemTime;

use serde::{de::DeserializeOwned, Deserialize, Serialize};

pub struct ActionContext<'a> {
    pub creds: &'a CredentialContext<'a>,
}

// HasSchema stand-in (real one lives in nebula-schema per Tech Spec §2.2.1).
pub trait HasSchema {
    fn schema() -> serde_json::Value;
}

// ── §2.2.1 StatelessAction (UNCHANGED from v2 shape; Tech Spec lifted
//    HasSchema + DeserializeOwned/Serialize bounds onto trait per CP1) ──

pub trait StatelessAction: Send + Sync + 'static {
    type Input: HasSchema + DeserializeOwned + Send + 'static;
    type Output: Serialize + Send + 'static;
    type Error: std::error::Error + Send + Sync + 'static;
    fn execute<'a>(
        &'a self,
        ctx: &'a ActionContext<'a>,
        input: Self::Input,
    ) -> impl Future<Output = Result<Self::Output, Self::Error>> + Send + 'a;
}

// ── §2.2.2 StatefulAction (Q7 R1 amended: init_state + migrate_state restored) ──
//
// Pre-amendment (v2): only `execute` declared; lifecycle methods absent.
// Post-amendment: production parity per `crates/action/src/stateful.rs:56-66`
// — both lifecycle methods restored. State bound widened to
// `Serialize + DeserializeOwned + Clone + Send + Sync + 'static` for
// engine-side persisted-iteration round-trip + retry/redrive.

pub trait StatefulAction: Send + Sync + 'static {
    type Input: HasSchema + DeserializeOwned + Send + 'static;
    type Output: Serialize + Send + 'static;
    type State: Serialize + DeserializeOwned + Clone + Send + Sync + 'static;
    type Error: std::error::Error + Send + Sync + 'static;

    /// Q7 R1 amendment — engine-driven; called once per execution start.
    fn init_state(&self) -> Self::State;

    /// Q7 R1 amendment — engine consults only when checkpoint deserialize fails;
    /// `Some(migrated)` continues, `None` propagates the original error.
    fn migrate_state(&self, _old: serde_json::Value) -> Option<Self::State> {
        None
    }

    fn execute<'a>(
        &'a self,
        ctx: &'a ActionContext<'a>,
        state: &'a mut Self::State,
        input: Self::Input,
    ) -> impl Future<Output = Result<Self::Output, Self::Error>> + Send + 'a;
}

// ── §2.2.3 TriggerAction (Q6 + Q7 R3 + Q8 F2 amended) ──
//
// Pre-amendment (v2): only `type Source`/`type Error`/`handle()` declared,
// `handle()` returned `Result<(), Self::Error>` (fire-and-forget unit).
// Post-amendment (3 layers):
//
// - Q6: `start()` + `stop()` lifecycle restored from production
//   `crates/action/src/trigger.rs:61-72`.
// - Q7 R3: `handle()` returns `Result<TriggerEventOutcome, Self::Error>` —
//   per-event multiplicity (Skip / Emit / EmitMany) per production
//   `trigger.rs:215-264`. `accepts_events()` predicate added per
//   production `trigger.rs:359-361`.
// - Q8 F2: `idempotency_key(&self, event)` hook default-opt-in per
//   `q8-phase2-synthesis.md` §3 F2 architect-default position.

pub trait TriggerSource: Send + Sync + 'static {
    type Event: Send + 'static;
}

pub trait TriggerAction: Send + Sync + 'static {
    type Source: TriggerSource;
    type Error: std::error::Error + Send + Sync + 'static;

    /// Q6 amendment — register external listener / schedule / queue consumer.
    /// Two valid shapes per `TriggerHandler` contract: setup-and-return OR
    /// run-until-cancelled. Cancel-safe at every `.await` per §3.4.
    fn start<'a>(
        &'a self,
        ctx: &'a ActionContext<'a>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'a;

    /// Q6 amendment — pair of `start()`. Clears state set by start so a
    /// subsequent start is accepted.
    fn stop<'a>(
        &'a self,
        ctx: &'a ActionContext<'a>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'a;

    /// Q7 R3 amendment — engine-side gate for engine-pushed events.
    /// Default `false` per production `trigger.rs:359-361` (poll-shape
    /// keeps default; webhook-shape overrides to `true`).
    fn accepts_events(&self) -> bool {
        false
    }

    /// Q8 F2 amendment — per-event idempotency hook. Default `None`
    /// preserves single-worker behavior (no engine dedup intent).
    /// Cluster-mode coordination cascade (slot 2) consumes the key
    /// against per-trigger `dedup_window`. Authors override to declare
    /// idempotency intent — see §3.7 placeholder.
    fn idempotency_key<'a>(
        &'a self,
        _event: &'a <Self::Source as TriggerSource>::Event,
    ) -> Option<IdempotencyKey> {
        None
    }

    /// Q7 R3 amendment — returns per-event multiplicity outcome.
    fn handle<'a>(
        &'a self,
        ctx: &'a ActionContext<'a>,
        event: <Self::Source as TriggerSource>::Event,
    ) -> impl Future<Output = Result<TriggerEventOutcome, Self::Error>> + Send + 'a;
}

/// Q7 R3 amendment — outcome of processing an external event pushed to a
/// trigger. Per production `crates/action/src/trigger.rs:215-264`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum TriggerEventOutcome {
    /// Filtered out — no workflow execution.
    Skip,
    /// Single workflow execution with the supplied JSON input.
    Emit(serde_json::Value),
    /// Fan-out to N workflow executions (batched delivery, RSS feed, Kafka batch).
    EmitMany(Vec<serde_json::Value>),
}

/// Q8 F2 amendment — per-event idempotency key for cluster-mode dedup-window
/// engine logic. Authors derive from event content (NOT monotonic time) so
/// cross-worker dedup matches.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IdempotencyKey(String);

impl IdempotencyKey {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

// ── §2.2.4 ResourceAction (Q7 R2 amended: configure/cleanup paradigm) ──
//
// Pre-amendment (v2): declared `Input` / `Output` / `execute(&self, ctx,
// &resource, input)` — spike-specification artifact not grounded in production.
// Post-amendment: shape restored to production `crates/action/src/resource.rs:36-52`
// — `ResourceAction` is a graph-scoped DI primitive; engine runs `configure`
// before downstream nodes, lends `&Self::Resource` to the subtree (consumer
// actions read via `ctx.resource()`, NOT through `ResourceAction::execute`),
// calls `cleanup` when scope ends. Spurious `execute` / `Input` / `Output`
// dropped — preserves production ABI exactly.

pub trait Resource: Send + Sync + 'static {
    type Credential: Credential;
}

pub trait ResourceAction: Send + Sync + 'static {
    type Resource: Resource;
    type Error: std::error::Error + Send + Sync + 'static;

    /// Q7 R2 amendment — engine runs before downstream nodes; resulting
    /// `Self::Resource` is owned by engine for the scope's lifetime.
    fn configure<'a>(
        &'a self,
        ctx: &'a ActionContext<'a>,
    ) -> impl Future<Output = Result<Self::Resource, Self::Error>> + Send + 'a;

    /// Q7 R2 amendment — receives ownership of resource (consume); engine
    /// calls exactly once per `configure`. Double-cleanup is engine bug
    /// surfaced as `ActionError::Fatal` at adapter dyn boundary.
    fn cleanup<'a>(
        &'a self,
        resource: Self::Resource,
        ctx: &'a ActionContext<'a>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'a;

    // Note: NO `execute` method on `ResourceAction`. Production
    // `crates/action/src/resource.rs:36-52` declares only `configure` +
    // `cleanup`. The pre-amendment `execute` was wrong against production.
}

// ============================================================
// 6. *Handler companion traits — Q1 amendment: #[async_trait]
//    + Q7 R4/R5 dyn-boundary shape changes
// ============================================================
//
// Per Tech Spec §2.4 (Q1 amendment per §15.9): pre-amendment shape used
// hand-written `BoxFut<'a, T>` returns per method (mimicking what
// `#[async_trait]` emits internally). Post-amendment: §2.4 adopts
// `#[async_trait::async_trait]` per ADR-0024 §Decision items 1+4 (the ADR
// explicitly enumerates these four `*Handler` traits among the 14
// `dyn`-consumed traits approved for `#[async_trait]`).
//
// Q7 R4 (per §15.11): `ResourceHandler` adopts `Box<dyn Any + Send + Sync>`
// resource handoff per production `resource.rs:59-107`; spurious `execute`
// dropped.
//
// Q7 R5 (per §15.11): `TriggerHandler` adopts `TriggerEvent` envelope (NOT
// JSON) per production `trigger.rs:97-122` — type-erased `Box<dyn Any>`
// payload + `TypeId` diagnostic. Adapter-side downcast is the typification
// boundary — see Tech Spec §3.5 typification path narrative.

use async_trait::async_trait;

/// Q7 R5 amendment — type-erased event envelope at the dyn boundary
/// per production `crates/action/src/trigger.rs:97-122`.
pub struct TriggerEvent {
    pub id: Option<String>,
    pub received_at: SystemTime,
    pub payload: Box<dyn Any + Send + Sync>,
    pub payload_type: TypeId,
    pub payload_type_name: &'static str,
}

#[async_trait]
pub trait StatelessHandler: Send + Sync + 'static {
    fn metadata(&self) -> &ActionMetadata;
    async fn execute(
        &self,
        ctx: &ActionContext<'_>,
        input: serde_json::Value,
    ) -> Result<serde_json::Value, ActionError>;
}

#[async_trait]
pub trait StatefulHandler: Send + Sync + 'static {
    fn metadata(&self) -> &ActionMetadata;
    async fn execute(
        &self,
        ctx: &ActionContext<'_>,
        state: &mut serde_json::Value,
        input: serde_json::Value,
    ) -> Result<serde_json::Value, ActionError>;
}

#[async_trait]
pub trait TriggerHandler: Send + Sync + 'static {
    fn metadata(&self) -> &ActionMetadata;
    async fn start(&self, ctx: &ActionContext<'_>) -> Result<(), ActionError>;
    async fn stop(&self, ctx: &ActionContext<'_>) -> Result<(), ActionError>;

    /// Q7 R5 amendment — engine consults before dispatching events through
    /// `handle_event`. Default `false` — webhook adapter overrides to `true`.
    fn accepts_events(&self) -> bool {
        false
    }

    /// Q7 R5 amendment — default returns `ActionError::fatal` per production
    /// `trigger.rs:373-389`. Pull-driven triggers (poll) never have this
    /// called; only adapters that override `accepts_events()` to `true` see it.
    async fn handle_event(
        &self,
        ctx: &ActionContext<'_>,
        event: TriggerEvent,
    ) -> Result<TriggerEventOutcome, ActionError> {
        let _ = (ctx, event);
        Err(ActionError::fatal("trigger does not accept external events"))
    }
}

#[async_trait]
pub trait ResourceHandler: Send + Sync + 'static {
    fn metadata(&self) -> &ActionMetadata;

    /// Q7 R4 amendment — returns type-erased instance for engine-side handoff
    /// per production `crates/action/src/resource.rs:59-92`.
    async fn configure(
        &self,
        config: serde_json::Value,
        ctx: &ActionContext<'_>,
    ) -> Result<Box<dyn Any + Send + Sync>, ActionError>;

    /// Q7 R4 amendment — adapter downcasts box to typed Resource;
    /// downcast-mismatch is `ActionError::Fatal` per `resource.rs:195-200`.
    async fn cleanup(
        &self,
        resource: Box<dyn Any + Send + Sync>,
        ctx: &ActionContext<'_>,
    ) -> Result<(), ActionError>;
}

// ============================================================
// 7. ActionHandler enum — UNCHANGED from v2 (4 variants, no Control)
// ============================================================

#[non_exhaustive]
pub enum ActionHandler {
    Stateless(Arc<dyn StatelessHandler>),
    Stateful(Arc<dyn StatefulHandler>),
    Trigger(Arc<dyn TriggerHandler>),
    Resource(Arc<dyn ResourceHandler>),
}

// ============================================================
// 8. Sealed DX traits (Q7 R6 amended: Webhook/Poll bound to Action)
// ============================================================
//
// Pre-amendment (v2): WebhookAction / PollAction declared as **subtraits**
// of `TriggerAction`. Production reality (`crates/action/src/webhook.rs:578`
// + `crates/action/src/poll.rs:800`): both are
// `pub trait XAction: Action + Send + Sync + 'static` — **peers** of
// `TriggerAction`, not subtraits. They have their own associated types,
// own lifecycle methods, and erase to `dyn TriggerHandler` only at the dyn
// boundary via dedicated adapters (`WebhookTriggerAdapter`, `PollTriggerAdapter`).
//
// Q7 R6 (per §15.11): re-pinned to peer-of-Action shape — bound chain
// `+ Action` (NOT `+ TriggerAction`); each DX trait declares its own
// associated types verbatim from production.

pub trait ActionSlots {
    fn credential_slots(&self) -> &'static [SlotBinding];
}

pub trait Action: ActionSlots + Send + Sync + 'static {}

mod sealed_dx {
    pub trait ControlActionSealed {}
    pub trait PaginatedActionSealed {}
    pub trait BatchActionSealed {}
    pub trait WebhookActionSealed {}
    pub trait PollActionSealed {}
}

// ── DX traits that ARE subtraits of a primary (Stateless / Action) ──

/// Erases to `Stateless` via `ControlActionAdapter` — see Tech Spec §2.5
/// (no `Control` variant on `ActionHandler` enum).
pub trait ControlAction: sealed_dx::ControlActionSealed + StatelessAction { /* ... */ }

pub struct PageResult<O, C>(PhantomData<(O, C)>);

pub trait PaginatedAction: sealed_dx::PaginatedActionSealed + Action {
    type Input: HasSchema + Send + Sync;
    type Output: Send + Sync;
    type Cursor: Serialize + DeserializeOwned + Clone + Send + Sync;
    fn max_pages(&self) -> u32 { 100 }
    fn fetch_page<'a>(
        &'a self,
        input: &'a Self::Input,
        cursor: Option<&'a Self::Cursor>,
        ctx: &'a ActionContext<'a>,
    ) -> impl Future<Output = Result<PageResult<Self::Output, Self::Cursor>, ActionError>> + Send + 'a;
}

pub struct BatchItemResult<O>(PhantomData<O>);

pub trait BatchAction: sealed_dx::BatchActionSealed + Action {
    type Input: HasSchema + Send + Sync;
    type Item: Serialize + DeserializeOwned + Clone + Send + Sync;
    type Output: Serialize + DeserializeOwned + Clone + Send + Sync;
    fn batch_size(&self) -> usize { 50 }
    fn extract_items(&self, input: &Self::Input) -> Vec<Self::Item>;
    fn process_item<'a>(
        &'a self,
        item: Self::Item,
        ctx: &'a ActionContext<'a>,
    ) -> impl Future<Output = Result<Self::Output, ActionError>> + Send + 'a;
    fn merge_results(&self, results: Vec<BatchItemResult<Self::Output>>) -> Self::Output;
}

// ── DX traits that are PEERS of TriggerAction (Q7 R6 amendment) ──
//
// Production reality: webhook.rs:578 + poll.rs:800 declare these as
// `Action + Send + Sync + 'static`, NOT as `: TriggerAction`. Each carries
// its own lifecycle and associated types; erasure to `dyn TriggerHandler`
// happens via dedicated adapters.

pub struct WebhookRequest;
pub struct WebhookResponse;
pub struct WebhookConfig;
impl Default for WebhookConfig {
    fn default() -> Self {
        Self
    }
}

pub trait WebhookAction: sealed_dx::WebhookActionSealed + Action + Send + Sync + 'static {
    /// Q7 R6 — state held between activate/deactivate (e.g., webhook
    /// registration ID). No Serde/Default bounds — webhook state is ephemeral,
    /// not persisted across process restarts in v1 per `webhook.rs:579-585`.
    type State: Clone + Send + Sync;

    fn on_activate<'a>(
        &'a self,
        ctx: &'a ActionContext<'a>,
    ) -> impl Future<Output = Result<Self::State, ActionError>> + Send + 'a;

    fn handle_request<'a>(
        &'a self,
        request: &'a WebhookRequest,
        state: &'a Self::State,
        ctx: &'a ActionContext<'a>,
    ) -> impl Future<Output = Result<WebhookResponse, ActionError>> + Send + 'a;

    fn on_deactivate<'a>(
        &'a self,
        _state: Self::State,
        _ctx: &'a ActionContext<'a>,
    ) -> impl Future<Output = Result<(), ActionError>> + Send + 'a {
        async { Ok(()) }
    }

    fn config(&self) -> WebhookConfig {
        WebhookConfig::default()
    }
}

pub struct PollConfig;
pub struct PollCursor<C>(PhantomData<C>);
pub struct PollResult<E>(PhantomData<E>);

pub trait PollAction: sealed_dx::PollActionSealed + Action + Send + Sync + 'static {
    /// Default-bound so `initial_cursor` has default impl. Per `poll.rs:802-806`.
    type Cursor: Serialize + DeserializeOwned + Clone + Default + Send + Sync;
    type Event: Serialize + Send + Sync;

    fn poll_config(&self) -> PollConfig;

    fn validate<'a>(
        &'a self,
        _ctx: &'a ActionContext<'a>,
    ) -> impl Future<Output = Result<(), ActionError>> + Send + 'a {
        async { Ok(()) }
    }

    fn initial_cursor<'a>(
        &'a self,
        _ctx: &'a ActionContext<'a>,
    ) -> impl Future<Output = Result<Self::Cursor, ActionError>> + Send + 'a {
        async { Ok(Self::Cursor::default()) }
    }

    fn poll<'a>(
        &'a self,
        cursor: &'a mut PollCursor<Self::Cursor>,
        ctx: &'a ActionContext<'a>,
    ) -> impl Future<Output = Result<PollResult<Self::Event>, ActionError>> + Send + 'a;
}

// Crate-internal blanket impls — per-DX-trait, gating on the right primary.
impl<T: StatelessAction + ActionSlots + Send + Sync + 'static> sealed_dx::ControlActionSealed for T {}
impl<T: Action> sealed_dx::PaginatedActionSealed for T {}
impl<T: Action> sealed_dx::BatchActionSealed for T {}
impl<T: Action> sealed_dx::WebhookActionSealed for T {}
impl<T: Action> sealed_dx::PollActionSealed for T {}

// ============================================================
// 9. ActionMetadata::max_concurrent (Q8 F9 NEW per §3.6.1)
// ============================================================
//
// Per Tech Spec §3.6.1 (Q8 F9 amendment per §15.12). Engine-side dispatch
// throttle hint — engine respects this at action dispatch time per
// dispatch-pool sizing. Cluster-mode coordination cascade (slot 2) reads
// before dispatching `Arc<dyn StatelessHandler>` / `Arc<dyn StatefulHandler>`.
// Pre-cluster-mode engine builds may ignore the hint (no-op).

#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionMetadata {
    // ... existing fields per crates/action/src/metadata.rs:98-117 ...
    pub key: String,
    pub version: semver::Version,

    /// Q8 F9 amendment — per-action concurrency limit.
    /// `None` (default) — no per-action limit (engine-global throttle still applies).
    /// `Some(N)` — at most N concurrent in-flight executions of this action
    /// across the workflow's dispatch pool.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_concurrent: Option<NonZeroU32>,
}

#[derive(Debug, thiserror::Error)]
pub enum ActionError {
    #[error("fatal: {0}")]
    Fatal(String),
    #[error("validation: {0}")]
    Validation(String),
}

impl ActionError {
    pub fn fatal(msg: impl Into<String>) -> Self {
        Self::Fatal(msg.into())
    }
}

// ============================================================
// 10. NodeDefinition::action_version (Q8 F12 NEW per §3.6.2)
// ============================================================
//
// Per Tech Spec §3.6.2 (Q8 F12 amendment per §15.12). Surface contract only
// — Tech Spec records the obligation (NodeDefinition carries the field at
// workflow-save time); engine cascade defines exact dispatch enforcement
// (warn vs fail-closed default; opt-in auto-upgrade policy). Cross-reference:
// engine cascade scope per §1.2 N4.

pub struct ActionKey(pub String);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeDefinition {
    pub action_key: String, // ActionKey shape elided for spike

    /// Q8 F12 amendment — workflow-version save-time action pin. Records the
    /// action's `BaseMetadata::version` at workflow-save time, so engine
    /// dispatches against the version the workflow author saw. Without the
    /// pin, workflow saved at action vN and dispatched after action vN+1
    /// lands silently runs vN+1 — invisible version drift.
    pub action_version: semver::Version,

    // ... other engine-side NodeDefinition fields ...
}

// ============================================================
// 11. Engine cluster-mode trait placeholders (Q8 F13 NEW per §3.7; doc-only)
// ============================================================
//
// Per Tech Spec §3.7 (Q8 F13 amendment per §15.12). The four traits below
// are **declared shape, not implementation**. Engine cascade (slot 2 per
// `docs/tracking/cascade-queue.md`) lands the actual implementations.
// Action surface depends only on the shapes for hook-method return types
// (e.g., `IdempotencyKey` per §2.2.3 Q8 F2 amendment); pre-cluster-mode
// engine builds satisfy the contract via no-op default impls or
// compile-time conditional (engine cascade decides).
//
// Action authors do NOT impl these. They are the engine-side coordination
// surface that cluster-mode cascade implements. Tech Spec declares the shape
// so action-side hook return types align with engine consumption; engine
// cascade locks the exact body shape.

/// Persistence layer for poll-trigger cursors across worker restart and
/// cluster-mode rebalance. Per Tech Spec §1.2 N4 + §8.1.2 cursor in-memory
/// ownership narrative.
///
/// **Doc-only contract.** Engine cascade implements; methods elided.
pub trait CursorPersistence: Send + Sync + 'static {
    // engine-cascade scope — locks the exact persistence API
}

/// Multi-worker coordination for at-most-one-leader-at-a-time triggers.
/// Per Strategy §3.1 component 7 + §5.1.5 — TriggerAction's
/// `on_leader_acquire` / `on_leader_release` hooks consume this trait at
/// engine-coordination layer.
///
/// **Doc-only contract.** Engine cascade implements; methods elided.
pub trait LeaderElection: Send + Sync + 'static {
    // engine-cascade scope — locks the exact election API
}

/// Per-trigger external-subscription registry (webhook URL stability across
/// worker rebalance). Without this ledger, webhook re-registration on
/// worker restart races against external service de-dup; with it, the
/// engine restores the prior registration token from the ledger so the
/// external service sees a continuous registration. Per §2.2.3 webhook
/// trigger lifecycle (`start` / `stop` registration).
///
/// **Doc-only contract.** Engine cascade implements; methods elided.
pub trait ExternalSubscriptionLedger: Send + Sync + 'static {
    // engine-cascade scope — locks the exact ledger API
}

/// Missed-fire replay ledger for time-based triggers (CronSchedule /
/// IntervalSchedule / OneShotSchedule per cascade slot 3). Worker downtime
/// during a scheduled fire window leaves a backlog; the ledger lets the
/// engine replay missed fires per the trigger's missed-fire policy
/// (FireOnce / FireAll / Skip). Per ScheduleAction cascade slot 3
/// architect-recommended shape.
///
/// **Doc-only contract.** Engine cascade implements; methods elided.
pub trait ScheduleLedger: Send + Sync + 'static {
    // engine-cascade scope — locks the exact ledger API
}

// ============================================================
// 12. ActionSlots blanket marker — UNCHANGED from v2 §6
// ============================================================
//
// Action authors do NOT implement `ActionSlots` by hand. The `#[action]`
// macro emits this from the `credentials(slot: Type)` zone. A struct with
// a bare `CredentialRef<_>` field outside the zone has no `ActionSlots`
// impl, cannot satisfy the `Action` blanket marker, cannot be registered
// in the engine — Probe 3 (v2).
//
// Production proc-macro should ALSO emit a `compile_error!` when it sees
// a `CredentialRef<_>` field outside the `credentials(...)` zone — DX
// layer, complementing the type-system layer.

// Add blanket impls for each primary trait family (UNCHANGED from v2).
impl<T: StatelessAction + ActionSlots> Action for T {}
// (similar blanket impls for StatefulAction / ResourceAction / TriggerAction
// in production code; spike sources omit to avoid coherence collisions
// — production code uses sealed_dx adapters per §2.6 instead.)
