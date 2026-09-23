//! Credential slot revoke: synchronous taint, then cancellation-safe
//! drain and revoke-hook dispatch.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use nebula_core::{ResourceKey, ScopeLevel};
use nebula_credential::SecretFreeMessage;

use super::{
    EpochRevokeOutcome, Manager, RevokeTail, SlotDispatchOutcome, SlotDrainOutcome,
    SlotHookDirection, TaintedSlot, slot_hook_observation_deadline,
};
use crate::{
    error::Error, events::ResourceEvent, metrics::SlotDispatchMetricOutcome,
    runtime::acquire_loop::SlotHookWaitOutcome,
};

impl Manager {
    /// Terminally clears an identity-pinned slot, then synchronously taints
    /// the row before running the bounded drain and revoke hook.
    ///
    /// Revoke wins over every in-flight or delayed refresh without requiring
    /// the caller to synthesize a material epoch.
    ///
    /// # Errors
    ///
    /// Returns an error when the identity-pinned resource cannot be resolved,
    /// the slot cannot be revoked, or synchronous tainting fails.
    pub async fn revoke_credential_slot_for_identity(
        &self,
        key: &ResourceKey,
        scope: ScopeLevel,
        slot: &str,
        slot_identity: &crate::dedup::SlotIdentity,
    ) -> Result<EpochRevokeOutcome, Error> {
        let managed = self.lookup_any_for_slot_identity_structural(key, &scope, slot_identity)?;
        let update = managed.revoke_credential_slot(slot).map_err(|source| {
            Error::permanent("credential slot revoke failed")
                .with_source(source)
                .with_resource_key(key.clone())
        })?;
        match update {
            crate::SlotUpdate::Revoked => {
                let tainted = self.taint_now(key, slot, managed)?;
                Ok(EpochRevokeOutcome::Applied(
                    self.drain_and_revoke(tainted, Self::DEFAULT_REVOKE_DRAIN_TIMEOUT)
                        .await,
                ))
            },
            crate::SlotUpdate::AlreadyRevoked => Ok(EpochRevokeOutcome::AlreadyRevoked),
            crate::SlotUpdate::Installed | crate::SlotUpdate::Stale { .. } => Err(
                Error::permanent("credential slot revoke returned an invalid install outcome")
                    .with_resource_key(key.clone()),
            ),
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
    pub const DEFAULT_REVOKE_DRAIN_TIMEOUT: Duration = Duration::from_secs(30);

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
        drain_timeout: Duration,
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
        let deadline = slot_hook_observation_deadline(drain_timeout);
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
}
