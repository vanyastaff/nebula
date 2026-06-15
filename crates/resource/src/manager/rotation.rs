//! Credential slot rotation / revoke: the refresh + two-phase
//! (synchronous-taint then cancellation-safe drain+hook) revoke surface,
//! their shared post-resolution dispatch, and the type-erased `(key, scope)`
//! row resolution helpers the rotation entry points use.

use std::{sync::Arc, time::Instant};

use nebula_core::{ResourceKey, ScopeLevel};

use super::Manager;
use crate::{
    error::Error,
    events::ResourceEvent,
    hook_guard::{DEFAULT_AUTHOR_HOOK_CEILING, HookFault, guard_author_hook},
};

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
/// `tokio::time::timeout` wrapping the whole tail; the three terminal states
/// are reported here rather than inferred from a dropped outer future. See
/// the [`manager`](crate::manager) module docs for why an outer timeout
/// wrapper would be unsafe (it could drop the future before the hook ran):
///
/// - [`Done`](Self::Done) — the revoke hook completed `Ok`.
/// - [`HookFailed`](Self::HookFailed) — the hook returned `Err` (carried
///   verbatim).
/// - [`HookTimedOut`](Self::HookTimedOut) — the hook itself did not
///   complete within the budget. The row stays tainted (the taint ran in
///   the synchronous phase-1); only a *hung hook* is bounded, never the
///   taint, and never at the cost of skipping a hook after a slow drain.
#[derive(Debug)]
#[must_use = "the revoke tail outcome must be recorded (it is not a silent success)"]
pub enum RevokeTail {
    /// Drain + revoke hook completed; the hook returned `Ok`. (A
    /// best-effort drain timeout that still reached a successful hook is
    /// still `Done` — the drain timeout is non-fatal.)
    Done,
    /// The revoke hook returned an error. The row stays tainted; the
    /// inner error is preserved for the caller's outcome accounting.
    HookFailed(Error),
    /// The revoke hook did not complete within the per-resource budget
    /// (a wedged `on_credential_revoke`). The row stays tainted; this is
    /// the only thing the budget bounds.
    HookTimedOut,
}

impl RevokeTail {
    /// Adapts the tail outcome to `Result<(), Error>` for the back-compat
    /// convenience callers ([`Manager::revoke_slot`] /
    /// [`Manager::revoke_slot_for_identity`]) that run taint+tail
    /// back-to-back and
    /// only need pass/fail. A hook timeout becomes a retryable transient
    /// error (the row is tainted; a later retry is meaningful), distinct
    /// from a hook failure which carries the hook's own error.
    pub(super) fn into_result(self) -> Result<(), Error> {
        match self {
            RevokeTail::Done => Ok(()),
            RevokeTail::HookFailed(e) => Err(e),
            RevokeTail::HookTimedOut => Err(Error::transient(
                "revoke hook timed out — row stays tainted, no new leases",
            )),
        }
    }
}

impl Manager {
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
    /// Emits [`ResourceEvent::SlotRefreshed`] on success or
    /// [`ResourceEvent::SlotRefreshFailed`] (with an already-stringified,
    /// credential-free error) on failure, and records the corresponding
    /// slot-refresh metric.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no resource is registered for
    ///   `key` at `scope`.
    /// - [`ErrorKind::Cancelled`](crate::error::ErrorKind::Cancelled) if the manager is shutting
    ///   down.
    /// - Whatever the resource's `on_credential_refresh` hook maps into [`Error`].
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
    ) -> Result<(), Error> {
        let managed = self.lookup_any_for_slot(key, &scope)?;
        self.refresh_resolved(key, slot, managed).await
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
    ) -> Result<(), Error> {
        let managed = self.lookup_any_for_slot_identity_structural(key, &scope, slot_identity)?;
        self.refresh_resolved(key, slot, managed).await
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
    ) -> Result<(), Error> {
        let started = Instant::now();
        tracing::Span::current().record("topology", managed.topology_tag().as_str());

        // Bound + isolate the author's `on_credential_refresh` hook: one that
        // hangs or panics must fail closed as a recorded refresh outcome, never
        // wedge or crash the rotation fan-out. No caller threads a refresh
        // budget here, so the framework ceiling is the backstop — which is what
        // makes the `timed_out` outcome reachable for refresh at all.
        //
        // SAFETY (unwind): the dispatch only mutates the author's own borrowed
        // instances under the store lock; a caught panic drops that lock guard
        // (releasing the lock) and leaves the store's structure/epoch intact —
        // at worst one instance is left un-refreshed (re-refreshed or evicted on
        // a later sweep), never a torn store.
        let guarded = guard_author_hook(
            DEFAULT_AUTHOR_HOOK_CEILING,
            managed.dispatch_on_refresh(slot),
        )
        .await;
        tracing::Span::current().record("duration_ms", started.elapsed().as_millis() as u64);

        // Exactly one outcome per dispatch; the attempts total is the sum
        // across `outcome` labels (success + failed + timed_out).
        let (outcome, result): (crate::metrics::SlotDispatchOutcome, Result<(), Error>) =
            match guarded {
                Ok(Ok(())) => (crate::metrics::SlotDispatchOutcome::Success, Ok(())),
                Ok(Err(e)) => (crate::metrics::SlotDispatchOutcome::Failed, Err(e)),
                Err(HookFault::Panicked) => (
                    crate::metrics::SlotDispatchOutcome::Failed,
                    Err(Error::permanent(
                        "slot refresh hook panicked — the topology's \
                         `on_credential_refresh` hook unwound (isolated, fan-out not crashed)",
                    )),
                ),
                Err(HookFault::TimedOut) => (
                    crate::metrics::SlotDispatchOutcome::TimedOut,
                    Err(Error::backpressure(
                        "slot refresh hook timed out — the topology's \
                         `on_credential_refresh` hook did not complete in time",
                    )),
                ),
            };

        if let Some(m) = &self.metrics {
            m.record_slot_refresh_outcome(outcome);
        }
        match &result {
            Ok(()) => {
                self.emit(ResourceEvent::SlotRefreshed {
                    key: key.clone(),
                    slot: slot.to_owned(),
                });
                tracing::debug!("slot refresh hook completed");
            },
            Err(e) => {
                self.emit(ResourceEvent::SlotRefreshFailed {
                    key: key.clone(),
                    slot: slot.to_owned(),
                    error: e.to_string(),
                });
                tracing::warn!(error = %e, "slot refresh hook failed");
            },
        }
        result
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
        Ok(Self::taint_now(key, slot, managed))
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
        Ok(Self::taint_now(key, slot, managed))
    }

    /// Applies the taint synchronously and packages the [`TaintedSlot`]
    /// handle. Shared tail of [`taint_slot`](Self::taint_slot) /
    /// [`taint_slot_for_identity`](Self::taint_slot_for_identity); the
    /// safety-critical
    /// invariant — *taint is fully applied before this returns* — is written
    /// once here. This is **phase 1** of the two-phase revoke; see the
    /// [`manager`](crate::manager) module docs for the canonical invariant
    /// (why both stores are synchronous-before-`.await`, the TOCTOU close,
    /// and the revoke-epoch fence).
    fn taint_now(
        key: &ResourceKey,
        slot: &str,
        managed: Arc<dyn crate::registry::ManagedHandle>,
    ) -> TaintedSlot {
        tracing::Span::current().record("topology", managed.topology_tag().as_str());
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
        TaintedSlot {
            key: key.clone(),
            slot: slot.to_owned(),
            managed,
            tainted_at: Instant::now(),
        }
    }

    /// Default per-resource revoke budget for the back-compat
    /// back-to-back convenience callers ([`revoke_slot`](Self::revoke_slot)
    /// / [`revoke_slot_for_identity`](Self::revoke_slot_for_identity)).
    ///
    /// 30 s — the same budget the manager-wide `graceful_shutdown` drain
    /// uses and the value [`drain_and_revoke`](Self::drain_and_revoke)
    /// previously hard-coded for the drain wait. The engine rotation
    /// fan-out does **not** use this: it passes its own per-resource
    /// rotation budget so the timeout has one owner end-to-end (resource runtime status
    /// §Deferred / #690 review).
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
    /// 3. Emit [`ResourceEvent::SlotRevoked`] / `SlotRevokeFailed`.
    ///
    /// **Single budget owner (per-resource revoke deferral / #690 review).** The
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
    /// The caller **must not** wrap this call in its own
    /// `tokio::time::timeout`. The previous design did, and a slow drain
    /// could make that outer timeout elapse and **drop the whole future
    /// before the hook ran** — silently skipping the documented
    /// "hook still runs after a timed-out drain" guarantee. Bounding both
    /// waits *inside* this method (one owner, no outer wrapper) means a
    /// timed-out drain can never skip the hook, and only a hung hook is
    /// bounded — never the taint.
    ///
    /// **Cancellation-safety.** The taint is *not* in this future — it
    /// ran in the synchronous
    /// [`taint_slot_for_identity`](Self::taint_slot_for_identity)
    /// phase. So if this future *is* dropped anyway (an outer abort, task
    /// cancel), the row stays tainted and consistent: new acquires are
    /// still rejected, the credential is never silently un-revoked.
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
        //    A drain timeout is *terminal* for this dispatch's outcome
        //    metric: it records `TimedOut` and the subsequent hook
        //    success/failure does NOT record a second outcome (one dispatch
        //    = exactly one outcome). The hook still runs and its event /
        //    returned outcome are unaffected — this is the contract the
        //    removed outer `tokio::time::timeout` wrapper used to break.
        let drain_result = managed.wait_for_in_flight_drain(drain_timeout).await;
        let drain_timed_out = drain_result.is_err();
        if let Err(outstanding) = &drain_result {
            if let Some(m) = &self.metrics {
                m.record_slot_revoke_outcome(crate::metrics::SlotDispatchOutcome::TimedOut);
            }
            tracing::warn!(
                outstanding = *outstanding,
                "slot revoke: per-resource drain timed out; proceeding to \
                 revoke hook (resource already tainted, no new leases)"
            );
        }

        // 2. Dispatch the revoke hook against the live runtime, bounded by
        //    the SAME per-resource budget AND isolated from an unwinding panic.
        //    This is the only place the budget can cut the tail short: a wedged
        //    `on_credential_revoke` must not pin the fan-out row forever, and a
        //    panicking one must not crash the fan-out — both fail closed with
        //    the row left tainted. A timed-out drain (above) has *already*
        //    consumed the metric outcome, so a hook that then also faults does
        //    not double-record.
        //
        // SAFETY (unwind): the row was tainted synchronously before this await,
        // so a caught panic leaves it tainted (fail-closed — no further leases);
        // the dispatch mutates only borrowed instances under the store lock, so
        // the unwind drops the lock guard and leaves the store intact.
        let hook_outcome =
            guard_author_hook(drain_timeout, managed.dispatch_on_revoke(&slot)).await;
        tracing::Span::current().record("duration_ms", tainted_at.elapsed().as_millis() as u64);

        match hook_outcome {
            Ok(Ok(())) => {
                // Only record Success when the drain did not already record
                // the terminal TimedOut outcome for this dispatch.
                if !drain_timed_out && let Some(m) = &self.metrics {
                    m.record_slot_revoke_outcome(crate::metrics::SlotDispatchOutcome::Success);
                }
                self.emit(ResourceEvent::SlotRevoked {
                    key: key.clone(),
                    slot: slot.clone(),
                });
                tracing::debug!("slot revoke hook completed");
                RevokeTail::Done
            },
            Ok(Err(e)) => {
                if !drain_timed_out && let Some(m) = &self.metrics {
                    m.record_slot_revoke_outcome(crate::metrics::SlotDispatchOutcome::Failed);
                }
                self.emit(ResourceEvent::SlotRevokeFailed {
                    key,
                    slot,
                    error: e.to_string(),
                });
                tracing::warn!(error = %e, "slot revoke hook failed");
                RevokeTail::HookFailed(e)
            },
            Err(HookFault::Panicked) => {
                // The hook unwound. Caught — the fan-out is not crashed and the
                // row stays tainted (phase 1). A panicking revoke is a hook
                // failure: record `Failed` unless the drain already recorded a
                // terminal outcome for this dispatch.
                if !drain_timed_out && let Some(m) = &self.metrics {
                    m.record_slot_revoke_outcome(crate::metrics::SlotDispatchOutcome::Failed);
                }
                let e = Error::permanent(
                    "slot revoke hook panicked — the topology's `on_credential_revoke` \
                     hook unwound (isolated, fan-out not crashed)",
                );
                self.emit(ResourceEvent::SlotRevokeFailed {
                    key,
                    slot,
                    error: e.to_string(),
                });
                tracing::error!("slot revoke hook panicked (row stays tainted, no new leases)");
                RevokeTail::HookFailed(e)
            },
            Err(HookFault::TimedOut) => {
                // The hook itself wedged. The row stays tainted (phase 1).
                // Record `TimedOut` unless the drain already did (one
                // dispatch = exactly one outcome).
                if !drain_timed_out && let Some(m) = &self.metrics {
                    m.record_slot_revoke_outcome(crate::metrics::SlotDispatchOutcome::TimedOut);
                }
                self.emit(ResourceEvent::SlotRevokeFailed {
                    key,
                    slot,
                    error: "revoke hook timed out".to_owned(),
                });
                tracing::warn!(
                    timeout_ms = drain_timeout.as_millis() as u64,
                    "slot revoke hook timed out (row stays tainted, no new leases)"
                );
                RevokeTail::HookTimedOut
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
    pub async fn revoke_slot(
        &self,
        key: &ResourceKey,
        scope: ScopeLevel,
        slot: &str,
    ) -> Result<(), Error> {
        let tainted = self.taint_slot(key, scope, slot)?;
        self.drain_and_revoke(tainted, Self::DEFAULT_REVOKE_DRAIN_TIMEOUT)
            .await
            .into_result()
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
    /// [`revoke_slot`](Self::revoke_slot) this is the back-compat
    /// back-to-back path; the engine fan-out drives the two phases separately
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
    ) -> Result<(), Error> {
        let tainted = self.taint_slot_for_identity(key, scope, slot, slot_identity)?;
        self.drain_and_revoke(tainted, Self::DEFAULT_REVOKE_DRAIN_TIMEOUT)
            .await
            .into_result()
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
        use crate::registry::LookupOutcome;
        self.shutdown_guard()?;
        match self.registry.get(key, scope) {
            LookupOutcome::Found(any) => Ok(any),
            LookupOutcome::NotFound => Err(Error::not_found(key)),
            // Fail closed: do not drive a rotation/revoke hook against an
            // arbitrarily-chosen tenant's row when several resolved-
            // credential rows share this `(key, scope)`. The engine's
            // per-slot fan-out targets the specific resolved row.
            LookupOutcome::Ambiguous { rows } => Err(Error::ambiguous(format!(
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
        use crate::registry::PinnedLookup;
        self.shutdown_guard()?;
        match self.registry.get_for(key, scope, slot_identity) {
            PinnedLookup::Found(any) => Ok(any),
            PinnedLookup::NotFound => Err(Error::not_found(key)),
        }
    }
}
