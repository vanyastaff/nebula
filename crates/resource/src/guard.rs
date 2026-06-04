//! Resource guard — the value callers hold while using a resource.
//!
//! [`ResourceGuard`] wraps a lease in one of three ownership modes:
//!
//! - **Owned**: caller owns the lease outright (no pool return).
//! - **Guarded**: exclusive lease returned to pool on drop.
//! - **Shared**: `Arc`-wrapped lease with shared access.

use std::{
    future::Future,
    ops::Deref,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering as AtomicOrdering},
    },
    time::{Duration, Instant},
};

use futures::FutureExt as _;
use nebula_core::ResourceKey;
use nebula_eventbus::EventBus;
use tokio::sync::{Notify, OwnedSemaphorePermit};

use crate::{
    events::ResourceEvent, release_queue::ReleaseQueue, resource::Resource,
    topology_tag::TopologyTag,
};

/// The awaited teardown future a release callback produces.
///
/// Returning the future (rather than submitting it to a queue) lets the
/// caller-facing [`ResourceGuard::release`] checkpoint `await` it and
/// observe the recycle/destroy/reset `Result`. The `Drop` fallback submits
/// the very same future to the [`ReleaseQueue`], discarding its `Result`,
/// so the queued path stays best-effort.
type ReleaseFuture = Pin<Box<dyn Future<Output = Result<(), crate::Error>> + Send>>;

/// Callback invoked when a guarded lease is released.
///
/// It does not perform the teardown itself; it *builds* the teardown future
/// from the returned lease + tainted flag. The guard then either awaits that
/// future inline ([`ResourceGuard::release`]) or submits it to the
/// [`ReleaseQueue`] (`Drop`).
type GuardedRelease<R> =
    Box<dyn FnOnce(<R as Resource>::Lease, bool) -> ReleaseFuture + Send + Sync>;

/// Callback invoked when a shared (`Arc`-wrapped) lease is released.
///
/// Like [`GuardedRelease`] it returns the teardown future rather than
/// running it, so both the awaited and queued release paths share one body.
type SharedRelease = Box<dyn FnOnce(bool) -> ReleaseFuture + Send + Sync>;

/// A drain tracker: an in-flight `(active_count, waiters)` pair. One is the
/// manager-wide `graceful_shutdown` tracker; another is each
/// `ManagedResource`'s own counter that `Manager::revoke_slot` drains in
/// isolation. See the [`manager`](crate::manager) module docs for the
/// canonical two-phase-revoke / drain invariant.
pub(crate) type DrainTracker = Arc<(AtomicU64, Notify)>;

/// The `(manager_wide, per_resource)` pair an acquire pre-increments and
/// hands to its [`ResourceGuard`]. Both are decremented + notified on guard
/// drop: the first unblocks `graceful_shutdown`, the second unblocks the
/// originating resource's isolated revoke drain.
pub(crate) type DrainTrackers = (DrainTracker, DrainTracker);

/// A guard over a leased resource.
///
/// Dereferences to [`R::Lease`](Resource::Lease) for ergonomic access. The
/// guard holds the in-flight reservation: dropping it returns the lease
/// to its owning topology (recycle / destroy / Arc decrement, per
/// topology).
///
/// # Drop
///
/// Drop is the **release pathway** and runs synchronously to:
///
/// 1. Decrement the manager-wide drain tracker (unblocks
///    `Manager::graceful_shutdown` once it hits zero).
/// 2. Decrement the per-resource in-flight counter (unblocks
///    `Manager::revoke_slot` draining this row).
/// 3. Hand the lease back to its owning topology runtime:
///    - **Pooled** — `Pooled::recycle` is awaited; on `Keep` the
///      instance returns to the idle queue, on `Drop` it queues a
///      destroy on the release queue.
///    - **Resident** — the `Arc` strong-count is decremented; no
///      per-acquire release work.
///    - **Bounded** — the semaphore permit is released; if
///      `BoundedRelease` is implemented, its reset is queued on the
///      release queue.
/// 4. Emit
///    [`ResourceEvent::Released { held, tainted }`](crate::events::ResourceEvent::Released).
///
/// Call [`ResourceGuard::taint`] **before** drop to skip recycle and
/// force destroy on a misbehaving lease.
///
/// # Cancellation
///
/// Drop runs in any cancellation context, including a cancelled
/// `tokio::task`. The drop path itself contains no `.await`; any async
/// work (destroy, `BoundedRelease::reset`) is pushed onto the release
/// queue which survives task cancellation. **Async release is
/// best-effort on crash** — see canon §11.4.
///
/// # Panics
///
/// Drop does not panic. If a release callback the topology runtime
/// installed panics, the panic is caught and logged via `tracing`;
/// drain counters are still decremented so shutdown cannot deadlock.
#[must_use = "dropping a ResourceGuard immediately releases the resource"]
pub struct ResourceGuard<R: Resource> {
    /// The live lease state. `Some` for the entire lifetime of a usable
    /// guard; only [`ResourceGuard::detach`] sets it to `None`, and `detach`
    /// consumes `self` by value — so a detached guard is not nameable and
    /// `Deref`/`Drop` after detach are unrepresentable rather than guarded by
    /// a runtime sentinel.
    inner: Option<GuardInner<R>>,
    resource_key: ResourceKey,
    topology_tag: TopologyTag,
    /// When this guard was acquired — used for lifetime tracking and the `Guard` trait.
    acquired_at: Instant,
    /// Optional manager-wide + per-resource drain trackers — each
    /// decremented on drop, the owning `Notify` woken when it hits zero.
    ///
    /// The first element is `Manager::drain_tracker` (`graceful_shutdown`
    /// drain); the second is the originating `ManagedResource`'s own
    /// in-flight tracker, which `Manager::revoke_slot` drains in isolation.
    /// Both are pre-incremented by `InFlightCounter` and handed off here, so
    /// a guard handed out for a row stays reflected in that row's revoke
    /// drain until it drops — part of the revoke-vs-acquire TOCTOU close.
    /// See the [`manager`](crate::manager) module docs for the canonical
    /// invariant.
    drain_counters: Option<DrainTrackers>,
    /// Optional manager event bus for emitting [`ResourceEvent::Released`]
    /// on drop. Attached by
    /// [`Manager::run_acquire`](crate::manager::Manager) right after the
    /// underlying topology runtime hands back the guard. `None` for
    /// guards minted outside the manager funnel (tests, fixtures, ad-hoc
    /// owned guards) — in that case the released event is silently
    /// skipped, matching the existing best-effort emit contract elsewhere
    /// in the crate.
    event_bus: Option<Arc<EventBus<ResourceEvent>>>,
    /// The owning topology's [`ReleaseQueue`], used **only** by the `Drop`
    /// fallback: `Drop` builds the teardown future via the release callback
    /// and submits it here (discarding its `Result`) so the queued path
    /// stays best-effort / fire-and-forget across task cancellation.
    /// `Some` for `Guarded` / `Shared` guards minted by the pool / bounded
    /// runtimes; `None` for `Owned` (resident) guards, which have no
    /// release work to queue. [`ResourceGuard::release`] does **not** touch
    /// this — it awaits the future inline instead of queueing it.
    release_queue: Option<Arc<ReleaseQueue>>,
}

enum GuardInner<R: Resource> {
    Owned(R::Lease),
    Guarded {
        value: Option<R::Lease>,
        on_release: Option<GuardedRelease<R>>,
        permit: Option<OwnedSemaphorePermit>,
        tainted: bool,
        generation: u64,
    },
    Shared {
        value: Arc<R::Lease>,
        on_release: Option<SharedRelease>,
        tainted: bool,
        generation: u64,
    },
}

impl<R: Resource> ResourceGuard<R> {
    /// Creates an owned guard — no pool, no release callback.
    pub fn owned(lease: R::Lease, resource_key: ResourceKey, topology_tag: TopologyTag) -> Self {
        Self {
            inner: Some(GuardInner::Owned(lease)),
            resource_key,
            topology_tag,
            acquired_at: Instant::now(),
            drain_counters: None,
            event_bus: None,
            release_queue: None,
        }
    }

    /// Creates a guarded guard — exclusive lease returned via callback on drop.
    ///
    /// `on_release` does not run the teardown directly; it **builds** the
    /// teardown future (recycle / destroy) from the returned lease + tainted
    /// flag. [`release`](Self::release) awaits that future inline and returns
    /// its `Result`; `Drop` submits it to `release_queue` (discarding the
    /// `Result`) as the best-effort fallback.
    pub fn guarded(
        lease: R::Lease,
        resource_key: ResourceKey,
        topology_tag: TopologyTag,
        generation: u64,
        on_release: impl FnOnce(R::Lease, bool) -> ReleaseFuture + Send + Sync + 'static,
        release_queue: Arc<ReleaseQueue>,
    ) -> Self {
        Self::guarded_with_permit(
            lease,
            resource_key,
            topology_tag,
            generation,
            on_release,
            None,
            release_queue,
        )
    }

    /// Creates a guarded guard with an optional semaphore permit.
    ///
    /// The permit is held as a separate field so that it is returned to the
    /// semaphore even if the release callback panics (caught by `catch_unwind`
    /// in the `Drop` impl). Without this, a panic in the callback would
    /// destroy the permit along with the unwound closure, permanently leaking
    /// a semaphore slot.
    ///
    /// `on_release` returns the teardown future rather than running it; see
    /// [`guarded`](Self::guarded). `release_queue` is the queue the `Drop`
    /// fallback submits that future to.
    pub fn guarded_with_permit(
        lease: R::Lease,
        resource_key: ResourceKey,
        topology_tag: TopologyTag,
        generation: u64,
        on_release: impl FnOnce(R::Lease, bool) -> ReleaseFuture + Send + Sync + 'static,
        permit: Option<OwnedSemaphorePermit>,
        release_queue: Arc<ReleaseQueue>,
    ) -> Self {
        Self {
            inner: Some(GuardInner::Guarded {
                value: Some(lease),
                on_release: Some(Box::new(on_release)),
                permit,
                tainted: false,
                generation,
            }),
            resource_key,
            topology_tag,
            acquired_at: Instant::now(),
            drain_counters: None,
            event_bus: None,
            release_queue: Some(release_queue),
        }
    }

    /// Creates a shared guard — `Arc`-wrapped lease with ref-count tracking.
    ///
    /// `on_release` returns the teardown future rather than running it; see
    /// [`guarded`](Self::guarded). `release_queue` is the queue the `Drop`
    /// fallback submits that future to.
    pub fn shared(
        lease: Arc<R::Lease>,
        resource_key: ResourceKey,
        topology_tag: TopologyTag,
        generation: u64,
        on_release: impl FnOnce(bool) -> ReleaseFuture + Send + Sync + 'static,
        release_queue: Arc<ReleaseQueue>,
    ) -> Self {
        Self {
            inner: Some(GuardInner::Shared {
                value: lease,
                on_release: Some(Box::new(on_release)),
                tainted: false,
                generation,
            }),
            resource_key,
            topology_tag,
            acquired_at: Instant::now(),
            drain_counters: None,
            event_bus: None,
            release_queue: Some(release_queue),
        }
    }

    /// Attaches the manager-wide + per-resource drain trackers for shutdown
    /// and revoke coordination.
    ///
    /// **Caller-owned increment**: this method does NOT increment either
    /// counter. Callers (the `Manager` acquire paths) must pre-increment
    /// both before any `await` past `lookup()` (via `InFlightCounter`) and
    /// hand the *already-counted slots* off here. The guard then owns both
    /// and decrements + notifies each on Drop.
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

    /// Marks the lease as tainted — it will be destroyed instead of recycled.
    pub fn taint(&mut self) {
        match &mut self.inner {
            None | Some(GuardInner::Owned(_)) => {}, // no-op for owned / detached
            Some(GuardInner::Guarded { tainted, .. } | GuardInner::Shared { tainted, .. }) => {
                *tainted = true;
            },
        }
    }

    /// Detaches the lease from pool management, converting to owned.
    ///
    /// Returns `Some(lease)` for owned and guarded guards, `None` for shared
    /// (since the `Arc` may have other holders).
    pub fn detach(mut self) -> Option<R::Lease> {
        // `take()` moves the state out and leaves `None` behind. `self` is
        // then dropped here; its `Drop` impl sees `None` and runs no release
        // callback — identical to the old `mem::replace` sentinel, but the
        // post-detach state is now structurally absent (no dummy variant, no
        // dead match arm to assert away).
        match self.inner.take() {
            Some(GuardInner::Owned(lease)) => Some(lease),
            Some(GuardInner::Guarded {
                value: Some(lease), ..
            }) => Some(lease),
            // Shared (`Arc` may have other holders) and the post-detach
            // already-`None` / `Guarded { value: None }` shapes all map to
            // `None`, preserving the prior return mapping verbatim.
            Some(GuardInner::Guarded { value: None, .. } | GuardInner::Shared { .. }) | None => {
                None
            },
        }
    }

    /// Returns how long this guard has been held.
    pub fn hold_duration(&self) -> Duration {
        self.acquired_at.elapsed()
    }

    /// Returns the resource key for this guard.
    pub fn resource_key(&self) -> &ResourceKey {
        &self.resource_key
    }

    /// Returns the topology tag identifying which topology this guard came from.
    pub fn topology_tag(&self) -> TopologyTag {
        self.topology_tag
    }

    /// Returns the generation counter, if this is a pooled guard.
    pub fn generation(&self) -> Option<u64> {
        match &self.inner {
            None | Some(GuardInner::Owned(_)) => None,
            Some(
                GuardInner::Guarded { generation, .. } | GuardInner::Shared { generation, .. },
            ) => Some(*generation),
        }
    }

    /// Explicit, awaited release checkpoint (canon §11.4).
    ///
    /// Runs the **same** teardown as [`Drop`] — recycle / destroy / reset —
    /// but **inline and awaited**, returning the recycle/destroy/reset
    /// `Result` so a caller who cares observes the outcome instead of
    /// relying on the best-effort, fire-and-forget [`ReleaseQueue`] fallback
    /// that `Drop` uses. Canon §11.4: authors must not assume release ran
    /// without an explicit checkpoint.
    ///
    /// Consuming `self` makes the subsequent drop fully **inert**: the
    /// release state (`inner`), the drain counters, and the event bus are
    /// all taken out here, so dropping the husk runs no second callback, no
    /// second drain decrement, and emits no second `Released` event.
    ///
    /// # Errors
    ///
    /// Returns the recycle/destroy/reset error verbatim (pool
    /// `release_entry`, bounded `release_one`). **On `Err` the teardown
    /// still completed the drain accounting and the slot is still released**
    /// — only the recycle/destroy/reset *outcome* is surfaced. For the
    /// bounded `Exclusive` cap the failed reset has already latched the
    /// runtime poisoned and returned the permit before the error reaches
    /// here (S4 / #384), exactly as on the queued path.
    pub async fn release(mut self) -> Result<(), crate::Error> {
        // Take the post-callback settle inputs OUT of `self` so the drop of
        // the husk at end of scope is inert — no double settle, no double
        // callback, no double `Released`. `emit_released` mirrors `Drop`:
        // `inner.is_some()` (a detached `None` skips the event).
        let inner = self.inner.take();
        let drain_counters = self.drain_counters.take();
        let event_bus = self.event_bus.take();
        let held = self.acquired_at.elapsed();
        let key = self.resource_key.clone();

        let emit_released = inner.is_some();
        let tainted = match &inner {
            Some(GuardInner::Guarded { tainted, .. } | GuardInner::Shared { tainted, .. }) => {
                *tainted
            },
            None | Some(GuardInner::Owned(_)) => false,
        };

        match inner {
            None | Some(GuardInner::Owned(_)) => {
                // Owned (resident) / detached: no release work, but the
                // drain accounting + (for `Owned`) the `Released` event
                // still run — byte-for-byte the `Drop` Owned/None arm.
                settle(
                    drain_counters,
                    event_bus,
                    &key,
                    held,
                    emit_released,
                    tainted,
                );
                Ok(())
            },
            Some(GuardInner::Guarded {
                value,
                on_release,
                permit,
                ..
            }) => {
                // Build the teardown future, then run it + the drain settle on
                // a DETACHED task (see `spawn_teardown_and_settle`): a caller
                // that awaits `release()` inside a cancellable task must not,
                // by being aborted mid-teardown, drop the teardown future or
                // leave the slot permanently counted in the drain. The permit
                // is moved in and held until the teardown resolves (#384).
                let teardown: ReleaseFuture = match (value, on_release) {
                    (Some(lease), Some(callback)) => callback(lease, tainted),
                    _ => Box::pin(async { Ok(()) }),
                };
                spawn_teardown_and_settle(
                    teardown,
                    permit,
                    drain_counters,
                    event_bus,
                    key,
                    held,
                    emit_released,
                    tainted,
                )
                .await
            },
            Some(GuardInner::Shared { on_release, .. }) => {
                let teardown: ReleaseFuture = match on_release {
                    Some(callback) => callback(tainted),
                    None => Box::pin(async { Ok(()) }),
                };
                spawn_teardown_and_settle(
                    teardown,
                    None,
                    drain_counters,
                    event_bus,
                    key,
                    held,
                    emit_released,
                    tainted,
                )
                .await
            },
        }
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

/// Runs a guard teardown future + the post-teardown [`settle`] on a **detached
/// task**, so [`ResourceGuard::release`] is cancel-safe.
///
/// Dropping a `JoinHandle` does not abort its task: if the caller that awaited
/// `release()` is cancelled mid-teardown, the await is abandoned but this task
/// still runs the teardown to completion and decrements the drain counters —
/// matching the `Drop` fallback, which offloads the same work to the
/// [`ReleaseQueue`]. Without this, a cancellation after the guard's state was
/// taken out would drop the teardown (leaking the pooled runtime) and skip the
/// drain settle (wedging `graceful_shutdown` / `revoke_slot`).
///
/// The teardown is wrapped in `catch_unwind` so a panicking resource teardown
/// still settles the drain (the panic is surfaced as a typed error), mirroring
/// `Drop`'s `catch_unwind`. The semaphore permit is moved in and dropped only
/// after the teardown resolves (#384).
#[expect(
    clippy::too_many_arguments,
    reason = "teardown + permit + the six `settle` inputs; bundling into a one-use struct adds more ceremony than it removes for this internal helper"
)]
async fn spawn_teardown_and_settle(
    teardown: ReleaseFuture,
    permit: Option<OwnedSemaphorePermit>,
    drain_counters: Option<DrainTrackers>,
    event_bus: Option<Arc<EventBus<ResourceEvent>>>,
    key: ResourceKey,
    held: Duration,
    emit_released: bool,
    tainted: bool,
) -> Result<(), crate::Error> {
    let task = tokio::spawn(async move {
        let outcome = match std::panic::AssertUnwindSafe(teardown).catch_unwind().await {
            Ok(res) => res,
            Err(_panic) => Err(crate::Error::transient(
                "resource teardown panicked during release()",
            )),
        };
        // #384: the permit outlives the teardown (a bounded `Exclusive` reset
        // must complete before the slot frees), then drops here.
        drop(permit);
        // Drain accounting ALWAYS runs — even on teardown error/panic, and
        // even if the caller that awaited `release()` was cancelled (this task
        // is detached from that cancellation).
        settle(
            drain_counters,
            event_bus,
            &key,
            held,
            emit_released,
            tainted,
        );
        outcome
    });
    match task.await {
        Ok(outcome) => outcome,
        // The detached task was aborted (e.g. runtime shutdown). The process
        // is tearing down; report it rather than panic on the `JoinError`.
        Err(_join_err) => Err(crate::Error::cancelled()),
    }
}

impl<R: Resource> Deref for ResourceGuard<R> {
    type Target = R::Lease;

    fn deref(&self) -> &Self::Target {
        match &self.inner {
            Some(GuardInner::Owned(lease)) => lease,
            Some(GuardInner::Guarded {
                value: Some(lease), ..
            }) => lease,
            Some(GuardInner::Shared { value, .. }) => value,
            // `None` and `Guarded { value: None }` are only produced by
            // `detach`, which consumes `self` by value — so a detached guard
            // cannot be named, let alone dereferenced. This arm exists solely
            // to satisfy the total `Deref` signature for a state that is
            // structurally impossible to construct here. The former runtime
            // accessed-after-detach abort is now a compile error by
            // construction rather than a discipline check.
            // guard-justified: total Deref fn forces one arm for the
            // detach-only state, which cannot be reached (detach moves self).
            Some(GuardInner::Guarded { value: None, .. }) | None => unreachable!(),
        }
    }
}

impl<R: Resource> Drop for ResourceGuard<R> {
    fn drop(&mut self) {
        // Snapshot the released-event payload once up front: `held` is fixed
        // by now (the guard is being dropped), and `tainted` depends on the
        // inner variant. `None` (detached) and `Owned` carry no taint
        // concept — taint only applies to pool-returned (`Guarded`) and
        // ref-counted (`Shared`) modes. We emit the event below, *after*
        // the release callback has run, so `tainted` already reflects any
        // late `taint()` call the callback may have observed.
        let held = self.acquired_at.elapsed();
        let event_tainted = match &self.inner {
            Some(GuardInner::Guarded { tainted, .. } | GuardInner::Shared { tainted, .. }) => {
                *tainted
            },
            None | Some(GuardInner::Owned(_)) => false,
        };

        // `emit_released` mirrors the historical contract: emit iff `inner`
        // is `Some` (a detached `None` skips the event — the lease is now
        // caller-owned and emitting `Released` would be a false signal).
        // Sampled before the match so the post-callback `settle` sees the
        // same value; the variant is never replaced below (only its fields
        // are taken), so `inner.is_some()` is stable across this body.
        let emit_released = self.inner.is_some();

        // A detached guard left `inner` as `None`: nothing to release here
        // (the lease is now caller-owned). The drain-tracker decrement in
        // `settle` still runs unconditionally — identical to the old
        // sentinel path, where the dummy `Guarded { value: None,
        // on_release: None }` also ran no callback yet fell through to the
        // same drain accounting.
        match &mut self.inner {
            None | Some(GuardInner::Owned(_)) => {}, // nothing to do
            Some(GuardInner::Guarded {
                value,
                on_release,
                permit,
                tainted,
                ..
            }) => {
                // Take the permit out BEFORE the callback runs. It will be
                // dropped at the end of this scope — after catch_unwind —
                // ensuring the semaphore slot is returned even if building +
                // submitting the release future panics.
                let _permit_guard = permit.take();

                if let (Some(lease), Some(callback)) = (value.take(), on_release.take()) {
                    let tainted = *tainted;
                    let release_queue = self.release_queue.take();
                    // catch_unwind prevents a double-panic abort if BUILDING
                    // the release future (the callback) or submitting it
                    // panics. Unwind-safe: `lease` is *moved* into the
                    // closure and `self` retains no alias to it (the permit
                    // was already taken out above), so an unwind cannot leave
                    // shared guard state in a torn condition. The callback
                    // now only *builds* the teardown future; `Drop` submits
                    // it to the queue discarding its `Result`, keeping the
                    // queued path best-effort / error-swallowing exactly as
                    // before.
                    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let fut = callback(lease, tainted);
                        if let Some(rq) = release_queue {
                            rq.submit(move || {
                                Box::pin(async move {
                                    let _ = fut.await;
                                })
                            });
                        }
                    }))
                    .is_err()
                    {
                        tracing::error!(
                            key = %self.resource_key,
                            "release callback panicked in ResourceGuard Drop"
                        );
                    }
                }
                // _permit_guard drops here, returning the slot to the semaphore.
            },
            Some(GuardInner::Shared {
                on_release,
                tainted,
                ..
            }) => {
                if let Some(callback) = on_release.take() {
                    let tainted = *tainted;
                    let release_queue = self.release_queue.take();
                    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let fut = callback(tainted);
                        if let Some(rq) = release_queue {
                            rq.submit(move || {
                                Box::pin(async move {
                                    let _ = fut.await;
                                })
                            });
                        }
                    }))
                    .is_err()
                    {
                        tracing::error!(
                            key = %self.resource_key,
                            "release callback panicked in ResourceGuard Drop"
                        );
                    }
                }
            },
        }

        // Post-callback settle: the two-tracker drain decrement + the
        // best-effort `Released` emit, shared byte-for-byte with
        // `ResourceGuard::release`. `event_tainted` was snapshotted up front
        // (after any late `taint()`); the drain decrement runs
        // unconditionally and the emit is gated on `emit_released`.
        settle(
            self.drain_counters.take(),
            self.event_bus.take(),
            &self.resource_key,
            held,
            emit_released,
            event_tainted,
        );
    }
}

impl<R: Resource> std::fmt::Debug for ResourceGuard<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mode = match &self.inner {
            Some(GuardInner::Owned(_)) => "Owned",
            Some(GuardInner::Guarded { .. }) => "Guarded",
            Some(GuardInner::Shared { .. }) => "Shared",
            // Unreachable for any nameable guard (detach consumes `self`);
            // present only because `Debug` is total over the field.
            None => "Detached",
        };
        f.debug_struct("ResourceGuard")
            .field("resource_key", &self.resource_key)
            .field("topology_tag", &self.topology_tag)
            .field("mode", &mode)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Guard trait implementations (nebula_core::Guard / TypedGuard)
// ---------------------------------------------------------------------------

impl<R: Resource> nebula_core::Guard for ResourceGuard<R> {
    fn guard_kind(&self) -> &'static str {
        "resource"
    }

    fn acquired_at(&self) -> Instant {
        self.acquired_at
    }
}

impl<R: Resource> nebula_core::TypedGuard for ResourceGuard<R> {
    type Inner = R::Lease;

    fn as_inner(&self) -> &Self::Inner {
        self
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

    use super::*;

    // A trivial resource for testing.
    struct DummyResource;

    impl crate::resource::ResourceConfig for () {
        fn validate(&self) -> Result<(), crate::Error> {
            Ok(())
        }
    }

    impl From<std::convert::Infallible> for crate::Error {
        fn from(v: std::convert::Infallible) -> Self {
            match v {}
        }
    }

    impl Resource for DummyResource {
        type Config = ();
        type Runtime = ();
        type Lease = u32;
        type Error = std::convert::Infallible;
        fn key() -> ResourceKey {
            nebula_core::resource_key!("dummy")
        }

        async fn create(
            &self,
            _config: &(),
            _ctx: &crate::context::ResourceContext,
        ) -> Result<(), std::convert::Infallible> {
            Ok(())
        }
    }

    fn test_key() -> ResourceKey {
        nebula_core::resource_key!("test")
    }

    /// Builds a single-worker [`ReleaseQueue`] for guard tests. Requires an
    /// ambient Tokio runtime (the queue spawns its workers), so callers are
    /// `#[tokio::test]`. The handle is dropped here (detaching the workers);
    /// these tests assert build-time callback side-effects, not queued-future
    /// completion, so the workers never need to be awaited.
    fn test_rq() -> Arc<ReleaseQueue> {
        let (rq, _handle) = ReleaseQueue::new(1);
        Arc::new(rq)
    }

    #[test]
    fn owned_deref() {
        let handle = ResourceGuard::<DummyResource>::owned(42, test_key(), TopologyTag::Pool);
        assert_eq!(*handle, 42);
    }

    #[tokio::test]
    async fn guarded_calls_release_on_drop() {
        let released = Arc::new(AtomicBool::new(false));
        let released_clone = released.clone();
        let value = Arc::new(AtomicU32::new(0));
        let value_clone = value.clone();

        {
            let _handle = ResourceGuard::<DummyResource>::guarded(
                99,
                test_key(),
                TopologyTag::Pool,
                1,
                move |lease, tainted| {
                    value_clone.store(lease, Ordering::Relaxed);
                    released_clone.store(!tainted, Ordering::Relaxed);
                    Box::pin(async { Ok(()) })
                },
                test_rq(),
            );
            assert!(!released.load(Ordering::Relaxed));
        }
        // After drop
        assert!(released.load(Ordering::Relaxed));
        assert_eq!(value.load(Ordering::Relaxed), 99);
    }

    #[tokio::test]
    async fn shared_calls_release_on_drop() {
        let released = Arc::new(AtomicBool::new(false));
        let released_clone = released.clone();

        {
            let _handle = ResourceGuard::<DummyResource>::shared(
                Arc::new(77),
                test_key(),
                TopologyTag::Resident,
                1,
                move |_tainted| {
                    released_clone.store(true, Ordering::Relaxed);
                    Box::pin(async { Ok(()) })
                },
                test_rq(),
            );
        }
        assert!(released.load(Ordering::Relaxed));
    }

    #[tokio::test]
    async fn taint_marks_guarded() {
        let was_tainted = Arc::new(AtomicBool::new(false));
        let was_tainted_clone = was_tainted.clone();

        {
            let mut handle = ResourceGuard::<DummyResource>::guarded(
                1,
                test_key(),
                TopologyTag::Pool,
                1,
                move |_lease, tainted| {
                    was_tainted_clone.store(tainted, Ordering::Relaxed);
                    Box::pin(async { Ok(()) })
                },
                test_rq(),
            );
            handle.taint();
        }
        assert!(was_tainted.load(Ordering::Relaxed));
    }

    #[test]
    fn detach_owned_returns_lease() {
        let handle = ResourceGuard::<DummyResource>::owned(42, test_key(), TopologyTag::Pool);
        let lease = handle.detach();
        assert_eq!(lease, Some(42));
    }

    #[tokio::test]
    async fn detach_guarded_returns_lease_and_skips_callback() {
        let released = Arc::new(AtomicBool::new(false));
        let released_clone = released;

        let handle = ResourceGuard::<DummyResource>::guarded(
            10,
            test_key(),
            TopologyTag::Pool,
            1,
            move |_lease, _tainted| {
                released_clone.store(true, Ordering::Relaxed);
                Box::pin(async { Ok(()) })
            },
            test_rq(),
        );
        let lease = handle.detach();
        assert_eq!(lease, Some(10));
        // Callback should NOT have fired (the dummy drop handles None gracefully)
    }

    #[tokio::test]
    async fn detach_shared_returns_none() {
        let handle = ResourceGuard::<DummyResource>::shared(
            Arc::new(5),
            test_key(),
            TopologyTag::Resident,
            1,
            |_| Box::pin(async { Ok(()) }),
            test_rq(),
        );
        let lease = handle.detach();
        assert_eq!(lease, None);
    }

    #[tokio::test]
    async fn detach_guarded_returns_permit_to_semaphore() {
        use std::sync::Arc as StdArc;

        use tokio::sync::Semaphore;

        // Single-slot semaphore: detach drops `GuardInner::Guarded`
        // implicitly after extracting the lease, so the held
        // `OwnedSemaphorePermit` must be reclaimed without going through
        // the Drop-impl's explicit `permit.take()` branch. If a future
        // refactor leaks the permit, the post-detach acquire below fails.
        let sem = StdArc::new(Semaphore::new(1));
        assert_eq!(sem.available_permits(), 1);

        let permit = StdArc::clone(&sem)
            .try_acquire_owned()
            .expect("first permit is available");

        let handle = ResourceGuard::<DummyResource>::guarded_with_permit(
            21,
            test_key(),
            TopologyTag::Pool,
            1,
            |_lease, _tainted| Box::pin(async { Ok(()) }),
            Some(permit),
            test_rq(),
        );

        // While the guard holds the permit the bounded capacity is
        // exhausted: a second acquire must fail.
        assert_eq!(sem.available_permits(), 0);
        assert!(
            sem.try_acquire().is_err(),
            "semaphore must be exhausted while the guard holds the only permit"
        );

        // detach extracts the lease and discards the Guarded variant,
        // dropping the permit indirectly.
        let lease = handle.detach();
        assert_eq!(
            lease,
            Some(21),
            "detach must still return the guarded lease"
        );

        // The bounded/exclusive slot must be reclaimed: detach must not
        // leak capacity even though it bypasses the Drop permit branch.
        assert_eq!(
            sem.available_permits(),
            1,
            "detach must return the permit to the semaphore"
        );
        let reacquired = sem
            .try_acquire()
            .expect("permit must be reclaimable after detach");
        drop(reacquired);
    }

    #[test]
    fn hold_duration_is_zero_for_owned() {
        let handle = ResourceGuard::<DummyResource>::owned(1, test_key(), TopologyTag::Pool);
        // Owned guards now also track acquired_at, so hold_duration may be
        // very small but not necessarily ZERO.  Just assert it is tiny.
        assert!(handle.hold_duration() < Duration::from_millis(100));
    }

    #[test]
    fn resource_key_and_topology_tag() {
        let key = test_key();
        let handle = ResourceGuard::<DummyResource>::owned(1, key.clone(), TopologyTag::Pool);
        assert_eq!(*handle.resource_key(), key);
        assert_eq!(handle.topology_tag(), TopologyTag::Pool);
    }

    #[tokio::test]
    async fn taint_on_shared_handle_is_seen_by_callback() {
        let was_tainted = Arc::new(AtomicBool::new(false));
        let wt = was_tainted.clone();

        {
            let mut handle = ResourceGuard::<DummyResource>::shared(
                Arc::new(42),
                test_key(),
                TopologyTag::Resident,
                1,
                move |tainted| {
                    wt.store(tainted, Ordering::Relaxed);
                    Box::pin(async { Ok(()) })
                },
                test_rq(),
            );
            handle.taint();
        }

        assert!(
            was_tainted.load(Ordering::Relaxed),
            "taint() on Shared guard should be visible in release callback"
        );
    }

    #[tokio::test]
    async fn detach_guarded_does_not_fire_callback() {
        let released = Arc::new(AtomicBool::new(false));
        let r = released.clone();

        let handle = ResourceGuard::<DummyResource>::guarded(
            10,
            test_key(),
            TopologyTag::Pool,
            1,
            move |_lease, _tainted| {
                r.store(true, Ordering::Relaxed);
                Box::pin(async { Ok(()) })
            },
            test_rq(),
        );
        let lease = handle.detach();
        assert_eq!(lease, Some(10));
        assert!(
            !released.load(Ordering::Relaxed),
            "detach should skip the release callback"
        );
    }

    #[test]
    fn resource_guard_implements_guard_trait() {
        use nebula_core::Guard;
        let handle = ResourceGuard::<DummyResource>::owned(42, test_key(), TopologyTag::Pool);
        assert_eq!(handle.guard_kind(), "resource");
        // acquired_at should be very recent
        assert!(handle.acquired_at().elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn resource_guard_implements_typed_guard_trait() {
        use nebula_core::TypedGuard;
        let handle = ResourceGuard::<DummyResource>::owned(42, test_key(), TopologyTag::Pool);
        assert_eq!(*handle.as_inner(), 42);
    }

    // A lease whose `Drop` is observable, so we can prove detach does not
    // double-invoke it or leak it.
    struct DropProbe(Arc<AtomicU32>);

    impl Drop for DropProbe {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    struct DropProbeResource;

    impl Resource for DropProbeResource {
        type Config = ();
        type Runtime = ();
        type Lease = DropProbe;
        type Error = std::convert::Infallible;
        fn key() -> ResourceKey {
            nebula_core::resource_key!("dropprobe")
        }

        async fn create(
            &self,
            _config: &(),
            _ctx: &crate::context::ResourceContext,
        ) -> Result<(), std::convert::Infallible> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn detach_guarded_with_observable_drop_lease_does_not_double_drop_or_leak() {
        let drops = Arc::new(AtomicU32::new(0));
        let cb_fired = Arc::new(AtomicBool::new(false));
        let cb_fired_clone = cb_fired.clone();

        let lease = DropProbe(drops.clone());
        let handle = ResourceGuard::<DropProbeResource>::guarded(
            lease,
            test_key(),
            TopologyTag::Pool,
            1,
            move |_lease, _tainted| {
                // Would normally recycle the lease; detach must skip this so
                // the lease is handed to the caller, not also released here.
                cb_fired_clone.store(true, Ordering::Relaxed);
                Box::pin(async { Ok(()) })
            },
            test_rq(),
        );

        let detached = handle.detach().expect("guarded detach yields the lease");
        // Guard dropped during `detach`: the release callback must NOT have
        // run, and the lease must NOT have been dropped yet (it moved out).
        assert!(
            !cb_fired.load(Ordering::Relaxed),
            "detach must not fire the release callback"
        );
        assert_eq!(
            drops.load(Ordering::Relaxed),
            0,
            "lease must move to the caller, not be dropped by the guard"
        );

        drop(detached);
        assert_eq!(
            drops.load(Ordering::Relaxed),
            1,
            "the detached lease drops exactly once, when the caller drops it"
        );
        assert!(
            !cb_fired.load(Ordering::Relaxed),
            "the release callback must never fire after detach"
        );
    }

    #[tokio::test]
    async fn panicking_release_callback_still_returns_the_permit() {
        use std::sync::Arc as StdArc;

        use tokio::sync::Semaphore;

        // Single-slot semaphore: if the permit is destroyed with the
        // unwinding callback instead of being returned, the second acquire
        // below would block forever.
        let sem = StdArc::new(Semaphore::new(1));
        let permit = StdArc::clone(&sem)
            .try_acquire_owned()
            .expect("first permit is available");

        {
            let handle = ResourceGuard::<DummyResource>::guarded_with_permit(
                7,
                test_key(),
                TopologyTag::Pool,
                1,
                |_lease, _tainted| panic!("release callback panics on purpose"),
                Some(permit),
                test_rq(),
            );
            // Dropping `handle` runs the panicking callback (which builds the
            // release future) inside catch_unwind; the permit was taken out
            // *before* the callback, so it is returned to the semaphore even
            // though the build unwinds.
            drop(handle);
        }

        // The slot must be reclaimable: this would fail if the panicking
        // callback had taken the permit down with it.
        let reclaimed = sem
            .try_acquire()
            .expect("permit must be returned despite the callback panic");
        drop(reclaimed);
    }
}
