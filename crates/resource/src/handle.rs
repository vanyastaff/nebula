//! Resource handle — the value callers hold while using a resource.
//!
//! [`ResourceHandle`] wraps a lease in one of three ownership modes:
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
type GuardedRelease<R> = Box<dyn FnOnce(<R as Resource>::Lease, bool) + Send>;

/// A handle to a leased resource.
///
/// Dereferences to `R::Lease` for ergonomic access. On drop, guarded and
/// shared handles notify the pool (or release callback) so the lease can
/// be recycled or destroyed.
#[must_use = "dropping a ResourceHandle immediately releases the resource"]
pub struct ResourceHandle<R: Resource> {
    inner: HandleInner<R>,
    resource_key: ResourceKey,
    topology_tag: TopologyTag,
    /// Optional drain tracker — decrements on drop, notifies when zero.
    drain_counter: Option<Arc<(AtomicU64, Notify)>>,
}

enum HandleInner<R: Resource> {
    Owned(R::Lease),
    Guarded {
        value: Option<R::Lease>,
        on_release: Option<GuardedRelease<R>>,
        permit: Option<OwnedSemaphorePermit>,
        tainted: bool,
        acquired_at: Instant,
        generation: u64,
    },
    Shared {
        value: Arc<R::Lease>,
        on_release: Option<Box<dyn FnOnce(bool) + Send>>,
        tainted: bool,
        acquired_at: Instant,
        generation: u64,
    },
}

impl<R: Resource> ResourceHandle<R> {
    /// Creates an owned handle — no pool, no release callback.
    pub fn owned(lease: R::Lease, resource_key: ResourceKey, topology_tag: TopologyTag) -> Self {
        Self {
            inner: HandleInner::Owned(lease),
            resource_key,
            topology_tag,
            drain_counter: None,
        }
    }

    /// Creates a guarded handle — exclusive lease returned via callback on drop.
    pub fn guarded(
        lease: R::Lease,
        resource_key: ResourceKey,
        topology_tag: TopologyTag,
        generation: u64,
        on_release: impl FnOnce(R::Lease, bool) + Send + 'static,
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

    /// Creates a guarded handle with an optional semaphore permit.
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
        on_release: impl FnOnce(R::Lease, bool) + Send + 'static,
        permit: Option<OwnedSemaphorePermit>,
    ) -> Self {
        Self {
            inner: HandleInner::Guarded {
                value: Some(lease),
                on_release: Some(Box::new(on_release)),
                permit,
                tainted: false,
                acquired_at: Instant::now(),
                generation,
            },
            resource_key,
            topology_tag,
            drain_counter: None,
        }
    }

    /// Creates a shared handle — `Arc`-wrapped lease with ref-count tracking.
    pub fn shared(
        lease: Arc<R::Lease>,
        resource_key: ResourceKey,
        topology_tag: TopologyTag,
        generation: u64,
        on_release: impl FnOnce(bool) + Send + 'static,
    ) -> Self {
        Self {
            inner: HandleInner::Shared {
                value: lease,
                on_release: Some(Box::new(on_release)),
                tainted: false,
                acquired_at: Instant::now(),
                generation,
            },
            resource_key,
            topology_tag,
            drain_counter: None,
        }
    }

    /// Attaches a drain tracker for shutdown coordination.
    ///
    /// Increments the counter immediately; decrements on drop.
    pub(crate) fn with_drain_tracker(mut self, tracker: Arc<(AtomicU64, Notify)>) -> Self {
        tracker.0.fetch_add(1, AtomicOrdering::Release);
        self.drain_counter = Some(tracker);
        self
    }

    /// Marks the lease as tainted — it will be destroyed instead of recycled.
    pub fn taint(&mut self) {
        match &mut self.inner {
            HandleInner::Owned(_) => {}, // no-op for owned
            HandleInner::Guarded { tainted, .. } | HandleInner::Shared { tainted, .. } => {
                *tainted = true;
            },
        }
    }

    /// Detaches the lease from pool management, converting to owned.
    ///
    /// Returns `Some(lease)` for owned and guarded handles, `None` for shared
    /// (since the `Arc` may have other holders).
    pub fn detach(mut self) -> Option<R::Lease> {
        match &mut self.inner {
            HandleInner::Owned(_) => {
                // Move out via replacement — we'll forget self afterward.
                let inner = std::mem::replace(
                    &mut self.inner,
                    // Dummy: immediately replaced, never accessed.
                    HandleInner::Guarded {
                        value: None,
                        on_release: None,
                        permit: None,
                        tainted: true,
                        acquired_at: Instant::now(),
                        generation: 0,
                    },
                );
                match inner {
                    HandleInner::Owned(lease) => Some(lease),
                    _ => unreachable!(),
                }
            },
            HandleInner::Guarded { .. } => {
                let inner = std::mem::replace(
                    &mut self.inner,
                    HandleInner::Guarded {
                        value: None,
                        on_release: None,
                        permit: None,
                        tainted: true,
                        acquired_at: Instant::now(),
                        generation: 0,
                    },
                );
                match inner {
                    HandleInner::Guarded {
                        value: Some(lease), ..
                    } => Some(lease),
                    _ => None,
                }
            },
            HandleInner::Shared { .. } => None,
        }
    }

    /// Returns how long this handle has been held.
    pub fn hold_duration(&self) -> std::time::Duration {
        match &self.inner {
            HandleInner::Owned(_) => std::time::Duration::ZERO,
            HandleInner::Guarded { acquired_at, .. } | HandleInner::Shared { acquired_at, .. } => {
                acquired_at.elapsed()
            },
        }
    }

    /// Returns the resource key for this handle.
    pub fn resource_key(&self) -> &ResourceKey {
        &self.resource_key
    }

    /// Returns the topology tag identifying which topology this handle came from.
    pub fn topology_tag(&self) -> TopologyTag {
        self.topology_tag
    }

    /// Returns the generation counter, if this is a pooled handle.
    pub fn generation(&self) -> Option<u64> {
        match &self.inner {
            HandleInner::Owned(_) => None,
            HandleInner::Guarded { generation, .. } | HandleInner::Shared { generation, .. } => {
                Some(*generation)
            },
        }
    }
}

impl<R: Resource> Deref for ResourceHandle<R> {
    type Target = R::Lease;

    fn deref(&self) -> &Self::Target {
        match &self.inner {
            HandleInner::Owned(lease) => lease,
            HandleInner::Guarded {
                value: Some(lease), ..
            } => lease,
            HandleInner::Guarded { value: None, .. } => {
                panic!("ResourceHandle accessed after detach")
            },
            HandleInner::Shared { value, .. } => value,
        }
    }
}

impl<R: Resource> Drop for ResourceHandle<R> {
    fn drop(&mut self) {
        match &mut self.inner {
            HandleInner::Owned(_) => {}, // nothing to do
            HandleInner::Guarded {
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
                            "release callback panicked in ResourceHandle Drop"
                        );
                    }
                }
                // _permit_guard drops here, returning the slot to the semaphore.
            },
            HandleInner::Shared {
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
                            "release callback panicked in ResourceHandle Drop"
                        );
                    }
                }
            },
        }

        // Drain tracking: decrement active count and notify shutdown waiters.
        if let Some(ref tracker) = self.drain_counter
            && tracker.0.fetch_sub(1, AtomicOrdering::Release) == 1
        {
            tracker.1.notify_waiters();
        }
    }
}

impl<R: Resource> std::fmt::Debug for ResourceHandle<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mode = match &self.inner {
            HandleInner::Owned(_) => "Owned",
            HandleInner::Guarded { .. } => "Guarded",
            HandleInner::Shared { .. } => "Shared",
        };
        f.debug_struct("ResourceHandle")
            .field("resource_key", &self.resource_key)
            .field("topology_tag", &self.topology_tag)
            .field("mode", &mode)
            .finish()
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
        type Auth = ();
        fn key() -> ResourceKey {
            nebula_core::resource_key!("dummy")
        }

        async fn create(
            &self,
            _config: &(),
            _auth: &(),
            _ctx: &dyn crate::ctx::Ctx,
        ) -> Result<(), std::convert::Infallible> {
            Ok(())
        }
    }

    fn test_key() -> ResourceKey {
        nebula_core::resource_key!("test")
    }

    #[test]
    fn owned_deref() {
        let handle = ResourceHandle::<DummyResource>::owned(42, test_key(), TopologyTag::Pool);
        assert_eq!(*handle, 42);
    }

    #[test]
    fn guarded_calls_release_on_drop() {
        let released = Arc::new(AtomicBool::new(false));
        let released_clone = released.clone();
        let value = Arc::new(AtomicU32::new(0));
        let value_clone = value.clone();

        {
            let _handle = ResourceHandle::<DummyResource>::guarded(
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
            let _handle = ResourceHandle::<DummyResource>::shared(
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
            let mut handle = ResourceHandle::<DummyResource>::guarded(
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
        let handle = ResourceHandle::<DummyResource>::owned(42, test_key(), TopologyTag::Pool);
        let lease = handle.detach();
        assert_eq!(lease, Some(42));
    }

    #[test]
    fn detach_guarded_returns_lease_and_skips_callback() {
        let released = Arc::new(AtomicBool::new(false));
        let released_clone = released;

        let handle = ResourceHandle::<DummyResource>::guarded(
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
        let handle = ResourceHandle::<DummyResource>::shared(
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
        let handle = ResourceHandle::<DummyResource>::owned(1, test_key(), TopologyTag::Pool);
        assert_eq!(handle.hold_duration(), std::time::Duration::ZERO);
    }

    #[test]
    fn resource_key_and_topology_tag() {
        let key = test_key();
        let handle = ResourceHandle::<DummyResource>::owned(1, key.clone(), TopologyTag::Pool);
        assert_eq!(*handle.resource_key(), key);
        assert_eq!(handle.topology_tag(), TopologyTag::Pool);
    }

    #[test]
    fn taint_on_shared_handle_is_seen_by_callback() {
        let was_tainted = Arc::new(AtomicBool::new(false));
        let wt = was_tainted.clone();

        {
            let mut handle = ResourceHandle::<DummyResource>::shared(
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
            "taint() on Shared handle should be visible in release callback"
        );
    }

    #[test]
    fn detach_guarded_does_not_fire_callback() {
        let released = Arc::new(AtomicBool::new(false));
        let r = released.clone();

        let handle = ResourceHandle::<DummyResource>::guarded(
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
}
