//! Credential slot rotation / revoke: the refresh + two-phase
//! (synchronous-taint then cancellation-safe drain+hook) revoke surface,
//! their shared post-resolution dispatch, and the type-erased `(key, scope)`
//! row resolution helpers the rotation entry points use.

use std::{sync::Arc, time::Instant};

use nebula_core::{ResourceKey, ScopeLevel};
use nebula_credential::SecretFreeMessage;

use super::Manager;
use crate::{
    error::Error,
    events::ResourceEvent,
    metrics::SlotDispatchMetricOutcome,
    runtime::acquire_loop::{
        RetiredCleanupObservation, SlotHookAdmission, SlotHookDeferral, SlotHookObservation,
        SlotHookSettlement, SlotHookWaitOutcome,
    },
};

#[derive(Debug, Clone, Copy)]
enum SlotHookDirection {
    Refresh,
    Revoke,
}

/// Result of the in-flight drain that precedes a slot hook.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use = "the preceding drain outcome carries revoke safety information"]
#[non_exhaustive]
pub enum SlotDrainOutcome {
    /// The operation has no drain phase (credential refresh).
    NotRequired,
    /// All in-flight leases drained before the revoke hook was submitted.
    Drained,
    /// The best-effort drain timed out before the revoke hook was submitted.
    /// The hook still ran or became queue-owned, so this is not permission to
    /// retry it.
    #[non_exhaustive]
    TimedOut {
        /// Leases still outstanding when the drain budget elapsed.
        outstanding_leases: u64,
    },
}

/// Accepted outcome of a credential-slot hook dispatch.
///
/// `Err` from a manager slot method means the work was rejected before queue
/// acceptance or an observed hook failed. [`Deferred`](Self::Deferred) means
/// an admitted hook remains queue-owned. [`Abandoned`](Self::Abandoned) is a
/// distinct terminal loss: ownership settled without a hook result and retry
/// is unsafe because execution may already have produced external effects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use = "slot dispatch outcome carries execution, observation, and drain state that must be handled"]
#[non_exhaustive]
pub enum SlotDispatchOutcome {
    /// The hook completed successfully and its result was observed.
    #[non_exhaustive]
    Completed {
        /// Outcome of the revoke drain, or [`SlotDrainOutcome::NotRequired`]
        /// for refresh.
        drain: SlotDrainOutcome,
    },
    /// The admitted hook exceeded its framework-owned execution budget.
    /// This terminal outcome is non-retryable because the hook may already
    /// have produced external effects before cancellation.
    #[non_exhaustive]
    TimedOut {
        /// Outcome of the revoke drain, or [`SlotDrainOutcome::NotRequired`]
        /// for refresh.
        drain: SlotDrainOutcome,
    },
    /// The queue owns an accepted hook whose completion was not observable
    /// by this caller. Queue execution is bounded and best-effort; callers
    /// must not retry this accepted work.
    #[non_exhaustive]
    Deferred {
        /// Outcome of the revoke drain, or [`SlotDrainOutcome::NotRequired`]
        /// for refresh.
        drain: SlotDrainOutcome,
        /// Why the admitted task could not be observed to a terminal result.
        reason: SlotDeferralReason,
    },
    /// The admitted queue task terminated without producing a hook result.
    /// This is a terminal, non-retryable loss rather than observer deferral.
    #[non_exhaustive]
    Abandoned {
        /// Outcome of the revoke drain, or [`SlotDrainOutcome::NotRequired`]
        /// for refresh.
        drain: SlotDrainOutcome,
    },
}

/// Why an admitted credential hook returned [`SlotDispatchOutcome::Deferred`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use = "deferral reason determines whether the observation budget elapsed"]
#[non_exhaustive]
pub enum SlotDeferralReason {
    /// Dispatch originated inside a cleanup queue where awaiting another
    /// queue receipt could deadlock.
    CleanupContext,
    /// The accepted hook had not started when the caller's post-admission
    /// observation budget elapsed.
    ObservationTimedOut,
}

impl From<SlotHookDeferral> for SlotDeferralReason {
    fn from(value: SlotHookDeferral) -> Self {
        match value {
            SlotHookDeferral::CleanupContext => Self::CleanupContext,
            SlotHookDeferral::ObservationTimedOut => Self::ObservationTimedOut,
        }
    }
}

/// A resource registry row whose credential slot has been **synchronously
/// tainted** by [`Manager::taint_slot`](Manager::taint_slot) /
/// [`Manager::taint_slot_for_identity`](Manager::taint_slot_for_identity) —
/// phase 1 of the
/// two-phase revoke (see the [`manager`](crate::manager) module docs for the
/// canonical invariant and why the taint is synchronous-before-the-tail).
///
/// Holding one is proof the taint already ran to completion: new acquires on
/// this row's credential are already rejected. It is consumed by
/// [`Manager::drain_and_revoke`](Manager::drain_and_revoke) to run the
/// cancellation-safe drain + revoke-hook tail.
///
/// Opaque by design: the only valid use is to pass it to
/// [`drain_and_revoke`](Manager::drain_and_revoke). It is **not** `Clone` —
/// one taint maps to exactly one drain/revoke tail.
#[must_use = "a TaintedSlot only completes the revoke when passed to Manager::drain_and_revoke"]
pub struct TaintedSlot {
    /// Structural key of the tainted resource registry row (span/event
    /// label only — no credential material).
    pub(super) key: ResourceKey,
    /// The credential slot on that row that was revoked.
    pub(super) slot: String,
    /// The resolved row whose taint flag was already set synchronously.
    pub(super) managed: Arc<dyn crate::registry::ManagedHandle>,
    /// When the synchronous taint was applied — the drain/revoke duration
    /// metric spans from here so it covers the whole revoke, not just the
    /// awaited tail.
    pub(super) tainted_at: Instant,
}

impl std::fmt::Debug for TaintedSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Deliberately omits `managed` (not `Debug`, and an internal
        // erased handle); only the credential-free routing labels.
        f.debug_struct("TaintedSlot")
            .field("key", &self.key)
            .field("slot", &self.slot)
            .finish_non_exhaustive()
    }
}

/// Outcome of the cancellation-safe revoke tail
/// ([`Manager::drain_and_revoke`]).
///
/// The tail has exactly one owner of the per-resource time budget (the
/// `drain_timeout` argument): the drain wait is bounded by it
/// (best-effort — a timed-out drain still proceeds to the hook), and the
/// revoke hook is *separately* bounded by it. There is **no** caller-side
/// `tokio::time::timeout` wrapping the whole tail; the hook states
/// are reported here rather than inferred from a dropped outer future. See
/// the [`manager`](crate::manager) module docs for why an outer timeout
/// wrapper would be unsafe (it could drop the future before the hook ran).
/// Four variants are terminal hook results; [`Deferred`](Self::Deferred) is
/// the sole non-terminal observer result:
///
/// - [`Done`](Self::Done) — the revoke hook completed `Ok`.
/// - [`HookFailed`](Self::HookFailed) — the hook returned `Err` (carried
///   verbatim).
/// - [`HookTimedOut`](Self::HookTimedOut) — the hook itself did not
///   complete within the budget. The row stays tainted (the taint ran in
///   the synchronous phase-1); only a *hung hook* is bounded, never the
///   taint, and never at the cost of skipping a hook after a slow drain.
/// - [`Deferred`](Self::Deferred) — the queue accepted the hook work, but
///   completion cannot be observed from the current cleanup context.
/// - [`Abandoned`](Self::Abandoned) — admitted ownership settled without a
///   hook result; retry is unsafe.
#[derive(Debug)]
#[must_use = "the revoke tail outcome must be recorded (it is not a silent success)"]
#[non_exhaustive]
pub enum RevokeTail {
    /// Drain + revoke hook completed; the hook returned `Ok`. (A
    /// best-effort drain timeout that still reached a successful hook is
    /// still `Done` — the drain timeout is non-fatal.)
    #[non_exhaustive]
    Done {
        /// Outcome of the best-effort drain that preceded the hook.
        drain: SlotDrainOutcome,
    },
    /// The revoke hook returned an error. The row stays tainted; the
    /// inner error is preserved for the caller's outcome accounting.
    #[non_exhaustive]
    HookFailed {
        /// Observed provider or topology hook error.
        error: Error,
        /// Outcome of the best-effort drain that preceded the hook.
        drain: SlotDrainOutcome,
    },
    /// The revoke hook did not complete within the per-resource budget
    /// (a wedged `on_credential_revoke`). The row stays tainted; this is
    /// the only thing the budget bounds.
    #[non_exhaustive]
    HookTimedOut {
        /// Outcome of the best-effort drain that preceded the hook.
        drain: SlotDrainOutcome,
    },
    /// The queue accepted the revoke hook, but it remains queue-owned and
    /// has not completed yet. This is neither hook success nor failure and
    /// must not be retried.
    #[non_exhaustive]
    Deferred {
        /// Outcome of the best-effort drain that preceded queue acceptance.
        drain: SlotDrainOutcome,
        /// Why the admitted hook was not observed to a terminal result.
        reason: SlotDeferralReason,
    },
    /// The admitted queue task terminated without producing a hook result.
    #[non_exhaustive]
    Abandoned {
        /// Outcome of the best-effort drain that preceded the hook.
        drain: SlotDrainOutcome,
    },
}

impl Manager {
    fn slot_hook_settlement(
        &self,
        key: ResourceKey,
        slot: String,
        direction: SlotHookDirection,
    ) -> (SlotHookSettlement, SlotHookAdmission) {
        let hook_metrics = self.metrics.clone();
        let hook_event_bus = Arc::clone(&self.event_bus);
        let cleanup_metrics = self.metrics.clone();
        let cleanup_event_bus = Arc::clone(&self.event_bus);
        let cleanup_key = key.clone();
        let cleanup_slot = slot.clone();
        let (settlement, admission) = SlotHookSettlement::new(Box::new(move |observation| {
            let metric_outcome = match &observation {
                SlotHookObservation::Completed => SlotDispatchMetricOutcome::Success,
                SlotHookObservation::Failed(_) => SlotDispatchMetricOutcome::Failed,
                SlotHookObservation::TimedOut(_) => SlotDispatchMetricOutcome::TimedOut,
                SlotHookObservation::Abandoned => SlotDispatchMetricOutcome::Abandoned,
            };
            if let Some(metrics) = &hook_metrics {
                match direction {
                    SlotHookDirection::Refresh => {
                        metrics.record_slot_refresh_outcome(metric_outcome);
                    },
                    SlotHookDirection::Revoke => {
                        metrics.record_slot_revoke_outcome(metric_outcome);
                    },
                }
            }

            let event = match (direction, observation) {
                (SlotHookDirection::Refresh, SlotHookObservation::Completed) => {
                    Some(ResourceEvent::SlotRefreshed { key, slot })
                },
                (SlotHookDirection::Revoke, SlotHookObservation::Completed) => {
                    Some(ResourceEvent::SlotRevoked { key, slot })
                },
                (SlotHookDirection::Refresh, SlotHookObservation::Failed(kind)) => {
                    Some(ResourceEvent::SlotRefreshFailed {
                        key,
                        slot,
                        kind,
                        message: SecretFreeMessage::new("credential refresh hook failed"),
                    })
                },
                (SlotHookDirection::Refresh, SlotHookObservation::TimedOut(kind)) => {
                    Some(ResourceEvent::SlotRefreshFailed {
                        key,
                        slot,
                        kind,
                        message: SecretFreeMessage::new("credential refresh hook timed out"),
                    })
                },
                (SlotHookDirection::Revoke, SlotHookObservation::Failed(kind)) => {
                    Some(ResourceEvent::SlotRevokeFailed {
                        key,
                        slot,
                        kind,
                        message: SecretFreeMessage::new("credential revoke hook failed"),
                    })
                },
                (SlotHookDirection::Revoke, SlotHookObservation::TimedOut(kind)) => {
                    Some(ResourceEvent::SlotRevokeFailed {
                        key,
                        slot,
                        kind,
                        message: SecretFreeMessage::new("credential revoke hook timed out"),
                    })
                },
                (SlotHookDirection::Refresh, SlotHookObservation::Abandoned) => {
                    tracing::warn!(
                        resource.key = %key,
                        slot,
                        ?direction,
                        "admitted credential hook was abandoned before terminal settlement"
                    );
                    Some(ResourceEvent::SlotRefreshFailed {
                        key,
                        slot,
                        kind: crate::ErrorKind::Cancelled,
                        message: SecretFreeMessage::new(
                            "admitted credential hook was abandoned before producing a result",
                        ),
                    })
                },
                (SlotHookDirection::Revoke, SlotHookObservation::Abandoned) => {
                    tracing::warn!(
                        resource.key = %key,
                        slot,
                        ?direction,
                        "admitted credential hook was abandoned before terminal settlement"
                    );
                    Some(ResourceEvent::SlotRevokeFailed {
                        key,
                        slot,
                        kind: crate::ErrorKind::Cancelled,
                        message: SecretFreeMessage::new(
                            "admitted credential hook was abandoned before producing a result",
                        ),
                    })
                },
            };
            if let Some(event) = event {
                let _ = hook_event_bus.emit(event);
            }
        }));
        let settlement = settlement.with_cleanup_observer(Box::new(move |cleanup| {
            if let Some(metrics) = &cleanup_metrics {
                metrics.record_release_error();
            }
            let kind = match cleanup {
                RetiredCleanupObservation::Failed(kind) => {
                    tracing::warn!(
                        resource.key = %cleanup_key,
                        slot = cleanup_slot,
                        ?direction,
                        error.kind = ?kind,
                        "framework-owned retired cleanup failed after credential hook settlement"
                    );
                    kind
                },
                RetiredCleanupObservation::TimedOut => {
                    tracing::warn!(
                        resource.key = %cleanup_key,
                        slot = cleanup_slot,
                        ?direction,
                        "framework-owned retired cleanup timed out after credential hook settlement"
                    );
                    crate::ErrorKind::Backpressure
                },
                RetiredCleanupObservation::Abandoned => {
                    tracing::warn!(
                        resource.key = %cleanup_key,
                        slot = cleanup_slot,
                        ?direction,
                        "framework-owned retired cleanup was abandoned"
                    );
                    crate::ErrorKind::Cancelled
                },
            };
            let _ = cleanup_event_bus.emit(ResourceEvent::RetiredCleanupFailed {
                key: cleanup_key,
                slot: cleanup_slot,
                kind,
            });
        }));
        (settlement, admission)
    }

    /// Notifies a registered resource that one of its `#[credential]`
    /// slots was rotated, after the engine has installed the fresh guard.
    ///
    /// Resolves `(key, scope)` to the live [`ManagedResource`](crate::ManagedResource) via the same
    /// registry lookup the `acquire_*` family uses, then borrows the live
    /// `Instance` per topology and invokes
    /// [`Provider::on_credential_refresh`](crate::resource::Provider::on_credential_refresh)
    /// for `slot`. The slot cell itself
    /// lives on the author's resource struct and is populated/rotated by
    /// the engine through `&self` (`SlotCell::store`) — this method does
    /// **not** own a slot map; it only drives the per-resource hook.
    ///
    /// Emits [`ResourceEvent::SlotRefreshed`] only after terminal completion or
    /// [`ResourceEvent::SlotRefreshFailed`] (with a typed error kind and fixed
    /// credential-free message) on failure, and records the corresponding
    /// slot-refresh metric. [`SlotDispatchOutcome::Deferred`] means the hook
    /// was accepted by the queue but cannot be observed to completion here;
    /// it must not be retried and this call emits neither a false success nor
    /// a false failure event.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no resource is registered for
    ///   `key` at `scope`.
    /// - [`ErrorKind::Cancelled`](crate::error::ErrorKind::Cancelled) if the manager is shutting
    ///   down.
    /// - Whatever the resource's `on_credential_refresh` hook maps into [`Error`].
    ///
    /// # Cancel safety
    ///
    /// Admission happens synchronously on the first poll. Before admission,
    /// dropping the future has no effect. After admission, the release queue
    /// owns the hook and its settlement: dropping this observer cannot cancel
    /// the hook, and the queue-owned settlement still records exactly one
    /// terminal metric and emits the matching terminal event.
    #[tracing::instrument(
        level = "debug",
        name = "nebula.resource.slot_refresh",
        skip(self),
        fields(key = %key, slot = %slot, topology, duration_ms)
    )]
    pub async fn refresh_slot(
        &self,
        key: &ResourceKey,
        scope: ScopeLevel,
        slot: &str,
    ) -> Result<SlotDispatchOutcome, Error> {
        let managed = self.lookup_any_for_slot(key, &scope)?;
        self.refresh_resolved(
            key,
            slot,
            managed,
            crate::hook_guard::MAX_ROTATION_DISPATCH_CEILING,
            crate::hook_guard::MAX_ROTATION_DISPATCH_CEILING,
        )
        .await
    }

    /// [`refresh_slot`](Self::refresh_slot) pinned to the **collision-free
    /// structural** resolved per-slot credential identity.
    ///
    /// Resolves the registry row whose `slot_identity` matches (via the same
    /// unambiguous-by-construction path [`get_for`](crate::registry::Registry::get_for)
    /// backs), so a multi-tenant `(key, scope)` routes the rotation to the
    /// *specific* resolved row instead of failing closed with
    /// [`ErrorKind::Ambiguous`](crate::error::ErrorKind::Ambiguous). This is
    /// the entry point the engine per-slot rotation fan-out drives once it
    /// has resolved a node's slot bindings; identity-agnostic
    /// [`refresh_slot`](Self::refresh_slot) stays fail-closed for the
    /// no-identity caller. The engine rotation fan-out records the
    /// structural [`SlotIdentity`](crate::dedup::SlotIdentity) at bind time,
    /// so routing is by exact string equality (no digest aliasing).
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no row of `key` at `scope`
    ///   matches `slot_identity`.
    /// - [`ErrorKind::Cancelled`](crate::error::ErrorKind::Cancelled) if the manager is shutting
    ///   down.
    /// - Whatever the resource's `on_credential_refresh` hook maps into [`Error`].
    ///
    /// # Cancel safety
    ///
    /// Cancel safe — same contract as [`refresh_slot`](Self::refresh_slot).
    #[tracing::instrument(
        level = "debug",
        name = "nebula.resource.slot_refresh",
        skip(self, slot_identity),
        fields(key = %key, slot = %slot, topology, duration_ms)
    )]
    pub async fn refresh_slot_for_identity(
        &self,
        key: &ResourceKey,
        scope: ScopeLevel,
        slot: &str,
        slot_identity: &crate::dedup::SlotIdentity,
    ) -> Result<SlotDispatchOutcome, Error> {
        self.refresh_slot_for_identity_with_timeout(
            key,
            scope,
            slot,
            slot_identity,
            crate::hook_guard::MAX_ROTATION_DISPATCH_CEILING,
            crate::hook_guard::MAX_ROTATION_DISPATCH_CEILING,
        )
        .await
    }

    pub(crate) async fn refresh_slot_for_identity_with_timeout(
        &self,
        key: &ResourceKey,
        scope: ScopeLevel,
        slot: &str,
        slot_identity: &crate::dedup::SlotIdentity,
        hook_timeout: std::time::Duration,
        observation_timeout: std::time::Duration,
    ) -> Result<SlotDispatchOutcome, Error> {
        let managed = self.lookup_any_for_slot_identity_structural(key, &scope, slot_identity)?;
        self.refresh_resolved(key, slot, managed, hook_timeout, observation_timeout)
            .await
    }

    /// Post-resolution refresh dispatch shared by
    /// [`refresh_slot`](Self::refresh_slot) (identity-agnostic) and
    /// [`refresh_slot_for_identity`](Self::refresh_slot_for_identity)
    /// (slot-identity-pinned).
    ///
    /// The two public entry points differ only in how they resolve the row;
    /// the hook dispatch, metric (exactly one outcome per dispatch), and
    /// event emission are identical and live here.
    async fn refresh_resolved(
        &self,
        key: &ResourceKey,
        slot: &str,
        managed: Arc<dyn crate::registry::ManagedHandle>,
        hook_timeout: std::time::Duration,
        observation_timeout: std::time::Duration,
    ) -> Result<SlotDispatchOutcome, Error> {
        let started = Instant::now();
        tracing::Span::current().record("topology", managed.topology_tag().as_str());

        // Unknown-slot validation: reject a slot name the resource type does
        // not declare, before the author's `on_credential_refresh` hook ever
        // dispatches. See `ManagedHandle::accepts_credential_slot_name` — a
        // type that declares no credential slots at all rejects every slot
        // name (fail closed: nothing to rotate). Still recorded as a real
        // `Failed` dispatch outcome and `SlotRefreshFailed` event — a
        // rejected call is observably a failed refresh to any subscriber
        // counting attempts, not a silent no-op that would undercount
        // `attempts` relative to `success + failed + timed_out`.
        if !managed.accepts_credential_slot_name(slot) {
            let err = Error::unknown_credential_slot(key.clone(), slot);
            if let Some(m) = &self.metrics {
                m.record_slot_refresh_outcome(SlotDispatchMetricOutcome::Failed);
            }
            self.emit(ResourceEvent::SlotRefreshFailed {
                key: key.clone(),
                slot: slot.to_owned(),
                kind: err.kind().clone(),
                message: SecretFreeMessage::new(
                    "credential refresh rejected: unknown credential slot",
                ),
            });
            tracing::warn!(error = %err, "slot refresh rejected: unknown credential slot");
            return Err(err);
        }

        // Submission is the ownership boundary. A rejection below happens
        // before admission and is returned as a retryable error. Once accepted,
        // the queue task owns the hook and terminal settlement. The caller only
        // observes its receipt for `observation_timeout` while the hook is
        // queued. Expiry before execution starts returns `Deferred` without
        // cancelling or double-accounting the admitted task. Once execution
        // starts, the caller awaits the hook's independently bounded terminal
        // result.
        let (settlement, admission) =
            self.slot_hook_settlement(key.clone(), slot.to_owned(), SlotHookDirection::Refresh);
        let accepted =
            match Arc::clone(&managed).submit_on_refresh(slot, hook_timeout, settlement, admission)
            {
                Ok(accepted) => accepted,
                Err(error) => {
                    if let Some(metrics) = &self.metrics {
                        metrics.record_slot_refresh_outcome(SlotDispatchMetricOutcome::Failed);
                    }
                    self.emit(ResourceEvent::SlotRefreshFailed {
                        key: key.clone(),
                        slot: slot.to_owned(),
                        kind: error.kind().clone(),
                        message: SecretFreeMessage::new("credential refresh hook admission failed"),
                    });
                    return Err(error);
                },
            };
        // Start the observer budget only after synchronous admission. The
        // provider execution budget starts when the queue dequeues the hook;
        // stamping this deadline before admission would make an equal hook
        // timeout unreachable under queue delay. Receipt polling is biased,
        // so a terminal hook result wins an exact deadline tie.
        let observation_deadline = tokio::time::Instant::now() + observation_timeout;
        let observed = accepted.wait_until(observation_deadline).await;
        tracing::Span::current().record("duration_ms", started.elapsed().as_millis() as u64);
        match observed {
            SlotHookWaitOutcome::Completed => Ok(SlotDispatchOutcome::Completed {
                drain: SlotDrainOutcome::NotRequired,
            }),
            SlotHookWaitOutcome::Deferred(reason) => {
                if let Some(metrics) = &self.metrics {
                    metrics.record_slot_refresh_deferred();
                }
                tracing::debug!(
                    outcome = "deferred",
                    "slot refresh hook is admitted and queue-owned; caller stops observing"
                );
                Ok(SlotDispatchOutcome::Deferred {
                    drain: SlotDrainOutcome::NotRequired,
                    reason: reason.into(),
                })
            },
            SlotHookWaitOutcome::Abandoned => Ok(SlotDispatchOutcome::Abandoned {
                drain: SlotDrainOutcome::NotRequired,
            }),
            SlotHookWaitOutcome::TimedOut(_error) => Ok(SlotDispatchOutcome::TimedOut {
                drain: SlotDrainOutcome::NotRequired,
            }),
            SlotHookWaitOutcome::Failed(error) => Err(error),
        }
    }

    /// **Phase 1 of the revoke port — synchronous, runs to completion before
    /// any `.await`.** Resolves the registry row pinned to the
    /// **collision-free structural** resolved per-slot credential identity
    /// and *taints it immediately* so the `acquire_*` funnel rejects new
    /// leases on the revoked credential, then returns a [`TaintedSlot`]
    /// handle the caller passes to
    /// [`drain_and_revoke`](Self::drain_and_revoke) for the cancellation-safe
    /// drain + hook tail.
    ///
    /// Why this is split off as a non-`async` function: the engine fan-out
    /// wraps the awaited tail in `tokio::time::timeout`. A Rust `async fn`
    /// body is *lazy* — if a `timeout` future is dropped before the runtime
    /// first polls it, the body never runs. Were the taint the first
    /// statement of an `async` body, a timeout that fired before the first
    /// poll would drop the future and **skip the taint entirely**, leaving
    /// new acquires accepted on a credential whose revoke "timed out". This
    /// function is plain `fn`: the taint is applied eagerly at the call site,
    /// fully completed before this returns, and therefore *outside* and
    /// *before* any per-resource timeout (per-resource revoke deferral).
    ///
    /// Identity routing: resolves the *exact* resolved registry row by
    /// structural string equality (no digest aliasing) via the
    /// unambiguous-by-construction
    /// [`get_for`](crate::registry::Registry::get_for) path, so a
    /// multi-tenant `(key, scope)` taints the *specific* resolved row
    /// instead of failing closed with
    /// [`ErrorKind::Ambiguous`](crate::error::ErrorKind::Ambiguous). This is
    /// the entry point the engine per-slot rotation fan-out drives on a
    /// lease revoke; identity-agnostic [`taint_slot`](Self::taint_slot) stays
    /// fail-closed for the no-identity caller. Synchronous-before-`.await`
    /// taint guarantee; see the [`manager`](crate::manager) module docs for
    /// the canonical invariant.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no row of `key` at `scope`
    ///   matches `slot_identity`.
    /// - [`ErrorKind::Cancelled`](crate::error::ErrorKind::Cancelled) if the manager is shutting
    ///   down.
    ///
    /// Carries only `key` / `slot` / `topology` (no credential material)
    /// onto the span.
    #[tracing::instrument(
        level = "debug",
        name = "nebula.resource.slot_taint",
        skip(self, slot_identity),
        fields(key = %key, slot = %slot, topology, op = "revoke")
    )]
    pub fn taint_slot_for_identity(
        &self,
        key: &ResourceKey,
        scope: ScopeLevel,
        slot: &str,
        slot_identity: &crate::dedup::SlotIdentity,
    ) -> Result<TaintedSlot, Error> {
        let managed = self.lookup_any_for_slot_identity_structural(key, &scope, slot_identity)?;
        self.taint_now(key, slot, managed)
    }

    /// [`taint_slot_for_identity`](Self::taint_slot_for_identity) for the
    /// slot-identity-agnostic caller (the convenience
    /// [`revoke_slot`](Self::revoke_slot) path and non-fan-out
    /// callers/tests).
    ///
    /// Same eager, pre-`await` taint guarantee as
    /// [`taint_slot_for_identity`](Self::taint_slot_for_identity); only row
    /// resolution differs (identity-agnostic, so a multi-tenant
    /// `(key, scope)` fails closed with
    /// [`ErrorKind::Ambiguous`](crate::error::ErrorKind::Ambiguous) rather
    /// than tainting an arbitrary tenant's row).
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no resource is registered for
    ///   `key` at `scope`.
    /// - [`ErrorKind::Ambiguous`](crate::error::ErrorKind::Ambiguous) if more than one
    ///   resolved-credential row exists for `(key, scope)`.
    /// - [`ErrorKind::Cancelled`](crate::error::ErrorKind::Cancelled) if the manager is shutting
    ///   down.
    #[tracing::instrument(
        level = "debug",
        name = "nebula.resource.slot_taint",
        skip(self),
        fields(key = %key, slot = %slot, topology, op = "revoke")
    )]
    pub fn taint_slot(
        &self,
        key: &ResourceKey,
        scope: ScopeLevel,
        slot: &str,
    ) -> Result<TaintedSlot, Error> {
        let managed = self.lookup_any_for_slot(key, &scope)?;
        self.taint_now(key, slot, managed)
    }

    /// Validates the slot name, then applies the taint synchronously and
    /// packages the [`TaintedSlot`] handle. Shared tail of
    /// [`taint_slot`](Self::taint_slot) /
    /// [`taint_slot_for_identity`](Self::taint_slot_for_identity); the
    /// safety-critical
    /// invariant — *taint is fully applied before this returns* — is written
    /// once here. This is **phase 1** of the two-phase revoke; see the
    /// [`manager`](crate::manager) module docs for the canonical invariant
    /// (why both stores are synchronous-before-`.await`, the TOCTOU close,
    /// and the revoke-epoch fence).
    ///
    /// # Errors
    ///
    /// [`Error::unknown_credential_slot`] if `slot` does not match a
    /// credential slot the resolved row's resource type declares (a
    /// no-slot type rejects every slot name — fail closed). Checked
    /// *before* the taint / epoch bump, so a rejected call leaves the row
    /// untouched — `revoke_slot`'s taint stays all-or-nothing. Recorded as a
    /// `Failed` revoke-dispatch outcome and `SlotRevokeFailed` event — this
    /// is phase 1 of the revoke port, so a rejection here is a failed
    /// revoke attempt to any subscriber, not a silent no-op that would
    /// undercount `attempts` relative to `success + failed + timed_out`.
    fn taint_now(
        &self,
        key: &ResourceKey,
        slot: &str,
        managed: Arc<dyn crate::registry::ManagedHandle>,
    ) -> Result<TaintedSlot, Error> {
        tracing::Span::current().record("topology", managed.topology_tag().as_str());
        // Unknown-slot validation, checked before any mutation: a rejected
        // slot name must not taint or bump the epoch — the row stays
        // exactly as it was.
        if !managed.accepts_credential_slot_name(slot) {
            let err = Error::unknown_credential_slot(key.clone(), slot);
            if let Some(m) = &self.metrics {
                m.record_slot_revoke_outcome(SlotDispatchMetricOutcome::Failed);
            }
            self.emit(ResourceEvent::SlotRevokeFailed {
                key: key.clone(),
                slot: slot.to_owned(),
                kind: err.kind().clone(),
                message: SecretFreeMessage::new(
                    "credential revoke rejected: unknown credential slot",
                ),
            });
            tracing::warn!(error = %err, "slot taint rejected: unknown credential slot");
            return Err(err);
        }
        // Phase-1 taint, synchronously before any caller `.await`: this
        // function is not `async`, so the store has already happened by the
        // time control returns and a subsequently-dropped drain-tail timeout
        // future cannot un-apply it.
        managed.taint();
        // Phase-1 revoke-epoch bump, in the *same* synchronous pre-`.await`
        // step as the taint, so the pooled return-to-idle paths fence any
        // instance authenticated with the now-revoked credential before the
        // hook walks the idle queue.
        managed.bump_revoke_epoch();
        Ok(TaintedSlot {
            key: key.clone(),
            slot: slot.to_owned(),
            managed,
            tainted_at: Instant::now(),
        })
    }

    /// Default per-resource revoke budget for the back-to-back convenience
    /// callers ([`revoke_slot`](Self::revoke_slot)
    /// / [`revoke_slot_for_identity`](Self::revoke_slot_for_identity)).
    ///
    /// 30 s — the same budget the manager-wide `graceful_shutdown` drain
    /// uses and the value [`drain_and_revoke`](Self::drain_and_revoke)
    /// previously hard-coded for the drain wait. The engine rotation
    /// fan-out does **not** use this: it passes its own per-resource
    /// rotation budget so the timeout has one owner end-to-end.
    pub const DEFAULT_REVOKE_DRAIN_TIMEOUT: std::time::Duration =
        std::time::Duration::from_secs(30);

    /// **Phase 2 of the revoke port — the cancellation-safe awaited tail.**
    /// Consumes a [`TaintedSlot`] from [`taint_slot`](Self::taint_slot) /
    /// [`taint_slot_for_identity`](Self::taint_slot_for_identity) (whose
    /// taint already ran
    /// synchronously) and performs the remaining steps:
    ///
    /// 1. **Drain** only *this resource's* in-flight handles via its own per-resource counter
    ///    (per-resource revoke deferral) — never the manager-wide `drain_tracker`, so a revoke is isolated
    ///    from in-flight traffic to unrelated resources.
    /// 2. **Dispatch** [`Provider::on_credential_revoke`](crate::resource::Provider::on_credential_revoke) against the live runtime per topology.
    /// 3. Queue-owned settlement emits [`ResourceEvent::SlotRevoked`] /
    ///    `SlotRevokeFailed` and records exactly one terminal metric.
    ///
    /// **Single budget owner.** The
    /// `drain_timeout` argument is the caller's per-resource budget and is
    /// the *only* timeout governing this tail. It bounds **two** waits
    /// independently:
    ///
    /// - the per-resource **drain** — *best-effort*: a drain timeout is
    ///   non-fatal, it records the `TimedOut` outcome metric and the tail
    ///   **still proceeds to the revoke hook** (the taint already stops
    ///   *new* leases; the hook makes the resource stop emitting on the
    ///   old credential);
    /// - the **revoke hook** itself — a *wedged* `on_credential_revoke`
    ///   is the only thing the budget actually cuts short
    ///   ([`RevokeTail::HookTimedOut`]).
    ///
    /// The caller must not add another timeout around this future. The method
    /// owns both deadlines: the drain is bounded first, then synchronous hook
    /// admission transfers ownership to the queue. Observation expiry returns
    /// [`RevokeTail::Deferred`] only while the admitted hook has not started.
    /// Once execution starts, this method awaits the independently bounded
    /// terminal hook result.
    ///
    /// # Cancel safety
    ///
    /// The taint runs synchronously before this future. If cancellation lands
    /// during the drain, the row remains tainted and no hook was admitted. If
    /// it lands after hook admission, the queue owns the hook and settlement;
    /// terminal accounting and the matching event still happen exactly once.
    #[tracing::instrument(
        level = "debug",
        name = "nebula.resource.slot_drain_revoke",
        skip(self, tainted),
        fields(
            key = %tainted.key,
            slot = %tainted.slot,
            topology = tainted.managed.topology_tag().as_str(),
            duration_ms,
            op = "revoke",
        )
    )]
    pub async fn drain_and_revoke(
        &self,
        tainted: TaintedSlot,
        drain_timeout: std::time::Duration,
    ) -> RevokeTail {
        let TaintedSlot {
            key,
            slot,
            managed,
            tainted_at,
        } = tainted;

        // 1. Drain **only this resource's** in-flight handles (resource runtime status
        //    §Deferred): a revoke on resource A must not block on in-flight
        //    traffic to an unrelated resource B, so this awaits the row's
        //    own per-resource counter — not the manager-wide `drain_tracker`
        //    (which stays the `graceful_shutdown` primitive). Bounded by the
        //    caller's per-resource budget so a stuck handle on *this*
        //    resource cannot wedge revoke; the taint (already applied
        //    synchronously in the phase-1 function) already stops new
        //    leases.
        //
        //    A drain timeout is retained as an orthogonal detail on every
        //    hook result. It never replaces success/failure/timeout/deferral
        //    and never authorizes retry of accepted work.
        let drain_result = managed.wait_for_in_flight_drain(drain_timeout).await;
        if let Err(outstanding) = &drain_result {
            tracing::warn!(
                outstanding = *outstanding,
                "slot revoke: per-resource drain timed out; proceeding to \
                 revoke hook (resource already tainted, no new leases)"
            );
        }
        let drain = match drain_result {
            Ok(()) => SlotDrainOutcome::Drained,
            Err(outstanding_leases) => SlotDrainOutcome::TimedOut { outstanding_leases },
        };

        // 2. Submit the revoke hook with the same per-resource hook budget.
        //    Submission itself does not await: success transfers hook and
        //    settlement ownership to the queue. The absolute observation
        //    deadline below starts after admission and returns `Deferred` only
        //    if work is still queued. Once started, the independently bounded
        //    terminal result wins. Neither state cancels accepted work or turns
        //    it into a retryable timeout.
        let (settlement, admission) =
            self.slot_hook_settlement(key.clone(), slot.clone(), SlotHookDirection::Revoke);
        let accepted = match Arc::clone(&managed).submit_on_revoke(
            &slot,
            drain_timeout,
            settlement,
            admission,
        ) {
            Ok(accepted) => accepted,
            Err(error) => {
                if let Some(metrics) = &self.metrics {
                    metrics.record_slot_revoke_outcome(SlotDispatchMetricOutcome::Failed);
                }
                self.emit(ResourceEvent::SlotRevokeFailed {
                    key,
                    slot,
                    kind: error.kind().clone(),
                    message: SecretFreeMessage::new("credential revoke hook admission failed"),
                });
                return RevokeTail::HookFailed { error, drain };
            },
        };
        let deadline = tokio::time::Instant::now() + drain_timeout;
        let hook_outcome = accepted.wait_until(deadline).await;
        tracing::Span::current().record("duration_ms", tainted_at.elapsed().as_millis() as u64);

        match hook_outcome {
            SlotHookWaitOutcome::Completed => {
                tracing::debug!("slot revoke hook completed");
                RevokeTail::Done { drain }
            },
            SlotHookWaitOutcome::Deferred(reason) => {
                if let Some(metrics) = &self.metrics {
                    metrics.record_slot_revoke_deferred();
                }
                tracing::debug!(
                    outcome = "deferred",
                    "slot revoke hook is admitted and queue-owned; caller stops observing"
                );
                RevokeTail::Deferred {
                    drain,
                    reason: reason.into(),
                }
            },
            SlotHookWaitOutcome::Abandoned => RevokeTail::Abandoned { drain },
            SlotHookWaitOutcome::Failed(error) => {
                tracing::warn!(error = %error, "slot revoke hook failed");
                RevokeTail::HookFailed { error, drain }
            },
            SlotHookWaitOutcome::TimedOut(_error) => {
                tracing::warn!(
                    timeout_ms = drain_timeout.as_millis() as u64,
                    "slot revoke hook timed out (row stays tainted, no new leases)"
                );
                RevokeTail::HookTimedOut { drain }
            },
        }
    }

    /// Notifies a registered resource that one of its `#[credential]` slots
    /// was revoked — **thin two-phase convenience** for non-fan-out callers
    /// and tests.
    ///
    /// Equivalent to [`taint_slot`](Self::taint_slot) immediately followed by
    /// [`drain_and_revoke`](Self::drain_and_revoke). The engine per-slot
    /// rotation fan-out deliberately does **not** call this: it must run the
    /// synchronous taint phase *outside* its `tokio::time::timeout` and wrap
    /// only the awaited drain/hook tail, so a dropped timeout future can
    /// never skip the taint (per-resource revoke deferral). This convenience is for the
    /// no-timeout caller where the two phases run back-to-back on the same
    /// task.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no resource is registered for
    ///   `key` at `scope`.
    /// - [`ErrorKind::Ambiguous`](crate::error::ErrorKind::Ambiguous) if more than one
    ///   resolved-credential row exists for `(key, scope)`.
    /// - [`ErrorKind::Cancelled`](crate::error::ErrorKind::Cancelled) if the manager is shutting
    ///   down.
    /// - Whatever the resource's `on_credential_revoke` hook maps into [`Error`].
    ///
    /// # Cancel safety
    ///
    /// This method is cancel safe. The taint and revoke-epoch bump run
    /// synchronously before the first await, so dropping the future never
    /// un-revokes the row — same contract as
    /// [`drain_and_revoke`](Self::drain_and_revoke).
    pub async fn revoke_slot(
        &self,
        key: &ResourceKey,
        scope: ScopeLevel,
        slot: &str,
    ) -> Result<SlotDispatchOutcome, Error> {
        let tainted = self.taint_slot(key, scope, slot)?;
        match self
            .drain_and_revoke(tainted, Self::DEFAULT_REVOKE_DRAIN_TIMEOUT)
            .await
        {
            RevokeTail::Done { drain } => Ok(SlotDispatchOutcome::Completed { drain }),
            RevokeTail::HookFailed { error, .. } => Err(error),
            RevokeTail::HookTimedOut { drain } => Ok(SlotDispatchOutcome::TimedOut { drain }),
            RevokeTail::Deferred { drain, reason } => {
                Ok(SlotDispatchOutcome::Deferred { drain, reason })
            },
            RevokeTail::Abandoned { drain } => Ok(SlotDispatchOutcome::Abandoned { drain }),
        }
    }

    /// [`revoke_slot`](Self::revoke_slot) pinned to the **collision-free
    /// structural** resolved per-slot credential identity — the
    /// slot-identity-aware two-phase convenience.
    ///
    /// Equivalent to
    /// [`taint_slot_for_identity`](Self::taint_slot_for_identity) immediately
    /// followed by [`drain_and_revoke`](Self::drain_and_revoke); a
    /// multi-tenant `(key, scope)` taints/drains/revokes the *specific*
    /// resolved row instead of failing closed with
    /// [`ErrorKind::Ambiguous`](crate::error::ErrorKind::Ambiguous). Like
    /// Like [`revoke_slot`](Self::revoke_slot), this is the back-to-back
    /// convenience path; the engine fan-out drives the two phases separately
    /// ([`taint_slot_for_identity`](Self::taint_slot_for_identity) outside
    /// the timeout, then [`drain_and_revoke`](Self::drain_and_revoke)) per
    /// per-resource revoke deferral.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no row of `key` at `scope`
    ///   matches `slot_identity`.
    /// - [`ErrorKind::Cancelled`](crate::error::ErrorKind::Cancelled) if the manager is shutting
    ///   down.
    /// - Whatever the resource's `on_credential_revoke` hook maps into [`Error`].
    pub async fn revoke_slot_for_identity(
        &self,
        key: &ResourceKey,
        scope: ScopeLevel,
        slot: &str,
        slot_identity: &crate::dedup::SlotIdentity,
    ) -> Result<SlotDispatchOutcome, Error> {
        let tainted = self.taint_slot_for_identity(key, scope, slot, slot_identity)?;
        match self
            .drain_and_revoke(tainted, Self::DEFAULT_REVOKE_DRAIN_TIMEOUT)
            .await
        {
            RevokeTail::Done { drain } => Ok(SlotDispatchOutcome::Completed { drain }),
            RevokeTail::HookFailed { error, .. } => Err(error),
            RevokeTail::HookTimedOut { drain } => Ok(SlotDispatchOutcome::TimedOut { drain }),
            RevokeTail::Deferred { drain, reason } => {
                Ok(SlotDispatchOutcome::Deferred { drain, reason })
            },
            RevokeTail::Abandoned { drain } => Ok(SlotDispatchOutcome::Abandoned { drain }),
        }
    }

    /// Type-erased `(key, scope)` → live `ManagedResource` resolution for
    /// the slot-rotation entry points.
    ///
    /// `refresh_slot` / `revoke_slot` take a `ResourceKey` (not a generic
    /// `R`), so they cannot use the typed `lookup::<R>`. This mirrors its
    /// shutdown-race guard (reject once `shutting_down` is observed) and
    /// resolves through the same registry the typed path uses, via the
    /// type-erased `ManagedHandle` view.
    fn lookup_any_for_slot(
        &self,
        key: &ResourceKey,
        scope: &ScopeLevel,
    ) -> Result<Arc<dyn crate::registry::ManagedHandle>, Error> {
        use crate::registry::HandleLookupOutcome;
        self.shutdown_guard()?;
        match self.registry.get_handle(key, scope) {
            HandleLookupOutcome::Found(any) => Ok(any),
            HandleLookupOutcome::NotFound => Err(Error::not_found(key)),
            // Fail closed: do not drive a rotation/revoke hook against an
            // arbitrarily-chosen tenant's row when several resolved-
            // credential rows share this `(key, scope)`. The engine's
            // per-slot fan-out targets the specific resolved row.
            HandleLookupOutcome::Ambiguous { rows } => Err(Error::ambiguous(format!(
                "{key}: {rows} resolved-credential registrations exist at this scope; \
                 slot rotation/revoke must target a resolved row, not an ambiguous \
                 (key, scope)"
            ))
            .with_resource_key(key.clone())),
        }
    }

    /// Returns whether a registry row exists for
    /// `(key, scope bag, slot_identity)`, keyed by the **collision-free
    /// structural** resolved-credential identity.
    ///
    /// This is the engine-facing entry: the engine records a structural
    /// [`SlotIdentity`](crate::dedup::SlotIdentity) at activation and asks
    /// the same structural identity here, so a row is visible *only* under
    /// its exact resolved binding set (no digest aliasing).
    #[must_use]
    pub fn has_registered_for_scope_identity(
        &self,
        key: &ResourceKey,
        scope: &nebula_core::Scope,
        slot_identity: &crate::dedup::SlotIdentity,
    ) -> bool {
        use crate::registry::AcquireLookupOutcome;
        if self.shutdown_guard().is_err() {
            return false;
        }
        matches!(
            self.registry.get_acquire_for(key, scope, slot_identity),
            AcquireLookupOutcome::Found { .. }
        )
    }

    /// Returns whether a registry row exists for
    /// `(key, scope level, slot_identity)`, keyed by the **collision-free
    /// structural** resolved-credential identity.
    ///
    /// Prefer
    /// [`has_registered_for_scope_identity`](Self::has_registered_for_scope_identity)
    /// when the full scope bag is available (execution + org/workspace).
    #[must_use]
    pub fn has_registered_for_identity(
        &self,
        key: &ResourceKey,
        scope: &ScopeLevel,
        slot_identity: &crate::dedup::SlotIdentity,
    ) -> bool {
        let scope_bag = crate::context::minimal_scope_for_level(scope);
        self.has_registered_for_scope_identity(key, &scope_bag, slot_identity)
    }

    /// [`lookup_any_for_slot`](Self::lookup_any_for_slot) pinned to a
    /// resolved per-slot credential identity via
    /// [`Registry::get_for`](crate::registry::Registry::get_for).
    ///
    /// [`get_for`](crate::registry::Registry::get_for) returns the
    /// 2-variant [`PinnedLookup`](crate::registry::PinnedLookup): a
    /// resolved slot identity pins exactly one `(scope, slot_identity)` row
    /// by construction, so there is **no `Ambiguous` case to map** — the
    /// "registry invariant breach" arm the old `u64` digest path had to
    /// fabricate a fail-closed deny for is now type-unrepresentable.
    fn lookup_any_for_slot_identity_structural(
        &self,
        key: &ResourceKey,
        scope: &ScopeLevel,
        slot_identity: &crate::dedup::SlotIdentity,
    ) -> Result<Arc<dyn crate::registry::ManagedHandle>, Error> {
        use crate::registry::PinnedHandleLookup;
        self.shutdown_guard()?;
        match self.registry.get_handle_for(key, scope, slot_identity) {
            PinnedHandleLookup::Found(any) => Ok(any),
            PinnedHandleLookup::NotFound => Err(Error::not_found(key)),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nebula_core::ResourceKey;

    use super::{Manager, SlotHookDirection};
    use crate::runtime::acquire_loop::{RetiredCleanupObservation, SlotHookObservation};
    use crate::{ErrorKind, ManagerConfig, ResourceEvent};

    #[tokio::test]
    async fn admitted_hook_abandonment_emits_one_terminal_metric_and_redacted_event() {
        let manager = Manager::with_config(
            ManagerConfig::default()
                .with_metrics_registry(Arc::new(nebula_metrics::MetricsRegistry::new())),
        );
        let mut events = manager.subscribe_events();
        let (settlement, admission) = manager.slot_hook_settlement(
            ResourceKey::new("abandoned-hook").expect("valid static resource key"),
            "credential".to_owned(),
            SlotHookDirection::Refresh,
        );

        admission.admit();
        drop(settlement);

        let snapshot = manager
            .metrics()
            .expect("configured manager exposes metrics")
            .snapshot()
            .slot_refresh_outcomes;
        assert_eq!(snapshot.success, 0);
        assert_eq!(snapshot.failed, 0);
        assert_eq!(snapshot.timed_out, 0);
        assert_eq!(snapshot.abandoned, 1);

        let event = events
            .try_recv()
            .expect("abandonment emits a terminal event");
        let ResourceEvent::SlotRefreshFailed { kind, message, .. } = event else {
            panic!("abandonment must emit the refresh failure observation")
        };
        assert_eq!(kind, ErrorKind::Cancelled);
        assert_eq!(
            message.as_str(),
            "admitted credential hook was abandoned before producing a result"
        );
        assert!(
            events.try_recv().is_none(),
            "abandonment must emit exactly one terminal event"
        );
    }

    #[tokio::test]
    async fn retained_cleanup_faults_are_separate_from_hook_terminal_settlement() {
        let cases = [
            (
                RetiredCleanupObservation::Failed(ErrorKind::Permanent),
                ErrorKind::Permanent,
            ),
            (RetiredCleanupObservation::TimedOut, ErrorKind::Backpressure),
            (RetiredCleanupObservation::Abandoned, ErrorKind::Cancelled),
        ];

        for (cleanup_observation, expected_kind) in cases {
            let manager = Manager::with_config(
                ManagerConfig::default()
                    .with_metrics_registry(Arc::new(nebula_metrics::MetricsRegistry::new())),
            );
            let mut events = manager.subscribe_events();
            let (settlement, admission) = manager.slot_hook_settlement(
                ResourceKey::new("retained-cleanup-fault").expect("valid static resource key"),
                "credential".to_owned(),
                SlotHookDirection::Refresh,
            );
            admission.admit();
            let cleanup_observer = settlement
                .settle(SlotHookObservation::Completed)
                .expect("manager installs retained cleanup observation");
            cleanup_observer(cleanup_observation);

            let snapshot = manager
                .metrics()
                .expect("configured manager exposes metrics")
                .snapshot();
            assert_eq!(snapshot.release_errors, 1);
            assert_eq!(snapshot.slot_refresh_outcomes.success, 1);
            assert_eq!(snapshot.slot_refresh_outcomes.failed, 0);
            assert_eq!(snapshot.slot_refresh_outcomes.timed_out, 0);
            assert_eq!(snapshot.slot_refresh_outcomes.abandoned, 0);

            let mut hook_successes = 0;
            let mut cleanup_failures = 0;
            while let Some(event) = events.try_recv() {
                match event {
                    ResourceEvent::SlotRefreshed { .. } => hook_successes += 1,
                    ResourceEvent::RetiredCleanupFailed { kind, .. } => {
                        assert_eq!(kind, expected_kind);
                        cleanup_failures += 1;
                    },
                    ResourceEvent::SlotRefreshFailed { .. } => {
                        panic!("retained cleanup must not rewrite the hook terminal outcome")
                    },
                    _ => {},
                }
            }
            assert_eq!(hook_successes, 1);
            assert_eq!(cleanup_failures, 1);
        }
    }
}
