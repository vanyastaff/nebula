//! Resource guard — the value callers hold while using a resource.
//!
//! [`ResourceGuard`] wraps a lease in one of three ownership modes:
//!
//! - **Owned**: caller owns the lease outright (no pool return).
//! - **Guarded**: exclusive lease returned to pool on drop.
//! - **Shared**: `Arc`-wrapped lease with shared access.

use std::{
    ops::Deref,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering as AtomicOrdering},
    },
    time::Instant,
};

use nebula_core::ResourceKey;
use tokio::sync::{Notify, OwnedSemaphorePermit};

use crate::{resource::Resource, topology_tag::TopologyTag};

/// Callback invoked when a guarded lease is released.
type GuardedRelease<R> = Box<dyn FnOnce(<R as Resource>::Lease, bool) + Send + Sync>;

/// A drain tracker: an in-flight `(active_count, waiters)` pair. One is the
/// manager-wide `graceful_shutdown` tracker; another is each
/// `ManagedResource`'s own counter that `Manager::revoke_slot` drains in
/// isolation (ADR-0067 §Deferred).
pub(crate) type DrainTracker = Arc<(AtomicU64, Notify)>;

/// The `(manager_wide, per_resource)` pair an acquire pre-increments and
/// hands to its [`ResourceGuard`]. Both are decremented + notified on guard
/// drop: the first unblocks `graceful_shutdown`, the second unblocks the
/// originating resource's isolated revoke drain.
pub(crate) type DrainTrackers = (DrainTracker, DrainTracker);

/// A guard over a leased resource.
///
/// Dereferences to `R::Lease` for ergonomic access. On drop, guarded and
/// shared guards notify the pool (or release callback) so the lease can
/// be recycled or destroyed.
#[must_use = "dropping a ResourceGuard immediately releases the resource"]
pub struct ResourceGuard<R: Resource> {
    inner: GuardInner<R>,
    resource_key: ResourceKey,
    topology_tag: TopologyTag,
    /// When this guard was acquired — used for lifetime tracking and the `Guard` trait.
    acquired_at: Instant,
    /// Optional manager-wide + per-resource drain trackers — each
    /// decremented on drop, the owning `Notify` woken when it hits zero.
    ///
    /// The first element is `Manager::drain_tracker` (`graceful_shutdown`
    /// drain); the second is the originating `ManagedResource`'s own
    /// in-flight tracker, which `Manager::revoke_slot` drains in isolation
    /// (ADR-0067 §Deferred). Both are pre-incremented by `InFlightCounter`
    /// and handed off here, so a guard handed out for a row reflects in that
    /// row's revoke drain — closing the revoke-vs-acquire TOCTOU
    /// (ADR-0044/0036).
    drain_counters: Option<DrainTrackers>,
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
        on_release: Option<Box<dyn FnOnce(bool) + Send + Sync>>,
        tainted: bool,
        generation: u64,
    },
}

impl<R: Resource> ResourceGuard<R> {
    /// Creates an owned guard — no pool, no release callback.
    pub fn owned(lease: R::Lease, resource_key: ResourceKey, topology_tag: TopologyTag) -> Self {
        Self {
            inner: GuardInner::Owned(lease),
            resource_key,
            topology_tag,
            acquired_at: Instant::now(),
            drain_counters: None,
        }
    }

    /// Creates a guarded guard — exclusive lease returned via callback on drop.
    pub fn guarded(
        lease: R::Lease,
        resource_key: ResourceKey,
        topology_tag: TopologyTag,
        generation: u64,
        on_release: impl FnOnce(R::Lease, bool) + Send + Sync + 'static,
    ) -> Self {
        Self::guarded_with_permit(
            lease,
            resource_key,
            topology_tag,
            generation,
            on_release,
            None,
        )
    }

    /// Creates a guarded guard with an optional semaphore permit.
    ///
    /// The permit is held as a separate field so that it is returned to the
    /// semaphore even if the release callback panics (caught by `catch_unwind`
    /// in the `Drop` impl). Without this, a panic in the callback would
    /// destroy the permit along with the unwound closure, permanently leaking
    /// a semaphore slot.
    pub fn guarded_with_permit(
        lease: R::Lease,
        resource_key: ResourceKey,
        topology_tag: TopologyTag,
        generation: u64,
        on_release: impl FnOnce(R::Lease, bool) + Send + Sync + 'static,
        permit: Option<OwnedSemaphorePermit>,
    ) -> Self {
        Self {
            inner: GuardInner::Guarded {
                value: Some(lease),
                on_release: Some(Box::new(on_release)),
                permit,
                tainted: false,
                generation,
            },
            resource_key,
            topology_tag,
            acquired_at: Instant::now(),
            drain_counters: None,
        }
    }

    /// Creates a shared guard — `Arc`-wrapped lease with ref-count tracking.
    pub fn shared(
        lease: Arc<R::Lease>,
        resource_key: ResourceKey,
        topology_tag: TopologyTag,
        generation: u64,
        on_release: impl FnOnce(bool) + Send + Sync + 'static,
    ) -> Self {
        Self {
            inner: GuardInner::Shared {
                value: lease,
                on_release: Some(Box::new(on_release)),
                tainted: false,
                generation,
            },
            resource_key,
            topology_tag,
            acquired_at: Instant::now(),
            drain_counters: None,
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
    /// This caller-owned ordering closes two races. (1) The
    /// `graceful_shutdown` race: an acquire that passed `lookup()` before
    /// `cancel.cancel()` could otherwise complete *after* `wait_for_drain()`
    /// observed `0` and the registry was cleared. (2) The revoke-vs-acquire
    /// TOCTOU (ADR-0044/0036): because the per-resource counter is
    /// incremented before the post-taint re-check and decremented only when
    /// this guard drops, `revoke_slot`'s per-resource drain (ADR-0067
    /// §Deferred) cannot complete while a guard handed out for that row is
    /// still live.
    pub(crate) fn with_drain_tracker(mut self, trackers: DrainTrackers) -> Self {
        self.drain_counters = Some(trackers);
        self
    }

    /// Marks the lease as tainted — it will be destroyed instead of recycled.
    pub fn taint(&mut self) {
        match &mut self.inner {
            GuardInner::Owned(_) => {}, // no-op for owned
            GuardInner::Guarded { tainted, .. } | GuardInner::Shared { tainted, .. } => {
                *tainted = true;
            },
        }
    }

    /// Detaches the lease from pool management, converting to owned.
    ///
    /// Returns `Some(lease)` for owned and guarded guards, `None` for shared
    /// (since the `Arc` may have other holders).
    pub fn detach(mut self) -> Option<R::Lease> {
        match &mut self.inner {
            GuardInner::Owned(_) => {
                // Move out via replacement — we'll forget self afterward.
                let inner = std::mem::replace(
                    &mut self.inner,
                    // Dummy: immediately replaced, never accessed.
                    GuardInner::Guarded {
                        value: None,
                        on_release: None,
                        permit: None,
                        tainted: true,
                        generation: 0,
                    },
                );
                match inner {
                    GuardInner::Owned(lease) => Some(lease),
                    _ => unreachable!(),
                }
            },
            GuardInner::Guarded { .. } => {
                let inner = std::mem::replace(
                    &mut self.inner,
                    GuardInner::Guarded {
                        value: None,
                        on_release: None,
                        permit: None,
                        tainted: true,
                        generation: 0,
                    },
                );
                match inner {
                    GuardInner::Guarded {
                        value: Some(lease), ..
                    } => Some(lease),
                    _ => None,
                }
            },
            GuardInner::Shared { .. } => None,
        }
    }

    /// Returns how long this guard has been held.
    pub fn hold_duration(&self) -> std::time::Duration {
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
            GuardInner::Owned(_) => None,
            GuardInner::Guarded { generation, .. } | GuardInner::Shared { generation, .. } => {
                Some(*generation)
            },
        }
    }
}

impl<R: Resource> Deref for ResourceGuard<R> {
    type Target = R::Lease;

    fn deref(&self) -> &Self::Target {
        match &self.inner {
            GuardInner::Owned(lease) => lease,
            GuardInner::Guarded {
                value: Some(lease), ..
            } => lease,
            GuardInner::Guarded { value: None, .. } => {
                panic!("ResourceGuard accessed after detach")
            },
            GuardInner::Shared { value, .. } => value,
        }
    }
}

impl<R: Resource> Drop for ResourceGuard<R> {
    fn drop(&mut self) {
        match &mut self.inner {
            GuardInner::Owned(_) => {}, // nothing to do
            GuardInner::Guarded {
                value,
                on_release,
                permit,
                tainted,
                ..
            } => {
                // Take the permit out BEFORE the callback runs. It will be
                // dropped at the end of this scope — after catch_unwind —
                // ensuring the semaphore slot is returned even if the
                // callback panics.
                let _permit_guard = permit.take();

                if let (Some(lease), Some(callback)) = (value.take(), on_release.take()) {
                    // catch_unwind prevents double-panic abort if callback panics.
                    let tainted = *tainted;
                    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        callback(lease, tainted);
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
            GuardInner::Shared {
                on_release,
                tainted,
                ..
            } => {
                if let Some(callback) = on_release.take() {
                    let tainted = *tainted;
                    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        callback(tainted);
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

        // Drain tracking: decrement BOTH the manager-wide and per-resource
        // active counts, waking each owning `Notify` on its 1 → 0 edge. The
        // manager-wide tracker unblocks `graceful_shutdown`; the per-resource
        // tracker unblocks `revoke_slot`'s isolated per-resource drain
        // (ADR-0067 §Deferred).
        if let Some((ref manager, ref per_resource)) = self.drain_counters {
            for tracker in [manager, per_resource] {
                if tracker.0.fetch_sub(1, AtomicOrdering::Release) == 1 {
                    tracker.1.notify_waiters();
                }
            }
        }
    }
}

impl<R: Resource> std::fmt::Debug for ResourceGuard<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mode = match &self.inner {
            GuardInner::Owned(_) => "Owned",
            GuardInner::Guarded { .. } => "Guarded",
            GuardInner::Shared { .. } => "Shared",
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

    #[test]
    fn owned_deref() {
        let handle = ResourceGuard::<DummyResource>::owned(42, test_key(), TopologyTag::Pool);
        assert_eq!(*handle, 42);
    }

    #[test]
    fn guarded_calls_release_on_drop() {
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
                },
            );
            assert!(!released.load(Ordering::Relaxed));
        }
        // After drop
        assert!(released.load(Ordering::Relaxed));
        assert_eq!(value.load(Ordering::Relaxed), 99);
    }

    #[test]
    fn shared_calls_release_on_drop() {
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
                },
            );
        }
        assert!(released.load(Ordering::Relaxed));
    }

    #[test]
    fn taint_marks_guarded() {
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
                },
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

    #[test]
    fn detach_guarded_returns_lease_and_skips_callback() {
        let released = Arc::new(AtomicBool::new(false));
        let released_clone = released;

        let handle = ResourceGuard::<DummyResource>::guarded(
            10,
            test_key(),
            TopologyTag::Pool,
            1,
            move |_lease, _tainted| {
                released_clone.store(true, Ordering::Relaxed);
            },
        );
        let lease = handle.detach();
        assert_eq!(lease, Some(10));
        // Callback should NOT have fired (the dummy drop handles None gracefully)
    }

    #[test]
    fn detach_shared_returns_none() {
        let handle = ResourceGuard::<DummyResource>::shared(
            Arc::new(5),
            test_key(),
            TopologyTag::Resident,
            1,
            |_| {},
        );
        let lease = handle.detach();
        assert_eq!(lease, None);
    }

    #[test]
    fn hold_duration_is_zero_for_owned() {
        let handle = ResourceGuard::<DummyResource>::owned(1, test_key(), TopologyTag::Pool);
        // Owned guards now also track acquired_at, so hold_duration may be
        // very small but not necessarily ZERO.  Just assert it is tiny.
        assert!(handle.hold_duration() < std::time::Duration::from_millis(100));
    }

    #[test]
    fn resource_key_and_topology_tag() {
        let key = test_key();
        let handle = ResourceGuard::<DummyResource>::owned(1, key.clone(), TopologyTag::Pool);
        assert_eq!(*handle.resource_key(), key);
        assert_eq!(handle.topology_tag(), TopologyTag::Pool);
    }

    #[test]
    fn taint_on_shared_handle_is_seen_by_callback() {
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
                },
            );
            handle.taint();
        }

        assert!(
            was_tainted.load(Ordering::Relaxed),
            "taint() on Shared guard should be visible in release callback"
        );
    }

    #[test]
    fn detach_guarded_does_not_fire_callback() {
        let released = Arc::new(AtomicBool::new(false));
        let r = released.clone();

        let handle = ResourceGuard::<DummyResource>::guarded(
            10,
            test_key(),
            TopologyTag::Pool,
            1,
            move |_lease, _tainted| {
                r.store(true, Ordering::Relaxed);
            },
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
        assert!(handle.acquired_at().elapsed() < std::time::Duration::from_secs(1));
    }

    #[test]
    fn resource_guard_implements_typed_guard_trait() {
        use nebula_core::TypedGuard;
        let handle = ResourceGuard::<DummyResource>::owned(42, test_key(), TopologyTag::Pool);
        assert_eq!(*handle.as_inner(), 42);
    }
}
