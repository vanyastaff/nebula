//! Resource guard — the value callers hold while using a resource.
//!
//! A manager-owned lease borrows its topology entry through `Deref`.
//! Explicit release and Drop both transfer that same entry to the cleanup queue.

use std::{
    ops::Deref,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering as AtomicOrdering},
    },
    time::{Duration, Instant},
};

use nebula_core::{ResourceKey, context::Context as _};
use nebula_eventbus::EventBus;
use tokio::sync::{Notify, OwnedSemaphorePermit};

use crate::{
    context::ResourceContext,
    events::ResourceEvent,
    metrics::ResourceOpsMetrics,
    release_queue::ReleaseSubmission,
    resource::Provider,
    runtime::{
        acquire_loop::release_entry,
        managed::{EntryOf, ManagedResource},
    },
    topology::Topology,
    topology_tag::TopologyTag,
};

/// Observable completion state of a consumed resource release.
///
/// Both variants mean the guard was consumed and cleanup ownership was safely
/// transferred or settled. Callers must never retry the consumed release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use = "a deferred release is accepted but has not completed yet"]
#[non_exhaustive]
pub enum ReleaseOutcome {
    /// The queued cleanup completed and its provider result was observed.
    Completed,
    /// Cleanup was accepted by its queue, but waiting from the current cleanup
    /// context could form a dependency cycle or exceed cooperative capacity.
    ///
    /// The queue owns the cleanup and schedules bounded, best-effort execution.
    /// Process termination, queue abandonment, or a worker fault can still
    /// prevent completion; those losses are reported through cleanup
    /// observability. This is an accepted ownership transfer, not an error or
    /// a guarantee that the provider hook will run, and must not be retried.
    Deferred,
}

/// A drain tracker: an in-flight `(active_count, waiters)` pair. One is the
/// manager-wide `graceful_shutdown` tracker; another is each
/// `ManagedResource`'s own counter that `Manager::revoke_slot` drains in
/// isolation. See the [`manager`](crate::manager) module docs for the
/// canonical two-phase-revoke / drain invariant.
pub(crate) type DrainTracker = Arc<(AtomicU64, Notify)>;

/// The `(manager_wide, per_resource)` pair an acquire pre-increments and
/// hands to its [`ResourceGuard`]. Both are decremented + notified when queued
/// release settles: the first unblocks `graceful_shutdown`, the second unblocks the
/// originating resource's isolated revoke drain.
pub(crate) type DrainTrackers = (DrainTracker, DrainTracker);

/// A manager-owned lease over a resource instance.
///
/// Dereferences to the instance inside the actual topology entry; the framework
/// never clones that instance or transfers it outside lifecycle cleanup. Both explicit
/// [`release`](Self::release) and Drop enqueue the same cleanup job.
/// An author may still expose cloneable aliases through its instance API; the
/// framework cannot revoke or account for those external aliases or their work.
///
/// The job holds the admission permit and drain reservations until cleanup
/// completes or is abandoned. Drop is best-effort; use explicit release to
/// observe provider errors. Cancelling its caller does not cancel queued work.
///
/// The guard does not offer a lifecycle-bypassing extraction operation:
///
/// ```compile_fail
/// use nebula_resource::{Provider, ResourceGuard};
/// fn extract<R: Provider>(guard: ResourceGuard<R>) {
///     guard.detach();
/// }
/// ```
///
/// Only the manager constructs resource guards:
///
/// ```compile_fail
/// use nebula_resource::{Provider, ResourceGuard};
/// fn fabricate<R: Provider>(instance: R::Instance) {
///     ResourceGuard::<R>::owned(instance, R::key(), nebula_resource::TopologyTag::Resident);
/// }
/// ```
#[must_use = "dropping a ResourceGuard immediately releases the resource"]
pub struct ResourceGuard<R: Provider> {
    // Only consuming release or Drop takes this entry; usable guards are live.
    entry: Option<EntryOf<R>>,
    managed: Arc<ManagedResource<R>>,
    permit: Option<OwnedSemaphorePermit>,
    checkout_epoch: u64,
    generation: u64,
    tainted: bool,
    metrics: Option<ResourceOpsMetrics>,
    resource_key: ResourceKey,
    topology_tag: TopologyTag,
    acquired_at: Instant,
    drain_counters: Option<DrainTrackers>,
    event_bus: Option<Arc<EventBus<ResourceEvent>>>,
    hold_watchdog: Option<HoldWatchdog>,
}

/// Aborts the hold-deadline watchdog task when the lease ends.
///
/// Without the abort, a released guard's watchdog would stay parked on its
/// timer for the full deadline, so live tasks would grow with
/// `acquire rate × max_hold_duration` instead of with live leases.
struct HoldWatchdog(tokio::task::AbortHandle);

impl Drop for HoldWatchdog {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub(crate) struct GuardIdentity {
    pub(crate) resource_key: ResourceKey,
    pub(crate) topology_tag: TopologyTag,
}

impl<R: Provider> ResourceGuard<R> {
    pub(crate) fn new(
        managed: Arc<ManagedResource<R>>,
        entry: EntryOf<R>,
        checkout_epoch: u64,
        permit: Option<OwnedSemaphorePermit>,
        generation: u64,
        metrics: Option<ResourceOpsMetrics>,
        identity: GuardIdentity,
    ) -> Self {
        Self {
            entry: Some(entry),
            managed,
            permit,
            checkout_epoch,
            generation,
            tainted: false,
            metrics,
            resource_key: identity.resource_key,
            topology_tag: identity.topology_tag,
            acquired_at: Instant::now(),
            drain_counters: None,
            event_bus: None,
            hold_watchdog: None,
        }
    }

    /// Attaches the manager-wide + per-resource drain trackers for shutdown
    /// and revoke coordination.
    ///
    /// **Caller-owned increment**: this method does NOT increment either
    /// counter. Callers (the `Manager` acquire paths) must pre-increment
    /// both before any `await` past `lookup()` (via `InFlightCounter`) and
    /// hand the *already-counted slots* off here. The guard then owns both
    /// and decrements + notifies each when queued release settles.
    ///
    /// This caller-owned ordering is what makes the pre-count span the whole
    /// guard lifetime, closing both the `graceful_shutdown` race and the
    /// revoke-vs-acquire TOCTOU. See the [`manager`](crate::manager) module
    /// docs for the canonical invariant.
    pub(crate) fn with_drain_tracker(mut self, trackers: DrainTrackers) -> Self {
        self.drain_counters = Some(trackers);
        self
    }

    /// Attaches the manager's event bus so this guard emits
    /// [`ResourceEvent::Released`] on drop. Wired by
    /// [`Manager::run_acquire`](crate::manager::Manager) right after the
    /// topology runtime hands back the guard. Without this, the guard
    /// silently skips the released event — the existing best-effort emit
    /// discipline applies here too.
    pub(crate) fn with_event_bus(mut self, event_bus: Arc<EventBus<ResourceEvent>>) -> Self {
        self.event_bus = Some(event_bus);
        self
    }

    /// Arms the optional hold-deadline watchdog (HikariCP-style leak detection).
    ///
    /// When `deadline` is `Some(d)` and an event bus is attached, spawns a
    /// background task that fires after `d` elapses. The guard owns the task's
    /// abort handle, so a lease that ends first cancels it; a task that fires
    /// therefore means the lease has been held past its deadline — a likely
    /// leaked or hung guard pinning a bounded slot — and the watchdog emits a
    /// [`ResourceEvent::HoldDeadlineExceeded`] plus a `WARN` span, both
    /// carrying the acquiring context's execution id, workflow id, and
    /// tracing span id — enough to go find *who* leaked it — and bumps
    /// `metrics`' `hold_deadline_exceeded` counter if metrics are
    /// configured. Live watchdog tasks are therefore bounded by live leases.
    ///
    /// `deadline = None` (the default [`Provider::max_hold_duration`]) is a
    /// no-op: no task is spawned and the guard pays nothing. `ctx`'s
    /// identifiers are read eagerly (before the task is spawned) since they
    /// are cheap `Copy` ids, not the context itself — the spawned task does
    /// not borrow or outlive `ctx`.
    pub(crate) fn with_hold_watchdog(
        mut self,
        deadline: Option<Duration>,
        ctx: &ResourceContext,
        metrics: Option<ResourceOpsMetrics>,
    ) -> Self {
        let Some(deadline) = deadline else {
            return self;
        };
        let Some(event_bus) = self.event_bus.clone() else {
            return self;
        };
        let key = self.resource_key.clone();
        let acquired_at = self.acquired_at;
        let execution_id = ctx.execution_id();
        let workflow_id = ctx.scope().workflow_id;
        let span_id = ctx.span_id();
        let task = tokio::spawn(async move {
            tokio::time::sleep(deadline).await;
            // Not aborted ⇒ the lease outlived its hold deadline.
            let held = acquired_at.elapsed();
            tracing::warn!(
                resource = %key,
                held_secs = held.as_secs_f64(),
                deadline_secs = deadline.as_secs_f64(),
                ?execution_id,
                ?workflow_id,
                ?span_id,
                "resource lease exceeded its hold deadline — possible leaked or hung guard"
            );
            if let Some(m) = &metrics {
                m.record_hold_deadline_exceeded();
            }
            let _ = event_bus.emit(ResourceEvent::HoldDeadlineExceeded {
                key,
                held,
                deadline,
                execution_id,
                workflow_id,
                span_id,
            });
        });
        self.hold_watchdog = Some(HoldWatchdog(task.abort_handle()));
        self
    }

    /// Marks the lease as tainted so release bypasses recycling.
    pub fn taint(&mut self) {
        self.tainted = true;
    }

    /// Returns how long this guard has been held.
    pub fn hold_duration(&self) -> Duration {
        self.acquired_at.elapsed()
    }

    /// The row's limit.
    ///
    /// Every acquire already consumed one permit. Calls made within the lease
    /// are paced by the [`Limited`](crate::rate_limit::Limited) client the
    /// resource built in `create`; use this handle only to
    /// [`penalize`](crate::rate_limit::ResourceLimiter::penalize) on a signal
    /// that does not come back from a call.
    pub fn limits(&self) -> &Arc<crate::rate_limit::ResourceLimiter> {
        &self.managed.rate_limiter
    }

    /// Returns the resource key for this guard.
    pub fn resource_key(&self) -> &ResourceKey {
        &self.resource_key
    }

    /// Returns the topology tag identifying which topology this guard came from.
    pub fn topology_tag(&self) -> TopologyTag {
        self.topology_tag
    }

    /// Returns the registration generation at acquisition.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Releases this lease and observes whether cleanup completed or was deferred.
    ///
    /// # Errors
    ///
    /// Returns a provider or release-hook error only when this call can observe
    /// the non-deferred cleanup path. It returns cancellation if the queue
    /// rejects or abandons the job. The guard is consumed and its reservation
    /// settles on every error; it cannot and must not be retried. A deferred
    /// cleanup remains queue-owned, and any later asynchronous failure is
    /// reported through tracing and resource metrics rather than this result.
    ///
    /// # Cancel safety
    ///
    /// Dropping the waiting caller discards only its receipt. The queue owns
    /// the entry and runs cleanup independently, bounded by its worker budget.
    ///
    /// # Examples
    ///
    /// A deferred result is an accepted ownership transfer. The consumed guard
    /// is no longer available and the operation must never be retried:
    ///
    /// ```
    /// use nebula_resource::{Error, Provider, ReleaseOutcome, ResourceGuard};
    ///
    /// async fn release_once<R: Provider>(guard: ResourceGuard<R>) -> Result<(), Error> {
    ///     match guard.release().await? {
    ///         ReleaseOutcome::Completed => {},
    ///         ReleaseOutcome::Deferred => {
    ///             // The cleanup queue owns the work; do not retry it.
    ///         },
    ///         _ => {}, // `ReleaseOutcome` may gain variants in future releases.
    ///     }
    ///     Ok(())
    /// }
    /// ```
    pub async fn release(mut self) -> Result<ReleaseOutcome, crate::Error> {
        match self.enqueue_release() {
            Some(Ok(submission)) => submission.wait().await.map(|outcome| match outcome {
                crate::release_queue::SubmissionOutcome::Completed => ReleaseOutcome::Completed,
                crate::release_queue::SubmissionOutcome::Deferred => ReleaseOutcome::Deferred,
            }),
            Some(Err(error)) => Err(error),
            None => Ok(ReleaseOutcome::Completed),
        }
    }

    fn enqueue_release(&mut self) -> Option<Result<ReleaseSubmission, crate::Error>> {
        let entry = self.entry.take()?;
        self.hold_watchdog.take();
        let metrics = self.metrics.take();
        let settlement = ReleaseSettlement {
            permit: self.permit.take(),
            drain_counters: self.drain_counters.take(),
            event_bus: self.event_bus.take(),
            metrics: metrics.clone(),
            key: self.resource_key.clone(),
            held: self.acquired_at.elapsed(),
            tainted: self.tainted,
            has_completed: false,
        };
        let managed = Arc::clone(&self.managed);
        let checkout_epoch = self.checkout_epoch;
        let tainted = self.tainted;
        Some(self.managed.release_queue.submit_release(move || {
            Box::pin(async move {
                if let Some(metrics) = &metrics {
                    metrics.record_release();
                }
                let outcome = release_entry(managed, entry, checkout_epoch, tainted, metrics).await;
                settlement.complete(outcome.is_err());
                outcome
            })
        }))
    }
}

/// Owns reservation settlement even when a queued factory or future is never
/// polled, while emitting `Released` only after cleanup actually returns.
struct ReleaseSettlement {
    permit: Option<OwnedSemaphorePermit>,
    drain_counters: Option<DrainTrackers>,
    event_bus: Option<Arc<EventBus<ResourceEvent>>>,
    metrics: Option<ResourceOpsMetrics>,
    key: ResourceKey,
    held: Duration,
    tainted: bool,
    has_completed: bool,
}

impl ReleaseSettlement {
    fn complete(mut self, has_error: bool) {
        self.has_completed = true;
        if has_error && let Some(metrics) = &self.metrics {
            metrics.record_release_error();
        }
    }
}

impl Drop for ReleaseSettlement {
    fn drop(&mut self) {
        if !self.has_completed {
            if let Some(metrics) = &self.metrics {
                metrics.record_release_error();
            }
            tracing::warn!(
                resource.key = %self.key,
                "resource release cleanup was abandoned before completion"
            );
        }
        self.permit.take();
        settle(
            self.drain_counters.take(),
            self.event_bus.take(),
            &self.key,
            self.held,
            self.has_completed,
            self.tainted,
        );
    }
}

/// Post-callback settle shared **byte-for-byte** by `Drop` and
/// [`ResourceGuard::release`].
///
/// Decrements BOTH drain trackers (manager-wide + per-resource) with
/// `Release` ordering, waking the owning `Notify` on each `1 → 0` edge, then
/// emits [`ResourceEvent::Released`] iff `emit_released && event_bus.is_some()`.
/// The ordering matches the historical `Drop`: drain decrement first, event
/// second, so observers see `Released` in the same order as the underlying
/// recycle/destroy effect.
fn settle(
    drain_counters: Option<DrainTrackers>,
    event_bus: Option<Arc<EventBus<ResourceEvent>>>,
    key: &ResourceKey,
    held: Duration,
    emit_released: bool,
    tainted: bool,
) {
    // Drain tracking: decrement BOTH the manager-wide and per-resource
    // active counts, waking each owning `Notify` on its 1 → 0 edge. The
    // manager-wide tracker unblocks `graceful_shutdown`; the per-resource
    // tracker unblocks `revoke_slot`'s isolated per-resource drain.
    if let Some((manager, per_resource)) = drain_counters {
        for tracker in [&manager, &per_resource] {
            if tracker.0.fetch_sub(1, AtomicOrdering::Release) == 1 {
                tracker.1.notify_waiters();
            }
        }
    }

    // Best-effort `Released` event — emitted after the drain decrement so
    // observers see it in recycle/destroy order. `PublishOutcome` is
    // intentionally discarded (no subscribers is the expected normal case).
    if emit_released && let Some(bus) = event_bus {
        let _ = bus.emit(ResourceEvent::Released {
            key: key.clone(),
            held,
            tainted,
        });
    }
}

impl<R: Provider> Deref for ResourceGuard<R> {
    type Target = R::Instance;

    fn deref(&self) -> &Self::Target {
        match &self.entry {
            Some(entry) => self.managed.topology.entry_instance(entry),
            // Only consuming release and Drop take the entry; neither exposes
            // the consumed guard for dereferencing.
            None => unreachable!("only consumed resource guards have no topology entry"),
        }
    }
}

impl<R: Provider> Drop for ResourceGuard<R> {
    fn drop(&mut self) {
        match self.enqueue_release() {
            Some(Ok(submission)) => submission.detach(),
            Some(Err(error)) => tracing::warn!(
                error.kind = ?error.kind(),
                resource.key = %self.resource_key,
                "resource guard release submission was rejected; cleanup was abandoned"
            ),
            None => {},
        }
    }
}

impl<R: Provider> std::fmt::Debug for ResourceGuard<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResourceGuard")
            .field("resource_key", &self.resource_key)
            .field("topology_tag", &self.topology_tag)
            .field("generation", &self.generation)
            .field("tainted", &self.tainted)
            .finish_non_exhaustive()
    }
}

// ---------------------------------------------------------------------------
// Guard trait implementations (nebula_core::Guard / TypedGuard)
// ---------------------------------------------------------------------------

impl<R: Provider> nebula_core::Guard for ResourceGuard<R> {
    fn guard_kind(&self) -> &'static str {
        "resource"
    }

    fn acquired_at(&self) -> Instant {
        self.acquired_at
    }
}

impl<R: Provider> nebula_core::TypedGuard for ResourceGuard<R> {
    type Inner = R::Instance;

    fn as_inner(&self) -> &Self::Inner {
        self
    }
}

#[cfg(test)]
#[path = "guard_tests.rs"]
mod tests;
