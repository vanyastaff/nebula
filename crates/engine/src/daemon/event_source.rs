//! EventSource topology + `EventSourceAdapter<E>: TriggerAction`.
//!
//! Event sources are resource providers adapted onto the engine's existing
//! `TriggerAction` substrate.
//!
//! # Why an adapter, not a TriggerAction extension
//!
//! `EventSource: Provider` and `TriggerAction: Action` sit on different bases.
//! `EventSourceAdapter<E>` bridges them as a typed action: the source owns its
//! [`ActionMetadataDraft`], while callers supply `Arc<E::Instance>` and an
//! `event_to_payload` closure. Erasure happens only through action's sealed
//! [`TriggerActionAdapter`].
//!
//! This mirrors `crates/action/src/poll.rs::PollTriggerAdapter` (which runs
//! `poll()` in an inline loop driven by `ctx.cancellation()` + `ctx.emitter()`).

use std::{
    future::Future,
    sync::{Arc, OnceLock},
};

use nebula_action::{
    Action, ActionError, ActionMetadataAdmissionError, ActionMetadataDraft, TriggerAction,
    TriggerActionAdapter, TriggerContext, TriggerEventOutcome, TriggerSource,
};
use nebula_core::Dependencies;
use nebula_resource::{ResourceContext, error::ErrorKind as ResourceErrorKind, resource::Provider};

/// EventSource — pull-based event subscription.
///
/// A long-lived event producer where consumers create subscriptions via
/// [`Self::subscribe`] and drain events via [`Self::recv`].
pub trait EventSource: Provider {
    /// The event type produced by this source.
    type Event: Send + Clone + 'static;
    /// An opaque subscription handle for receiving events.
    type Subscription: Send + 'static;

    /// Author-owned action catalog intent for this event source.
    fn action_metadata() -> ActionMetadataDraft;

    /// Action dependencies required while driving this event source.
    fn action_dependencies() -> &'static Dependencies {
        static DEPENDENCIES: OnceLock<Dependencies> = OnceLock::new();
        DEPENDENCIES.get_or_init(Dependencies::new)
    }

    /// Creates a new subscription to this event source.
    ///
    /// # Errors
    ///
    /// Returns [`nebula_resource::Error`] if the subscription cannot be
    /// created.
    fn subscribe(
        &self,
        runtime: &Self::Instance,
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

/// Runtime state for driving an [`EventSource`] through its explicit
/// subscribe/receive API outside the [`TriggerAction`] adapter path.
///
/// Most consumers should use [`EventSourceAdapter`] instead — it folds
/// EventSource into the engine's `TriggerAction` substrate. This struct stays
/// for the rare case where direct subscription management is needed
/// (e.g. testing, ad-hoc engine tooling).
pub struct EventSourceRuntime<E: EventSource> {
    _phantom: std::marker::PhantomData<E>,
}

impl<E: EventSource> EventSourceRuntime<E> {
    /// Creates a new event source runtime.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            _phantom: std::marker::PhantomData,
        }
    }
}

impl<E: EventSource> Default for EventSourceRuntime<E> {
    fn default() -> Self {
        Self::new()
    }
}

impl<E> EventSourceRuntime<E>
where
    E: EventSource + Send + Sync + 'static,
    E::Instance: Send + Sync + 'static,
{
    /// Creates a new subscription to the event source.
    ///
    /// # Errors
    ///
    /// Propagates errors from `EventSource::subscribe`.
    pub async fn subscribe(
        &self,
        resource: &E,
        runtime: &E::Instance,
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

/// Adapts an [`EventSource`] as a typed [`TriggerAction`].
///
/// # Construction
///
/// Callers supply:
/// - the typed `source: E`,
/// - an `Arc<E::Instance>` (caller is responsible for building `E::Instance` — typically via
///   `Resource::create()` outside the adapter),
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
    runtime: Arc<E::Instance>,
    // guard-justified: a single boxed-fn field — a type alias would not
    // improve readability over the inline signature.
    #[expect(
        clippy::type_complexity,
        reason = "single field — extracting to a type alias adds no readability"
    )]
    event_to_payload: Arc<dyn Fn(&E::Event) -> serde_json::Value + Send + Sync>,
}

impl<E> EventSourceAdapter<E>
where
    E: EventSource + Send + Sync + 'static,
    E::Instance: Send + Sync + 'static,
{
    /// Wrap an EventSource impl as a `TriggerAction`.
    pub fn new<F>(source: E, runtime: Arc<E::Instance>, event_to_payload: F) -> Self
    where
        F: Fn(&E::Event) -> serde_json::Value + Send + Sync + 'static,
    {
        Self {
            source,
            runtime,
            event_to_payload: Arc::new(event_to_payload),
        }
    }

    /// Admit this typed event source and erase it behind action's sealed trigger boundary.
    ///
    /// # Errors
    ///
    /// Returns a typed admission error when the source metadata or its associated schemas fail
    /// catalog admission.
    pub fn into_handler(self) -> Result<TriggerActionAdapter<Self>, ActionMetadataAdmissionError> {
        TriggerActionAdapter::new(self)
    }
}

impl<E> Action for EventSourceAdapter<E>
where
    E: EventSource + Send + Sync + 'static,
    E::Instance: Send + Sync + 'static,
{
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> ActionMetadataDraft {
        E::action_metadata()
    }

    fn dependencies() -> &'static Dependencies {
        E::action_dependencies()
    }
}

/// Event family marker for self-driven [`EventSourceAdapter`] triggers.
pub struct EventSourceTriggerSource;

impl TriggerSource for EventSourceTriggerSource {
    type Event = serde_json::Value;
}

impl<E> TriggerAction for EventSourceAdapter<E>
where
    E: EventSource + Send + Sync + 'static,
    E::Instance: Send + Sync + 'static,
{
    type Source = EventSourceTriggerSource;
    type Error = ActionError;

    async fn start(&self, ctx: &(impl TriggerContext + ?Sized)) -> Result<(), ActionError> {
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
                            // CANCEL SAFETY: start materialization is one transaction. A
                            // drop before commit writes nothing; after commit, execution,
                            // contract, revision references, and Start command are durable.
                            // EventSourceAdapter passes no event key, so this is an
                            // unconditional start.
                            match ctx.emitter().emit(payload, None).await {
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

    async fn stop(&self, ctx: &(impl TriggerContext + ?Sized)) -> Result<(), ActionError> {
        // Mirror PollTriggerAdapter::stop (poll.rs:1455) — cancel the trigger
        // context's cancellation token so the run-until-cancelled start() loop
        // observes the signal and returns Ok(()).
        ctx.cancellation().cancel();
        Ok(())
    }

    async fn handle(
        &self,
        _ctx: &(impl TriggerContext + ?Sized),
        _event: serde_json::Value,
    ) -> Result<TriggerEventOutcome, ActionError> {
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
        TriggerHandler,
        testing::{TestContextBuilder, TestTriggerContext},
    };
    use nebula_core::{Context, ResourceKey, action_key};
    use nebula_resource::{
        ResourceContext,
        error::Error as ResourceError,
        resource::{Provider, ResourceConfig, ResourceMetadataDraft},
    };

    use super::*;

    #[derive(Clone, Debug, Default, nebula_schema::Schema)]
    struct EmptyCfg;

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

    #[async_trait::async_trait]
    impl Provider for ThreeEventSource {
        type Config = EmptyCfg;
        type Instance = ();
        type Topology = nebula_resource::NoTopology;

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

        fn metadata() -> ResourceMetadataDraft {
            ResourceMetadataDraft::new(
                Self::key(),
                nebula_resource::metadata_name!("event-three"),
                "",
            )
        }
    }

    nebula_resource::no_credential_slots!(ThreeEventSource);

    impl EventSource for ThreeEventSource {
        type Event = u32;
        type Subscription = ();

        fn action_metadata() -> ActionMetadataDraft {
            make_metadata()
        }

        async fn subscribe(
            &self,
            _runtime: &Self::Instance,
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

    fn make_metadata() -> ActionMetadataDraft {
        ActionMetadataDraft::new(
            action_key!("test.event_source_adapter"),
            nebula_action::metadata_name!("EventSourceAdapterTest"),
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
            |e: &u32| serde_json::json!({ "n": *e }),
        )
        .into_handler()
        .expect("event source action metadata admits");

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
        let payloads = emitter.inputs();
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

    #[async_trait::async_trait]
    impl Provider for PermanentlyBrokenSource {
        type Config = EmptyCfg;
        type Instance = ();
        type Topology = nebula_resource::NoTopology;

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

        fn metadata() -> ResourceMetadataDraft {
            ResourceMetadataDraft::new(
                Self::key(),
                nebula_resource::metadata_name!("event-permanently-broken"),
                "",
            )
        }
    }

    nebula_resource::no_credential_slots!(PermanentlyBrokenSource);

    impl EventSource for PermanentlyBrokenSource {
        type Event = u32;
        type Subscription = ();

        fn action_metadata() -> ActionMetadataDraft {
            make_metadata()
        }

        async fn subscribe(
            &self,
            _runtime: &Self::Instance,
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
            |e: &u32| serde_json::json!({ "n": *e }),
        )
        .into_handler()
        .expect("event source action metadata admits");

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
            |e: &u32| serde_json::json!({ "n": *e }),
        )
        .into_handler()
        .expect("event source action metadata admits");

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
