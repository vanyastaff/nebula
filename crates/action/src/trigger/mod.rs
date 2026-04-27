//! Base trigger trait, handler contract, and transport-agnostic event envelope.
//!
//! A trigger action is a workflow starter — it lives outside the execution
//! graph and emits new workflow executions in response to external events
//! (webhooks, cron, polling, queue messages, etc.). The runtime drives the
//! `start`/`stop` lifecycle and receives workflow inputs through the trigger
//! context.
//!
//! ## Shape peers
//!
//! Specialized trigger families live in their own files and carry their
//! own transport-specific event types:
//!
//! - [`crate::webhook`] — HTTP webhooks, with `WebhookRequest` carrying method, path, query,
//!   headers, and body.
//! - [`crate::poll`] — pull-based periodic polling with cursor state.
//!
//! Both register their adapters against the dyn contract defined here
//! ([`TriggerHandler`]).
//!
//! ## Transport-agnostic envelope
//!
//! [`TriggerEvent`] is the type boundary between a transport and an
//! action **at the dyn layer**. It carries only what the runtime
//! needs — dedup id, arrival timestamp, and an opaque `Any`-erased
//! payload. The receiving handler downcasts the payload to its
//! transport-specific event type (for example,
//! [`crate::webhook::WebhookRequest`]).
//!
//! Transport-specific invariants (body size caps, header count limits,
//! HTTP methods, queue offsets, partition ids) stay inside the family
//! that owns them — they do NOT leak into the base trigger contract.

mod source;

use std::{
    any::{Any, TypeId},
    fmt,
    future::Future,
    pin::Pin,
    time::SystemTime,
};

use serde_json::Value;
pub use source::TriggerSource;

use crate::{context::TriggerContext, error::ActionError, metadata::ActionMetadata};

// ── Core trait ──────────────────────────────────────────────────────────────

/// Trigger action: workflow starter, lives outside the execution graph.
///
/// The runtime calls [`start`](Self::start) to begin listening (e.g.
/// webhook subscription, poll timer); [`stop`](Self::stop) to tear down.
/// Triggers emit new workflow executions; they do not run inside one.
///
/// Engine pushes external events via [`handle`](Self::handle); the
/// trigger returns a [`TriggerEventOutcome`] describing how many
/// workflow executions to start (skip / one / many).
///
/// ## Source associated type
///
/// `Source: TriggerSource` ties the trigger to its event family
/// (webhook / poll / queue / schedule). The typed event reaching
/// [`handle`](Self::handle) is `<Self::Source as TriggerSource>::Event`.
/// Per Tech Spec §2.2.3 spike Probe 2 — without `Source`, the impl
/// fails compile with E0046; this is intentional.
///
/// ## Idempotency
///
/// [`idempotency_key`](Self::idempotency_key) returns `None` by default;
/// triggers whose transport supplies a stable per-event id (webhook
/// delivery id, queue message id) override to return `Some(...)`.
/// Engine uses the key to suppress duplicate workflow executions per
/// PRODUCT_CANON §11.3 idempotency.
#[diagnostic::on_unimplemented(
    message = "`{Self}` does not implement TriggerAction",
    note = "implement `Source`, `Error`, and the `start`/`stop`/`handle` methods"
)]
pub trait TriggerAction: Send + Sync + 'static {
    /// Trigger event family — see [`TriggerSource`] (e.g.
    /// [`WebhookSource`](crate::WebhookSource), [`PollSource`](crate::PollSource)).
    type Source: TriggerSource;

    /// Error type returned by lifecycle methods.
    ///
    /// Most implementations use [`ActionError`](crate::ActionError) directly.
    /// Specialized triggers MAY use a richer typed error and let the
    /// adapter wrap it on the way to the dyn-layer.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Static metadata for this trigger.
    fn metadata(&self) -> &ActionMetadata;

    /// Start the trigger (register listener, schedule poll, etc.).
    ///
    /// Per [`TriggerHandler::start`](crate::TriggerHandler::start) for the
    /// two valid lifecycle shapes (setup-and-return vs run-until-cancelled)
    /// and cancel-safety contract.
    fn start(
        &self,
        ctx: &(impl TriggerContext + ?Sized),
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send;

    /// Stop the trigger (unregister, cancel schedule).
    fn stop(
        &self,
        ctx: &(impl TriggerContext + ?Sized),
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send;

    /// Whether this trigger accepts externally pushed events.
    /// Default: `false`. Override to return `true` from webhook / queue triggers.
    fn accepts_events(&self) -> bool {
        false
    }

    /// Stable transport-level dedup id for this event, if available.
    ///
    /// Default: `None`. Override when the transport supplies a stable
    /// per-event id (webhook delivery id, queue message id).
    /// Per Tech Spec §15.12 F2 + PRODUCT_CANON §11.3 idempotency.
    fn idempotency_key(
        &self,
        _event: &<Self::Source as TriggerSource>::Event,
    ) -> Option<crate::IdempotencyKey> {
        None
    }

    /// Handle an external event pushed to this trigger.
    ///
    /// Only called when [`accepts_events`](Self::accepts_events) returns
    /// `true`. The event is the typed `<Self::Source as TriggerSource>::Event`
    /// (e.g. `WebhookRequest` for `WebhookSource`).
    ///
    /// Returns a [`TriggerEventOutcome`] — `Skip` (filter out), `Emit(payload)`
    /// (start one workflow), or `EmitMany(payloads)` (fan-out).
    ///
    /// Default: returns [`Self::Error`] from the action's transport contract;
    /// triggers that do accept events MUST override.
    fn handle(
        &self,
        _ctx: &(impl TriggerContext + ?Sized),
        _event: <Self::Source as TriggerSource>::Event,
    ) -> impl std::future::Future<Output = Result<TriggerEventOutcome, Self::Error>> + Send {
        async {
            // Default body: triggers that don't accept events should never
            // have this called. Engine checks `accepts_events()` first; this
            // path is a defensive guard.
            //
            // We can't construct a Self::Error here without knowing its
            // shape — convention is that pushed-event triggers override
            // accepts_events()=true AND override handle(). Implementations
            // that opt into events but forget to override handle() will
            // hit `unimplemented!()`; this matches Tech Spec §2.2.3
            // expected-author-discipline contract.
            unimplemented!(
                "TriggerAction::handle: trigger reports accepts_events=true \
                 but did not override handle(); see Tech Spec §2.2.3"
            )
        }
    }
}

// ── Transport-agnostic event envelope ───────────────────────────────────────

/// Transport-agnostic event envelope at the [`TriggerHandler`] boundary.
///
/// Carries ONLY what the runtime needs: an optional dedup id, the arrival
/// timestamp at the transport, and an opaque payload. The concrete
/// transport-specific event (a [`crate::webhook::WebhookRequest`], a
/// future `SqsMessage`, etc.) lives inside `payload` as a type-erased
/// `Box<dyn Any + Send + Sync>` — the receiving handler downcasts it to
/// its own type via [`downcast`](Self::downcast).
///
/// # Why type erasure at the dyn boundary
///
/// Earlier versions of this type were an HTTP request in disguise
/// (`body: Vec<u8>`, `headers: HashMap<String, String>`, size/header
/// caps baked into the constructor). Every non-HTTP transport would
/// either have to grow this struct with optional fields or fake headers
/// it does not have. Erasing the payload keeps the base layer honest:
/// the runtime and the `TriggerHandler` trait do not encode the
/// semantics of any particular transport, while transport families
/// are free to define their own typed event with their own
/// invariants and constructors.
pub struct TriggerEvent {
    /// Stable dedup key assigned by the transport (webhook delivery id,
    /// SQS message id, Kafka offset). `None` if the transport cannot
    /// supply one — dedup then becomes the action's responsibility.
    id: Option<String>,

    /// Arrival time at the transport (not the action). Used by runtime
    /// metrics and by dedup / replay-window logic that needs to know
    /// when the event was observed, independent of processing latency.
    received_at: SystemTime,

    /// The transport-specific payload, erased at the dyn boundary.
    ///
    /// The trait object is `Any + Send + Sync + 'static`. Adapters
    /// downcast to their expected type; a mismatch is an engine-level
    /// routing bug and surfaces as [`ActionError::Fatal`] at the
    /// adapter.
    payload: Box<dyn Any + Send + Sync>,

    /// `TypeId` of the concrete payload, captured at construction so
    /// the adapter can report a meaningful error when it receives an
    /// event routed to the wrong handler family.
    payload_type: TypeId,
    /// Static type name of the payload for diagnostic messages only.
    payload_type_name: &'static str,
}

impl TriggerEvent {
    /// Build an envelope wrapping a transport-specific payload.
    ///
    /// `id` is the transport-supplied dedup key (pass `None` if the
    /// transport does not provide one). `received_at` is set to the
    /// current clock; use [`with_received_at`](Self::with_received_at)
    /// to override when replaying historical events.
    #[must_use]
    pub fn new<T>(id: Option<String>, payload: T) -> Self
    where
        T: Any + Send + Sync + 'static,
    {
        Self {
            id,
            received_at: SystemTime::now(),
            payload_type: TypeId::of::<T>(),
            payload_type_name: std::any::type_name::<T>(),
            payload: Box::new(payload),
        }
    }

    /// Override the arrival timestamp. Useful for replay from persisted
    /// events where the original arrival time should be preserved.
    #[must_use]
    pub fn with_received_at(mut self, at: SystemTime) -> Self {
        self.received_at = at;
        self
    }

    /// Transport-supplied dedup key, if any.
    #[must_use]
    pub fn id(&self) -> Option<&str> {
        self.id.as_deref()
    }

    /// Arrival timestamp at the transport.
    #[must_use]
    pub fn received_at(&self) -> SystemTime {
        self.received_at
    }

    /// Static type name of the erased payload — diagnostics only.
    #[must_use]
    pub fn payload_type_name(&self) -> &'static str {
        self.payload_type_name
    }

    /// Attempt to downcast the payload to the expected transport type.
    ///
    /// Returns the event itself on failure so the caller can surface a
    /// diagnostic pointing at [`payload_type_name`](Self::payload_type_name).
    /// Adapters that receive a mismatched event should raise
    /// [`ActionError::fatal`] — a mismatch is always an engine-level
    /// routing bug, not a user-recoverable condition.
    ///
    /// # Errors
    ///
    /// Returns `Err(self)` if the stored payload is not of type `T`.
    pub fn downcast<T>(self) -> Result<(Option<String>, SystemTime, T), Self>
    where
        T: Any + Send + Sync + 'static,
    {
        if self.payload_type != TypeId::of::<T>() {
            return Err(self);
        }
        match self.payload.downcast::<T>() {
            Ok(boxed) => Ok((self.id, self.received_at, *boxed)),
            Err(payload) => {
                // Should be unreachable because `payload_type` matched.
                Err(Self {
                    id: self.id,
                    received_at: self.received_at,
                    payload,
                    payload_type: self.payload_type,
                    payload_type_name: self.payload_type_name,
                })
            },
        }
    }
}

impl fmt::Debug for TriggerEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TriggerEvent")
            .field("id", &self.id)
            .field("received_at", &self.received_at)
            .field("payload_type", &self.payload_type_name)
            .finish_non_exhaustive()
    }
}

// ── Event outcome ───────────────────────────────────────────────────────────

/// Outcome of processing an external event pushed to a trigger.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum TriggerEventOutcome {
    /// Event filtered out — no workflow execution.
    Skip,
    /// Emit a single workflow execution with this input payload.
    Emit(Value),
    /// Emit multiple workflow executions (batch webhook, fan-out).
    EmitMany(Vec<Value>),
}

impl TriggerEventOutcome {
    /// Create a skip outcome.
    #[must_use]
    pub fn skip() -> Self {
        Self::Skip
    }

    /// Create a single-emit outcome.
    #[must_use]
    pub fn emit(payload: Value) -> Self {
        Self::Emit(payload)
    }

    /// Create a batch-emit outcome. Empty vec becomes `Skip`.
    #[must_use]
    pub fn emit_many(payloads: Vec<Value>) -> Self {
        if payloads.is_empty() {
            Self::Skip
        } else {
            Self::EmitMany(payloads)
        }
    }

    /// Whether this outcome will emit any executions.
    ///
    /// Returns `false` for `Skip` and for `EmitMany(vec![])` constructed
    /// directly (the `emit_many` constructor normalizes empty vecs to `Skip`).
    #[must_use]
    pub fn will_emit(&self) -> bool {
        match self {
            Self::Skip => false,
            Self::Emit(_) => true,
            Self::EmitMany(v) => !v.is_empty(),
        }
    }
}

// ── Handler trait ───────────────────────────────────────────────────────────

/// Trigger handler — start/stop lifecycle for workflow triggers.
///
/// Uses [`TriggerContext`] capability composition. Triggers live outside the
/// execution graph and emit new workflow executions.
///
/// # Errors
///
/// Returns [`ActionError`] if start or stop fails.
pub trait TriggerHandler: Send + Sync {
    /// Action metadata (key, version, capabilities).
    fn metadata(&self) -> &ActionMetadata;

    /// Start the trigger.
    ///
    /// # Semantics — two valid shapes
    ///
    /// Implementations fall into one of two categories:
    ///
    /// 1. **Setup-and-return** — register an external listener (webhook, message queue consumer),
    ///    then return immediately. The listener runs asynchronously outside this call. Example:
    ///    [`crate::webhook::WebhookTriggerAdapter`].
    ///
    /// 2. **Run-until-cancelled** — run the entire trigger loop inline, returning only when
    ///    `ctx.cancellation` fires or a fatal error occurs. Example:
    ///    [`crate::poll::PollTriggerAdapter`].
    ///
    /// **Callers MUST spawn `start()` in a dedicated task** and must not
    /// assume it returns promptly. Calling sites that drive multiple
    /// triggers sequentially will deadlock on shape (2).
    ///
    /// # Lifecycle
    ///
    /// `start` must be paired with [`stop`](Self::stop). Calling `start`
    /// twice without an intervening `stop` returns [`ActionError::Fatal`] —
    /// implementations MUST NOT silently re-register, as this would leak
    /// external resources (webhook registrations, poll loops, schedules).
    ///
    /// # Cancel safety
    ///
    /// Implementations MUST be cancel-safe — dropping the returned
    /// future at any `.await` point MUST NOT leak external resources,
    /// corrupt internal state, or lose in-flight work beyond what the
    /// trigger's contract already tolerates (e.g., a poll trigger may
    /// lose one cycle of cursor progress, but must not leak its
    /// listener registration).
    ///
    /// Shape-1 (setup-and-return) implementations register the
    /// listener, then return. Cancel safety is trivially satisfied
    /// because no `.await` sits on user state after registration.
    /// Shape-2 (run-until-cancelled) implementations must structure
    /// their `tokio::select!` so that each branch future is itself
    /// cancel-safe — see [`crate::poll::PollTriggerAdapter::start`]
    /// for a worked example using cancel-safe
    /// `CancellationToken::cancelled()` and `tokio::time::sleep`.
    ///
    /// # Errors
    ///
    /// Returns [`ActionError`] if the trigger cannot be started, if `start`
    /// is called twice without an intervening `stop`, or (for shape 2) if
    /// the trigger loop encounters a fatal error while running.
    fn start<'life0, 'life1, 'a>(
        &'life0 self,
        ctx: &'life1 dyn TriggerContext,
    ) -> Pin<Box<dyn Future<Output = Result<(), ActionError>> + Send + 'a>>
    where
        Self: 'a,
        'life0: 'a,
        'life1: 'a;

    /// Stop the trigger (unregister, cancel schedule).
    ///
    /// Clears any state set by [`start`](Self::start) so a subsequent
    /// `start` call is accepted. For shape-2 triggers (run-until-cancelled),
    /// prefer cancelling `ctx.cancellation` to let `start()` exit cleanly.
    ///
    /// # Errors
    ///
    /// Returns [`ActionError`] if the trigger cannot be stopped cleanly.
    fn stop<'life0, 'life1, 'a>(
        &'life0 self,
        ctx: &'life1 dyn TriggerContext,
    ) -> Pin<Box<dyn Future<Output = Result<(), ActionError>> + Send + 'a>>
    where
        Self: 'a,
        'life0: 'a,
        'life1: 'a;

    /// Whether this trigger accepts externally pushed events.
    ///
    /// Engine/webhook layer checks this before calling `handle_event`.
    /// Default: `false`.
    fn accepts_events(&self) -> bool {
        false
    }

    /// Handle an external event pushed to this trigger.
    ///
    /// Only called when [`accepts_events`](Self::accepts_events) returns `true`.
    /// Takes a transport-agnostic [`TriggerEvent`]; the adapter downcasts
    /// the erased payload to its family-specific type.
    ///
    /// # Errors
    ///
    /// Returns [`ActionError::Fatal`] by default — triggers that don't accept
    /// external events should never have this called.
    fn handle_event<'life0, 'life1, 'a>(
        &'life0 self,
        event: TriggerEvent,
        ctx: &'life1 dyn TriggerContext,
    ) -> Pin<Box<dyn Future<Output = Result<TriggerEventOutcome, ActionError>> + Send + 'a>>
    where
        Self: 'a,
        'life0: 'a,
        'life1: 'a,
    {
        let _ = (event, ctx);
        Box::pin(async {
            Err(ActionError::fatal(
                "trigger does not accept external events",
            ))
        })
    }
}

// ── TriggerActionAdapter ────────────────────────────────────────────────────

/// Wraps a [`TriggerAction`] as a [`dyn TriggerHandler`].
///
/// Simple delegation: `start` and `stop` call through to the typed trait
/// with [`TriggerContext`].
///
/// # Example
///
/// ```rust,ignore
/// let handler: Arc<dyn TriggerHandler> = Arc::new(TriggerActionAdapter::new(my_trigger));
/// ```
pub struct TriggerActionAdapter<A> {
    action: A,
}

impl<A> TriggerActionAdapter<A> {
    /// Wrap a typed trigger action.
    #[must_use]
    pub fn new(action: A) -> Self {
        Self { action }
    }

    /// Consume the adapter, returning the inner action.
    #[must_use]
    pub fn into_inner(self) -> A {
        self.action
    }
}

impl<A> TriggerHandler for TriggerActionAdapter<A>
where
    A: TriggerAction + Send + Sync + 'static,
{
    fn metadata(&self) -> &ActionMetadata {
        self.action.metadata()
    }

    /// Start the trigger by delegating to the typed action.
    ///
    /// # Errors
    ///
    /// Returns [`ActionError`] if the trigger cannot be started.
    fn start<'life0, 'life1, 'a>(
        &'life0 self,
        ctx: &'life1 dyn TriggerContext,
    ) -> Pin<Box<dyn Future<Output = Result<(), ActionError>> + Send + 'a>>
    where
        Self: 'a,
        'life0: 'a,
        'life1: 'a,
    {
        Box::pin(async move {
            self.action
                .start(ctx)
                .await
                .map_err(ActionError::fatal_from)
        })
    }

    /// Stop the trigger by delegating to the typed action.
    ///
    /// # Errors
    ///
    /// Returns [`ActionError`] if the trigger cannot be stopped cleanly.
    fn stop<'life0, 'life1, 'a>(
        &'life0 self,
        ctx: &'life1 dyn TriggerContext,
    ) -> Pin<Box<dyn Future<Output = Result<(), ActionError>> + Send + 'a>>
    where
        Self: 'a,
        'life0: 'a,
        'life1: 'a,
    {
        Box::pin(async move { self.action.stop(ctx).await.map_err(ActionError::fatal_from) })
    }
}

impl<A: TriggerAction> fmt::Debug for TriggerActionAdapter<A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TriggerActionAdapter")
            .field("action", &self.action.metadata().base.key)
            .finish_non_exhaustive()
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    use nebula_core::DeclaresDependencies;

    use super::*;
    use crate::{
        action::Action,
        testing::{TestContextBuilder, TestTriggerContext},
    };

    // ── TriggerActionAdapter tests ────────────────────────────────────────────

    struct MockTriggerAction {
        meta: ActionMetadata,
        started: AtomicBool,
    }

    impl MockTriggerAction {
        fn new() -> Self {
            Self {
                meta: ActionMetadata::new(
                    nebula_core::action_key!("test.trigger_action"),
                    "MockTrigger",
                    "Tracks start/stop",
                ),
                started: AtomicBool::new(false),
            }
        }
    }

    impl DeclaresDependencies for MockTriggerAction {}

    impl Action for MockTriggerAction {
        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }
    }

    impl TriggerAction for MockTriggerAction {
        async fn start(&self, _ctx: &(impl TriggerContext + ?Sized)) -> Result<(), ActionError> {
            self.started.store(true, Ordering::Release);
            Ok(())
        }

        async fn stop(&self, _ctx: &(impl TriggerContext + ?Sized)) -> Result<(), ActionError> {
            self.started.store(false, Ordering::Release);
            Ok(())
        }
    }

    fn make_trigger_ctx() -> TestTriggerContext {
        TestContextBuilder::new().build_trigger().0
    }

    #[test]
    fn trigger_adapter_is_dyn_compatible() {
        let adapter = TriggerActionAdapter::new(MockTriggerAction::new());
        let _: Arc<dyn TriggerHandler> = Arc::new(adapter);
    }

    #[tokio::test]
    async fn trigger_adapter_delegates_start_stop() {
        let action = MockTriggerAction::new();
        let adapter = TriggerActionAdapter::new(action);
        let ctx = make_trigger_ctx();

        adapter.start(&ctx).await.unwrap();
        assert!(adapter.action.started.load(Ordering::Acquire));

        adapter.stop(&ctx).await.unwrap();
        assert!(!adapter.action.started.load(Ordering::Acquire));
    }

    #[test]
    fn trigger_adapter_into_inner_returns_action() {
        let adapter = TriggerActionAdapter::new(MockTriggerAction::new());
        let action = adapter.into_inner();
        assert_eq!(
            action.metadata().base.key,
            nebula_core::action_key!("test.trigger_action")
        );
    }

    // ── TriggerEvent tests ────────────────────────────────────────────────────

    #[derive(Debug, PartialEq)]
    struct DummyPayload(&'static str);

    #[derive(Debug)]
    struct OtherPayload;

    #[test]
    fn trigger_event_downcast_success_yields_payload_id_and_timestamp() {
        let now = SystemTime::now();
        let event = TriggerEvent::new(Some("msg-1".to_string()), DummyPayload("hello"))
            .with_received_at(now);

        let (id, received_at, payload) = event.downcast::<DummyPayload>().unwrap();
        assert_eq!(id.as_deref(), Some("msg-1"));
        assert_eq!(received_at, now);
        assert_eq!(payload, DummyPayload("hello"));
    }

    #[test]
    fn trigger_event_downcast_mismatch_returns_event() {
        let event = TriggerEvent::new(None, OtherPayload);
        let type_name_before = event.payload_type_name();
        let back = event.downcast::<DummyPayload>().unwrap_err();
        // Error path preserves the envelope so the adapter can surface a
        // diagnostic pointing at the actual payload type.
        assert_eq!(back.payload_type_name(), type_name_before);
    }

    #[test]
    fn trigger_event_id_accessor_reflects_constructor_arg() {
        assert_eq!(
            TriggerEvent::new(Some("x".to_string()), DummyPayload("a")).id(),
            Some("x")
        );
        assert_eq!(TriggerEvent::new(None, DummyPayload("a")).id(), None);
    }

    #[test]
    fn trigger_event_debug_shows_payload_type_name() {
        let event = TriggerEvent::new(None, DummyPayload("x"));
        let debug = format!("{event:?}");
        assert!(
            debug.contains("DummyPayload"),
            "Debug impl should surface payload type: {debug}"
        );
    }

    // ── TriggerEventOutcome tests ─────────────────────────────────────────────

    #[test]
    fn trigger_event_outcome_skip() {
        let o = TriggerEventOutcome::skip();
        assert!(!o.will_emit());
        assert!(matches!(o, TriggerEventOutcome::Skip));
    }

    #[test]
    fn trigger_event_outcome_emit() {
        let o = TriggerEventOutcome::emit(serde_json::json!({"key": "val"}));
        assert!(o.will_emit());
        assert!(matches!(o, TriggerEventOutcome::Emit(_)));
    }

    #[test]
    fn trigger_event_outcome_emit_many() {
        let o = TriggerEventOutcome::emit_many(vec![serde_json::json!(1), serde_json::json!(2)]);
        assert!(o.will_emit());
        assert!(matches!(o, TriggerEventOutcome::EmitMany(v) if v.len() == 2));
    }

    #[test]
    fn trigger_event_outcome_empty_emit_many_is_skip() {
        let o = TriggerEventOutcome::emit_many(vec![]);
        assert!(!o.will_emit());
        assert!(matches!(o, TriggerEventOutcome::Skip));
    }

    #[test]
    fn trigger_event_outcome_direct_empty_emit_many_will_emit_false() {
        // Bypass the constructor — direct enum construction with empty vec.
        let o: TriggerEventOutcome = TriggerEventOutcome::EmitMany(vec![]);
        assert!(!o.will_emit());
    }
}
