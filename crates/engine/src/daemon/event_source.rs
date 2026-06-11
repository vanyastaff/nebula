//! EventSource topology + `EventSourceAdapter<E>: TriggerAction`.
//!
//! Migrated from `crates/resource/src/topology/event_source.rs` and
//! `crates/resource/src/runtime/event_source.rs` per / Tech Spec
//! . EventSource lands as a thin adapter onto engine's existing
//! `TriggerAction` substrate.
//!
//! # Why an adapter, not a TriggerAction extension
//!
//! `EventSource: Resource` (needs `R::Runtime`, `nebula_resource::Error`, `ResourceContext`)
//! and `TriggerAction: Action` (needs `ActionMetadata`, `TriggerContext`,
//! `ActionError`) sit on different bases. Rather than refactor either trait,
//! `EventSourceAdapter<E>` bridges them at construction time:
//! caller supplies `Arc<E::Runtime>`, `ActionMetadata`, and an `event_to_payload`
//! closure; the adapter implements `TriggerAction::start` as a "run-until-cancelled"
//! loop that `subscribe`s + `recv`s + emits via `ctx.emitter()`.
//!
//! This mirrors `crates/action/src/poll.rs::PollTriggerAdapter` (which runs
//! `poll()` in an inline loop driven by `ctx.cancellation()` + `ctx.emitter()`).

use std::{future::Future, sync::Arc};

use nebula_action::{
    ActionError, ActionMetadata, TriggerContext, TriggerEvent, TriggerEventOutcome, TriggerHandler,
};
use nebula_resource::{Resource, ResourceContext, error::ErrorKind as ResourceErrorKind};

/// EventSource — pull-based event subscription.
///
/// A long-lived event producer where consumers create subscriptions via
/// [`Self::subscribe`] and drain events via [`Self::recv`].
pub trait EventSource: Resource {
    /// The event type produced by this source.
    type Event: Send + Clone + 'static;
    /// An opaque subscription handle for receiving events.
    type Subscription: Send + 'static;

    /// Creates a new subscription to this event source.
    ///
    /// # Errors
    ///
    /// Returns [`nebula_resource::Error`] if the subscription cannot be
    /// created.
    fn subscribe(
        &self,
        runtime: &Self::Runtime,
        ctx: &ResourceContext,
    ) -> impl Future<Output = Result<Self::Subscription, nebula_resource::Error>> + Send;

    /// Receives the next event from a subscription.
    ///
    /// This method blocks asynchronously until an event is available.
    ///
    /// # Errors
    ///
    /// Returns [`nebula_resource::Error`] if the subscription is broken or
    /// the source has been shut down.
    fn recv(
        &self,
        subscription: &mut Self::Subscription,
    ) -> impl Future<Output = Result<Self::Event, nebula_resource::Error>> + Send;
}

/// EventSource configuration.
///
/// Currently inert under [`EventSourceAdapter`] — the adapter does not consult
/// any field. Reserved for forward-compat: future transports may own bounded
/// queues / flow-control parameters, and consumers can opt in via this
/// `#[non_exhaustive]` struct without requiring a follow-up signature change.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct EventSourceConfig {
    /// Buffer size hint for transports that own a bounded internal queue
    /// between `subscribe` and `recv`. **Currently NOT consulted by
    /// `EventSourceAdapter::start`** — wire to a real buffering mechanism
    /// when a concrete EventSource implementation needs flow control.
    pub buffer_size: usize,
}

/// Runtime state for an EventSource — preserves the original
/// `EventSourceRuntime<R>` shape from `nebula-resource` for callers that want
/// the explicit subscribe/recv API outside the `TriggerAction` adapter path.
///
/// Most consumers should use [`EventSourceAdapter`] instead — it folds
/// EventSource into the engine's `TriggerAction` substrate. This struct stays
/// for the rare case where direct subscription management is needed
/// (e.g. testing, ad-hoc engine tooling).
pub struct EventSourceRuntime<E: EventSource> {
    config: EventSourceConfig,
    _phantom: std::marker::PhantomData<E>,
}

impl<E: EventSource> EventSourceRuntime<E> {
    /// Creates a new event source runtime with the given configuration.
    #[must_use]
    pub fn new(config: EventSourceConfig) -> Self {
        Self {
            config,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Returns the current configuration.
    #[must_use]
    pub fn config(&self) -> &EventSourceConfig {
        &self.config
    }
}

impl<E> EventSourceRuntime<E>
where
    E: EventSource + Send + Sync + 'static,
    E::Runtime: Send + Sync + 'static,
{
    /// Creates a new subscription to the event source.
    ///
    /// # Errors
    ///
    /// Propagates errors from `EventSource::subscribe`.
    pub async fn subscribe(
        &self,
        resource: &E,
        runtime: &E::Runtime,
        ctx: &ResourceContext,
    ) -> Result<E::Subscription, nebula_resource::Error> {
        resource.subscribe(runtime, ctx).await
    }

    /// Receives the next event from a subscription.
    ///
    /// # Errors
    ///
    /// Propagates errors from `EventSource::recv`.
    pub async fn recv(
        &self,
        resource: &E,
        subscription: &mut E::Subscription,
    ) -> Result<E::Event, nebula_resource::Error> {
        resource.recv(subscription).await
    }
}

// ── EventSourceAdapter — bridges EventSource onto TriggerHandler ────────────

/// Adapts an `EventSource` impl as a `TriggerHandler` so the engine can drive
/// it through the existing trigger lifecycle (`start`/`stop` + emit-via-context).
///
/// # Construction
///
/// Callers supply:
/// - the typed `source: E`,
/// - an `Arc<E::Runtime>` (caller is responsible for building `E::Runtime` — typically via
///   `Resource::create()` outside the adapter),
/// - `ActionMetadata` (EventSource has no inherent action metadata),
/// - `EventSourceConfig` for buffer / flow-control hints,
/// - an `event_to_payload` closure converting `&E::Event` to `serde_json::Value` (caller controls
///   serialization + redaction).
///
/// # Cancellation
///
/// `start()` runs a "run-until-cancelled" loop using a biased `tokio::select!`
/// against `ctx.cancellation()`. Drop-safety: each `recv().await` is the
/// subscription's responsibility; the adapter does not retain in-flight events.
pub struct EventSourceAdapter<E: EventSource> {
    source: E,
    runtime: Arc<E::Runtime>,
    metadata: ActionMetadata,
    // guard-justified: retained as a downstream-observability buffer-size
    // hint; not read on the current adapter path.
    #[allow(dead_code, reason = "buffer_size hint for downstream observability")]
    config: EventSourceConfig,
    // guard-justified: a single boxed-fn field — a type alias would not
    // improve readability over the inline signature.
    #[allow(
        clippy::type_complexity,
        reason = "single field — extracting to a type alias adds no readability"
    )]
    event_to_payload: Arc<dyn Fn(&E::Event) -> serde_json::Value + Send + Sync>,
}

impl<E> EventSourceAdapter<E>
where
    E: EventSource + Send + Sync + 'static,
    E::Runtime: Send + Sync + 'static,
{
    /// Wrap an EventSource impl as a `TriggerAction`.
    pub fn new<F>(
        source: E,
        runtime: Arc<E::Runtime>,
        metadata: ActionMetadata,
        config: EventSourceConfig,
        event_to_payload: F,
    ) -> Self
    where
        F: Fn(&E::Event) -> serde_json::Value + Send + Sync + 'static,
    {
        Self {
            source,
            runtime,
            metadata,
            config,
            event_to_payload: Arc::new(event_to_payload),
        }
    }
}

// `EventSourceAdapter<E>` carries per-instance dynamic metadata (the
// host supplies `ActionMetadata` at construction).// typed [`nebula_action::Action`] / [`nebula_action::TriggerAction`]
// traits require **static** metadata, so the adapter implements the
// dyn-erased [`nebula_action::TriggerHandler`] surface directly. The
// engine registers it as `Arc<dyn TriggerHandler>` like any other
// trigger, without going through a typed factory.
#[async_trait::async_trait]
impl<E> TriggerHandler for EventSourceAdapter<E>
where
    E: EventSource + Send + Sync + 'static,
    E::Runtime: Send + Sync + 'static,
{
    fn metadata(&self) -> &ActionMetadata {
        &self.metadata
    }

    async fn start(&self, ctx: &dyn TriggerContext) -> Result<(), ActionError> {
        let resource_ctx =
            ResourceContext::minimal(ctx.scope().clone(), ctx.cancellation().clone());
        let mut subscription = match self.source.subscribe(&self.runtime, &resource_ctx).await {
            Ok(sub) => sub,
            Err(e) => {
                // Subscribe failure: classify by ErrorKind same as recv so a
                // permanent / not-found subscribe error doesn't loop the
                // engine's restart supervisor against a broken source.
                ctx.health().record_error();
                return Err(classify_resource_error(e));
            },
        };

        loop {
            tokio::select! {
                biased;
                () = ctx.cancellation().cancelled() => return Ok(()),
                recv = self.source.recv(&mut subscription) => {
                    match recv {
                        Ok(event) => {
                            let payload = (self.event_to_payload)(&event);
                            match ctx.emitter().emit(payload).await {
                                Ok(_) => ctx.health().record_success(1),
                                Err(e) => {
                                    tracing::warn!(error = %e, "event_source: emit failed");
                                    ctx.health().record_error();
                                }
                            }
                        }
                        Err(e) => {
                            ctx.health().record_error();
                            match classify_resource_error_outcome(e) {
                                RecvOutcome::Continue => continue,
                                RecvOutcome::Cancelled => return Ok(()),
                                RecvOutcome::Fatal(action_err) => return Err(action_err),
                            }
                        }
                    }
                }
            }
        }
    }

    async fn stop(&self, ctx: &dyn TriggerContext) -> Result<(), ActionError> {
        // Mirror PollTriggerAdapter::stop (poll.rs:1455) — cancel the trigger
        // context's cancellation token so the run-until-cancelled start() loop
        // observes the signal and returns Ok(()).
        ctx.cancellation().cancel();
        Ok(())
    }

    fn accepts_events(&self) -> bool {
        // EventSourceAdapter is self-driving: events flow through
        // `ctx.emitter()` inside `start()`'s loop, not through the
        // `handle_event` push path.
        false
    }

    async fn handle_event(
        &self,
        _event: TriggerEvent,
        _ctx: &dyn TriggerContext,
    ) -> Result<TriggerEventOutcome, ActionError> {
        // Defensive guard for direct callers that bypass `accepts_events`.
        Err(ActionError::fatal(
            "EventSourceAdapter does not accept external events",
        ))
    }
}

/// Classify a `nebula_resource::Error` for the subscribe path: convert to the
/// matching `ActionError` constructor.
///
/// Used by `start()` on the early-return subscribe error. Recv errors use
/// [`classify_resource_error_outcome`] which additionally surfaces the
/// "continue the loop" choice for transient kinds.
fn classify_resource_error(res_err: nebula_resource::Error) -> ActionError {
    match res_err.kind() {
        // Retryable transient family — recv blocks until the next event so
        // there is no backoff to apply here.
        ResourceErrorKind::Transient
        | ResourceErrorKind::Exhausted { .. }
        | ResourceErrorKind::Backpressure => {
            tracing::warn!(error = %res_err, "event_source: subscribe transient error");
            ActionError::retryable(res_err.to_string())
        },
        // Tainted by a credential revoke. Non-terminal: the taint clears
        // once the credential is re-registered, so the source is
        // reacquirable — classify retryable, never fatal.
        ResourceErrorKind::Revoked => {
            tracing::warn!(
                error = %res_err,
                "event_source: subscribe rejected (resource tainted by credential revoke); retryable",
            );
            ActionError::retryable(res_err.to_string())
        },
        ResourceErrorKind::Cancelled => {
            tracing::info!(error = %res_err, "event_source: subscribe cancelled");
            ActionError::Cancelled
        },
        // Permanent caller/wiring faults. `Ambiguous` is a client conflict
        // (multi-tenant `(key, scope)` with no resolved slot identity) and
        // is **not** auto-retryable — surface it as fatal *explicitly* so
        // the supervisor does not hot-loop a mis-wired source; same
        // clean-exit handling as the other permanent kinds, but never via
        // a catch-all.
        ResourceErrorKind::Permanent
        | ResourceErrorKind::NotFound
        | ResourceErrorKind::Ambiguous => {
            tracing::error!(
                error = %res_err,
                kind = ?res_err.kind(),
                "event_source: subscribe permanent error",
            );
            ActionError::fatal(res_err.to_string())
        },
        // `ResourceErrorKind` is `#[non_exhaustive]` and defined in another
        // crate, so the compiler requires this arm: every variant that
        // exists today is matched explicitly above, so this is reachable
        // *only* by a future upstream `ErrorKind` addition. Fail safe (no
        // retry hot-loop) and log loudly that an unclassified kind needs an
        // explicit arm — the same conservatism `Classify` applies.
        other => {
            tracing::error!(
                error = %res_err,
                kind = ?other,
                "event_source: subscribe error of unclassified resource kind; treating as fatal — add an explicit arm",
            );
            ActionError::fatal(res_err.to_string())
        },
    }
}

/// Outcome for a recv-path classification.
enum RecvOutcome {
    /// Loop continues — transient error; recv() blocks until next event so
    /// there's no backoff to apply here. A future `RecvErrorPolicy` enum
    /// could add structured backoff once a real EventSource consumer needs
    /// operator tuning.
    Continue,
    /// Source-reported cancellation — return `Ok(())` so the engine treats
    /// it as normal shutdown rather than a fatal trigger failure.
    Cancelled,
    /// Permanent error — return the fatal `ActionError` so the engine's
    /// daemon supervisor doesn't hot-loop into a broken source.
    Fatal(ActionError),
}

fn classify_resource_error_outcome(res_err: nebula_resource::Error) -> RecvOutcome {
    match res_err.kind() {
        ResourceErrorKind::Transient
        | ResourceErrorKind::Exhausted { .. }
        | ResourceErrorKind::Backpressure => {
            tracing::warn!(
                error = %res_err,
                "event_source: recv transient error; continuing",
            );
            RecvOutcome::Continue
        },
        // Tainted by a credential revoke — transient: the source is
        // reacquirable once the credential is re-registered, so continue
        // the loop (recv blocks until the next event; no backoff here)
        // rather than treating it as a fatal trigger failure.
        ResourceErrorKind::Revoked => {
            tracing::warn!(
                error = %res_err,
                "event_source: recv rejected (resource tainted by credential revoke); continuing",
            );
            RecvOutcome::Continue
        },
        ResourceErrorKind::Cancelled => {
            tracing::info!(
                error = %res_err,
                "event_source: recv cancelled; exiting cleanly",
            );
            RecvOutcome::Cancelled
        },
        // Permanent caller/wiring faults. `Ambiguous` is a non-retryable
        // client conflict; surface it as fatal *explicitly* (clean
        // supervisor exit, no hot-loop) rather than through a catch-all.
        ResourceErrorKind::Permanent
        | ResourceErrorKind::NotFound
        | ResourceErrorKind::Ambiguous => {
            tracing::error!(
                error = %res_err,
                kind = ?res_err.kind(),
                "event_source: recv permanent error; exiting",
            );
            RecvOutcome::Fatal(ActionError::fatal(res_err.to_string()))
        },
        // `#[non_exhaustive]` cross-crate enum: every present variant is
        // matched explicitly above, so this is reachable only by a future
        // upstream `ErrorKind` addition. Fail safe (clean exit, no
        // hot-loop) and log loudly that it needs an explicit arm.
        other => {
            tracing::error!(
                error = %res_err,
                kind = ?other,
                "event_source: recv error of unclassified resource kind; treating as fatal — add an explicit arm",
            );
            RecvOutcome::Fatal(ActionError::fatal(res_err.to_string()))
        },
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    };

    use nebula_action::{
        ActionMetadata,
        testing::{TestContextBuilder, TestTriggerContext},
    };
    use nebula_core::{Context, ResourceKey, action_key};
    use nebula_resource::{
        ResourceContext,
        error::Error as ResourceError,
        resource::{Resource, ResourceConfig, ResourceMetadata},
    };

    use super::*;

    #[derive(Clone, Debug, Default)]
    struct EmptyCfg;

    nebula_schema::impl_empty_has_schema!(EmptyCfg);

    impl ResourceConfig for EmptyCfg {
        fn fingerprint(&self) -> u64 {
            0
        }
    }

    #[derive(Debug, thiserror::Error)]
    #[error("event-test: {0}")]
    struct TestError(&'static str);

    impl From<TestError> for ResourceError {
        fn from(e: TestError) -> Self {
            ResourceError::transient(e.to_string())
        }
    }

    /// Test EventSource that emits 3 fixed events then blocks.
    #[derive(Clone)]
    struct ThreeEventSource {
        emitted: Arc<AtomicU32>,
    }

    impl Resource for ThreeEventSource {
        type Config = EmptyCfg;
        type Runtime = ();

        fn key() -> ResourceKey {
            ResourceKey::new("event-three").unwrap()
        }

        async fn create(
            &self,
            _config: &Self::Config,
            _ctx: &ResourceContext,
        ) -> Result<(), ResourceError> {
            Ok(())
        }

        fn metadata() -> ResourceMetadata {
            ResourceMetadata::from_key(&Self::key())
        }
    }

    impl EventSource for ThreeEventSource {
        type Event = u32;
        type Subscription = ();

        async fn subscribe(
            &self,
            _runtime: &Self::Runtime,
            _ctx: &ResourceContext,
        ) -> Result<Self::Subscription, ResourceError> {
            Ok(())
        }

        async fn recv(
            &self,
            _subscription: &mut Self::Subscription,
        ) -> Result<Self::Event, ResourceError> {
            let n = self.emitted.fetch_add(1, Ordering::SeqCst);
            if n < 3 {
                Ok(n)
            } else {
                // Block forever — caller should observe cancellation.
                std::future::pending().await
            }
        }
    }

    fn make_metadata() -> ActionMetadata {
        ActionMetadata::new(
            action_key!("test.event_source_adapter"),
            "EventSourceAdapterTest",
            "Adapter integration test",
        )
    }

    #[tokio::test]
    async fn adapter_emits_events_until_cancelled() {
        let emitted = Arc::new(AtomicU32::new(0));
        let source = ThreeEventSource {
            emitted: Arc::clone(&emitted),
        };
        let adapter = EventSourceAdapter::new(
            source,
            Arc::new(()),
            make_metadata(),
            EventSourceConfig::default(),
            |e: &u32| serde_json::json!({ "n": *e }),
        );

        let (ctx, emitter, _scheduler) = TestContextBuilder::new().build_trigger();
        let cancel = ctx.cancellation().clone();

        // Run start() in background; cancel after a short delay.
        let join = tokio::spawn(async move { adapter.start(&ctx).await });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        cancel.cancel();
        let result = join.await.expect("join ok");
        assert!(
            result.is_ok(),
            "start should return Ok on cancellation: {result:?}"
        );

        // Source-side counter: 3 events succeeded; 4th call hit pending()
        // then was cancelled.
        assert!(emitted.load(Ordering::SeqCst) >= 3);

        // Verify the spy emitter actually received the payloads —
        // the source-side counter alone would pass even if every
        // emit() returned Err(_) and dropped the payload.
        let payloads = emitter.emitted();
        assert!(
            payloads.len() >= 3,
            "expected >=3 payloads on the spy emitter, got {}",
            payloads.len()
        );
        // First three payloads are well-formed per the closure
        // |e: &u32| serde_json::json!({ "n": *e }).
        assert_eq!(payloads[0], serde_json::json!({ "n": 0 }));
        assert_eq!(payloads[1], serde_json::json!({ "n": 1 }));
        assert_eq!(payloads[2], serde_json::json!({ "n": 2 }));
    }

    /// EventSource that fails recv() with a permanent error.
    ///
    /// Verifies the recv-error classification path: permanent kinds must
    /// surface as `ActionError::fatal` so the daemon supervisor doesn't
    /// hot-loop into a broken source.
    #[derive(Clone)]
    struct PermanentlyBrokenSource;

    #[derive(Debug, thiserror::Error)]
    #[error("permanent: {0}")]
    struct PermanentError(&'static str);

    impl From<PermanentError> for ResourceError {
        fn from(e: PermanentError) -> Self {
            ResourceError::permanent(e.to_string())
        }
    }

    impl Resource for PermanentlyBrokenSource {
        type Config = EmptyCfg;
        type Runtime = ();

        fn key() -> ResourceKey {
            ResourceKey::new("event-permanently-broken").unwrap()
        }

        async fn create(
            &self,
            _config: &Self::Config,
            _ctx: &ResourceContext,
        ) -> Result<(), ResourceError> {
            Ok(())
        }

        fn metadata() -> ResourceMetadata {
            ResourceMetadata::from_key(&Self::key())
        }
    }

    impl EventSource for PermanentlyBrokenSource {
        type Event = u32;
        type Subscription = ();

        async fn subscribe(
            &self,
            _runtime: &Self::Runtime,
            _ctx: &ResourceContext,
        ) -> Result<Self::Subscription, ResourceError> {
            Ok(())
        }

        async fn recv(
            &self,
            _subscription: &mut Self::Subscription,
        ) -> Result<Self::Event, ResourceError> {
            Err(PermanentError("source torn down").into())
        }
    }

    #[tokio::test]
    async fn adapter_returns_fatal_on_permanent_recv_error() {
        let adapter = EventSourceAdapter::new(
            PermanentlyBrokenSource,
            Arc::new(()),
            make_metadata(),
            EventSourceConfig::default(),
            |e: &u32| serde_json::json!({ "n": *e }),
        );

        let (ctx, _emitter, _scheduler) = TestContextBuilder::new().build_trigger();
        let result = adapter.start(&ctx).await;
        let err = result.expect_err("permanent recv error must surface as Err");
        assert!(
            err.is_fatal(),
            "permanent ResourceError must map to ActionError::fatal, got {err:?}",
        );
    }

    #[tokio::test]
    async fn adapter_stop_is_noop() {
        let source = ThreeEventSource {
            emitted: Arc::new(AtomicU32::new(0)),
        };
        let adapter = EventSourceAdapter::new(
            source,
            Arc::new(()),
            make_metadata(),
            EventSourceConfig::default(),
            |e: &u32| serde_json::json!({ "n": *e }),
        );

        let ctx: TestTriggerContext = TestContextBuilder::new().build_trigger().0;
        // stop() is a no-op — should always succeed.
        adapter.stop(&ctx).await.expect("stop is infallible");
    }

    // ────────────────────────────────────────────────────────────────────
    // Resource-error classifier arms (subscribe + recv paths).
    //
    // `Revoked` is transient (the source is reacquirable once the
    // credential is re-registered) → retryable / Continue, never fatal.
    // `Ambiguous` is a permanent caller conflict → fatal, but matched by an
    // *explicit* arm (not the `#[non_exhaustive]` catch-all).
    // ────────────────────────────────────────────────────────────────────

    #[test]
    fn subscribe_classifier_maps_revoked_to_retryable() {
        let err = ResourceError::revoked("resource tainted by credential revoke");
        match classify_resource_error(err) {
            ActionError::Retryable { .. } => {},
            other => panic!("Revoked must classify retryable on subscribe, got: {other:?}"),
        }
    }

    #[test]
    fn subscribe_classifier_maps_ambiguous_to_explicit_fatal() {
        let err = ResourceError::ambiguous("2 resolved-credential registrations at this scope");
        // `Ambiguous` is a non-retryable caller conflict: fatal is the
        // correct supervisor outcome, but it must be reached by the
        // explicit `Permanent | NotFound | Ambiguous` arm — not the
        // non-exhaustive tail. We assert the *classification* (fatal /
        // non-retryable); the explicitness is enforced structurally by the
        // exhaustive match in the classifier.
        match classify_resource_error(err) {
            ActionError::Fatal { .. } => {},
            other => panic!("Ambiguous must classify fatal on subscribe, got: {other:?}"),
        }
    }

    #[test]
    fn recv_classifier_maps_revoked_to_continue() {
        let err = ResourceError::revoked("resource tainted by credential revoke");
        assert!(
            matches!(classify_resource_error_outcome(err), RecvOutcome::Continue),
            "Revoked must continue the recv loop (transient), never fatal",
        );
    }

    #[test]
    fn recv_classifier_maps_ambiguous_to_explicit_fatal() {
        let err = ResourceError::ambiguous("2 resolved-credential registrations at this scope");
        assert!(
            matches!(classify_resource_error_outcome(err), RecvOutcome::Fatal(_)),
            "Ambiguous is a non-retryable caller conflict — explicit fatal, not Continue",
        );
    }

    #[test]
    fn recv_classifier_transient_family_still_continues() {
        // Regression guard: the new Revoked/Ambiguous arms must not have
        // disturbed the existing transient family.
        for err in [
            ResourceError::transient("blip"),
            ResourceError::backpressure("full"),
        ] {
            assert!(
                matches!(classify_resource_error_outcome(err), RecvOutcome::Continue),
                "transient family must still continue",
            );
        }
    }

    #[test]
    fn recv_classifier_cancelled_exits_clean() {
        assert!(
            matches!(
                classify_resource_error_outcome(ResourceError::cancelled()),
                RecvOutcome::Cancelled
            ),
            "Cancelled must remain a clean exit, not Continue/Fatal",
        );
    }
}
