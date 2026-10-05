//! The acquire pipeline past lookup, the rate-limit wait and the strict
//! read, as a row reaches it without a `Manager` borrow.
//!
//! [`Manager::run_acquire`](super::Manager) waits for the row's rate limit,
//! reads the row's bound credentials outside every lock, then hands the
//! rest to [`AcquireLink::acquire_admitted`]: the in-flight count, the
//! admission checks under `Manager.admission` (lock #1), the recovery gate,
//! the dispatch and the hand-out check. A managed row facade
//! ([`ResourceHandle`](crate::call::ResourceHandle)) checks out one instance per
//! attempt long after it was built, after its own quota and row-gate waits,
//! through the same function — so an acquire and a row attempt are admitted
//! by one body. [`AcquireLink::dispatch_checkout`] is the guarded framework
//! acquire loop both dispatch.
//!
//! [`AdmissionLink`] alone reaches only strict rows (through their
//! `CredentialReads`); this link is held by the manager and by every row
//! facade, strict or not.

use std::{sync::Arc, time::Duration, time::Instant};

use nebula_core::context::Context as _;

use super::{
    AdmissionLink, InFlightCounter, Manager, StrictReading,
    gate::{GateAdmission, admit_through_gate, settle_gate_admission},
};
use crate::{
    context::ResourceContext,
    error::Error,
    events::ResourceEvent,
    guard::{DrainTracker, ResourceGuard},
    hook_guard::{HookFault, guard_author_hook},
    metrics::ResourceOpsMetrics,
    options::AcquireOptions,
    registry::ManagedHandle as _,
    resource::Provider,
    runtime::managed::ManagedResource,
    topology::Topology,
};

/// The manager state an admitted acquire needs after lookup: the admission
/// link, the manager-wide drain tracker, the operation metrics and the
/// slow-acquire threshold. See the module docs.
#[derive(Clone)]
pub(crate) struct AcquireLink {
    admission: AdmissionLink,
    drain_tracker: DrainTracker,
    metrics: Option<ResourceOpsMetrics>,
    slow_threshold: Option<Duration>,
}

impl std::fmt::Debug for AcquireLink {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AcquireLink")
            .field("admission", &self.admission)
            .field("slow_threshold", &self.slow_threshold)
            .finish_non_exhaustive()
    }
}

impl AcquireLink {
    pub(crate) fn new(
        admission: AdmissionLink,
        drain_tracker: DrainTracker,
        metrics: Option<ResourceOpsMetrics>,
        slow_threshold: Option<Duration>,
    ) -> Self {
        Self {
            admission,
            drain_tracker,
            metrics,
            slow_threshold,
        }
    }

    /// `Manager.admission`, the shutdown fence, cancellation and events.
    pub(crate) fn admission(&self) -> &AdmissionLink {
        &self.admission
    }

    /// The manager's operation counters, when configured.
    pub(crate) fn metrics(&self) -> Option<&ResourceOpsMetrics> {
        self.metrics.as_ref()
    }

    /// The manager-wide slow-acquire threshold.
    pub(crate) fn slow_threshold(&self) -> Option<Duration> {
        self.slow_threshold
    }

    /// Admits an acquire whose rate-limit wait and strict read (`reading`,
    /// `None` for a row that reads nothing) are done, then dispatches it.
    ///
    /// Pre-counts the acquire on the manager-wide and per-resource drain
    /// trackers, then, under `Manager.admission` (lock #1): the post-count
    /// taint and shutdown re-check, the reading applied to the row's gate,
    /// the row's suspension and phase, and the capture of the admission
    /// generation. Then the recovery gate, `dispatch` outside every lock,
    /// and the hand-out check against the captured generation (I1–I3, I7 in
    /// the [`manager`](crate::manager) module docs). `started` is when the
    /// caller's acquire began, for the wait metrics.
    ///
    /// # Errors
    ///
    /// The refusal of whichever check refused, or `dispatch`'s error.
    ///
    /// # Cancel safety
    ///
    /// As [`Manager::acquire`]: dropping the future releases the in-flight
    /// count and the recovery-gate ticket; an instance in flight is
    /// destroyed through the release queue.
    pub(crate) async fn acquire_admitted<R, F, Fut>(
        &self,
        managed: Arc<ManagedResource<R>>,
        ctx: &ResourceContext,
        options: &AcquireOptions,
        reading: Option<&StrictReading>,
        started: Instant,
        mut dispatch: F,
    ) -> Result<ResourceGuard<R>, Error>
    where
        R: Provider,
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<ResourceGuard<R>, Error>> + Send,
    {
        // Pre-count this acquire on both the manager-wide and per-resource
        // in-flight trackers, from the moment `lookup()` succeeds. RAII
        // decrements + notifies on every failure / cancel / panic path; on
        // success the slot is handed off to the resulting `ResourceGuard` and
        // held continuously until the guard drops. The `AcqRel` increment here
        // is strictly before the post-taint re-check below. Two-phase-revoke
        // invariant: see the `manager` module documentation.
        let (in_flight, admission) = {
            // Serialize readiness admission with credential demotion and
            // promotion. Once this counter is installed under the same gate,
            // a later replacement may demote the row but cannot retroactively
            // invalidate an acquire admitted against the preceding material.
            let _admission = self.admission.lock();
            let in_flight =
                InFlightCounter::new(self.drain_tracker.clone(), managed.in_flight_tracker());
            // Post-count re-check — now that this acquire is reflected in the
            // per-resource counter `revoke_slot` drains *and* the manager-wide
            // `drain_tracker` `graceful_shutdown` drains, re-observe both revoke
            // taint (closes the revoke-vs-acquire TOCTOU) and `shutting_down`
            // (closes the symmetric shutdown-vs-acquire use-after-drain).
            self.reject_if_tainted_or_shutting_down_post_count::<R>(&managed)?;
            // Strict credential admission, phase 2: re-check the installed
            // material and apply what the read saw (reopen, readmit, suspend)
            // before the suspension check below reads the gate. See
            // `strict_admission` and invariant I7.
            if let (Some(reading), Some(reads)) = (reading, managed.credential_reads.as_deref()) {
                self.admission.apply_strict_reading_under_admission(
                    &R::key(),
                    &*managed,
                    reading,
                    reads,
                )?;
            }
            // A bound credential denying use suspends the row: refuse before
            // the phase check (a suspended row keeps its phase) and before
            // the recovery gate (suspension is not backend ill health).
            if let Some(suspension) = managed.admission.suspension() {
                return Err(Manager::credential_unavailable_error(
                    &R::key(),
                    suspension.reason(),
                ));
            }
            if !managed.phase().is_accepting() {
                return Err(Error::backpressure(format!(
                    "{}: resource is {} and cannot accept acquires",
                    R::key(),
                    managed.phase()
                ))
                .with_resource_key(R::key()));
            }
            // Capture the admission generation under the same gate that
            // publishes and retires it: this lease is admitted under exactly
            // this generation and observes its closing notice. A row with no
            // current generation was retired after the checks above could
            // observe why (a removed or dropped-manager row).
            let admission = managed
                .admission
                .current()
                .ok_or_else(|| Manager::closed_admission_error::<R>(&managed, None))?;
            (in_flight, admission)
        };
        let gate_admission = admit_through_gate(&managed.recovery_gate)?;

        // Publish a `RetryAttempt` event when this acquire is the recovery
        // probe (the CAS-claimed single-probe slot that follows a transient
        // backend failure). `backoff_on_fail` carries the delay the gate
        // would impose *if this probe fails again* — the next caller's wait,
        // not a wait this acquire incurs. Emitted **before** `dispatch()` so
        // observers see the attempt go out rather than only the result. The
        // error field carries the prior failure message snapshotted in
        // `admit_through_gate` before the CAS rotated the gate.
        if let GateAdmission::Probe {
            attempt,
            backoff_on_fail,
            last_failure,
            ..
        } = &gate_admission
        {
            self.admission.emit(ResourceEvent::RetryAttempt {
                key: R::key(),
                attempt: *attempt,
                backoff: *backoff_on_fail,
                error: last_failure.clone().unwrap_or_default(),
            });
        }

        let result = match dispatch().await {
            // Hand-out check: the generation this acquire was admitted under
            // closed while it was in flight (a taint, removal or shutdown
            // straddled the create). The caller never receives the lease.
            // The built guard carries the in-flight slot into ordinary
            // release, so the entry returns (or, fenced, is destroyed)
            // before the revoke / shutdown drain observes the slot free.
            // See the `manager` module docs, "Admission generations".
            Ok(guard) if admission.is_closed() => {
                drop(guard.with_drain_tracker(in_flight.release_to_guard()));
                let refused = Manager::closed_admission_error::<R>(&managed, Some(&admission));
                tracing::debug!(
                    resource.key = %R::key(),
                    admission = admission.seq(),
                    error.kind = ?refused.kind(),
                    "acquire refused at hand-out: admission generation closed in flight"
                );
                Err(refused)
            },
            Ok(guard) => Ok(guard
                .with_admission(admission)
                .with_drain_tracker(in_flight.release_to_guard())),
            Err(error) => Err(error),
        };

        // Settle the gate ticket based on the acquire result. #322: this
        // makes the ticket ownership end-to-end — on success we `resolve`,
        // on retryable error we `fail_transient`, on permanent error we
        // `fail_permanent`. The `Drop` impl of `RecoveryTicket` covers
        // cancellation/panic paths. A hand-out refusal is `Revoked` or
        // `Cancelled`, neither of which is a backend-health signal.
        settle_gate_admission(gate_admission, &result);
        self.record_acquire_result(&result, started, ctx, options);
        // Attach the manager's event bus so the guard's `Drop` emits
        // `ResourceEvent::Released`. Done here, on the success path only,
        // because failed acquires never minted a guard to begin with —
        // there is nothing to release.
        result.map(|h| {
            h.with_event_bus(Arc::clone(self.admission.events()))
                .with_hold_watchdog(R::max_hold_duration(), ctx, self.metrics.clone())
        })
    }

    /// The framework acquire loop
    /// ([`ManagedResource::run_acquire_loop`]) bounded by `hook_timeout` and
    /// isolated from a panicking topology hook.
    ///
    /// Foolproofing for open (third-party) topologies: a careless
    /// `impl Topology` cannot wedge the caller by hanging, nor crash it by
    /// panicking. The dropped loop future releases the permit and destroys
    /// any in-flight entry via `EntryCreateGuard`.
    pub(crate) async fn dispatch_checkout<R>(
        &self,
        managed: &Arc<ManagedResource<R>>,
        ctx: &ResourceContext,
        options: &AcquireOptions,
        hook_timeout: Duration,
    ) -> Result<ResourceGuard<R>, Error>
    where
        R: Provider,
        R::Topology: Topology<R>,
    {
        // SAFETY (unwind): any instance in flight inside the acquire
        // loop is held by an `EntryCreateGuard` whose `Drop` destroys it,
        // and the revoke-epoch/taint reads happen before the guarded
        // await — so a caught panic unwinds through the `EntryCreateGuard`
        // (tearing the half-built slot down) and leaves no torn state.
        match guard_author_hook(
            hook_timeout,
            managed.run_acquire_loop(ctx, options, self.metrics.clone()),
        )
        .await
        {
            Ok(result) => result,
            Err(fault) => {
                fault.observe(&R::key(), "acquire");
                match fault {
                    HookFault::Panicked => Err(Error::permanent(format!(
                        "{}: topology acquire pipeline panicked — the resource's \
                         `impl Topology` hook unwound (isolated, caller not crashed)",
                        R::key()
                    ))),
                    HookFault::TimedOut => Err(Error::backpressure(format!(
                        "{}: acquire exceeded {hook_timeout:?} — the topology's \
                         create/accept/prepare hooks did not complete in time",
                        R::key()
                    ))),
                }
            },
        }
    }

    /// [`Manager::reject_if_tainted_or_shutting_down_post_count`], through
    /// the link's shutdown fence.
    fn reject_if_tainted_or_shutting_down_post_count<R: Provider>(
        &self,
        managed: &ManagedResource<R>,
    ) -> Result<(), Error> {
        if managed.is_tainted() {
            return Err(Manager::tainted_error::<R>());
        }
        self.admission.shutdown_guard()
    }

    /// Records acquire success/failure in aggregate metrics, the acquire-wait
    /// histogram, and emits the corresponding [`ResourceEvent`]; also checks
    /// the acquire-slow-log threshold.
    fn record_acquire_result<R: Provider>(
        &self,
        result: &Result<ResourceGuard<R>, Error>,
        started: Instant,
        ctx: &ResourceContext,
        options: &AcquireOptions,
    ) {
        // Resolve the resource key once: `R::key()` re-validates and re-interns
        // the literal on each call, and the error path emits up to two events.
        let key = R::key();
        let elapsed = started.elapsed();
        match result {
            Ok(_) => {
                if let Some(m) = &self.metrics {
                    m.record_acquire();
                }
                self.admission.emit(ResourceEvent::AcquireSuccess {
                    key: key.clone(),
                    duration: elapsed,
                });
            },
            Err(e) => {
                if let Some(m) = &self.metrics {
                    m.record_acquire_error();
                }
                // `BackpressureDetected` is a topology-pressure signal
                // (semaphore full, max sessions reached). It is a strict
                // subset of `AcquireFailed` — we emit both so subscribers
                // that filter on pressure get a typed event without having
                // to parse error strings, while the unified
                // `AcquireFailed` stream remains the canonical "acquire
                // didn't succeed" feed.
                if matches!(e.kind(), crate::error::ErrorKind::Backpressure) {
                    self.admission
                        .emit(ResourceEvent::BackpressureDetected { key: key.clone() });
                }
                self.admission.emit(ResourceEvent::AcquireFailed {
                    key: key.clone(),
                    kind: e.kind().clone(),
                    error: e.to_string(),
                });
            },
        }

        // Acquire wait-time histogram + waited/timed-out counters. A
        // deadline is "timed out" when it had already elapsed by the time
        // this (failed) acquire completed — mirrors sqlx/bb8's notion of an
        // acquire timeout, independent of which internal error path produced
        // the failure. Reuses the completion instant already captured in
        // `elapsed` (`started + elapsed`) rather than a fresh `Instant::now()`
        // here: the event emission above takes nonzero time, so a fresh read
        // could observe the deadline as elapsed even for a failure that
        // actually completed strictly before it.
        if let Some(m) = &self.metrics {
            let completed_at = started + elapsed;
            let timed_out = result.is_err() && options.deadline.is_some_and(|d| completed_at >= d);
            m.record_acquire_wait(elapsed, timed_out);
        }

        // Acquire-slow-log threshold — at most one WARN per acquire,
        // checked once here at completion. `AcquireOptions` overrides the
        // manager-wide default.
        if let Some(threshold) = options.acquire_slow_threshold.or(self.slow_threshold)
            && elapsed > threshold
        {
            tracing::warn!(
                target: "resource",
                %key,
                scope = ?ctx.scope(),
                elapsed = ?elapsed,
                threshold = ?threshold,
                "acquire exceeded the slow-acquire threshold"
            );
        }
    }
}
