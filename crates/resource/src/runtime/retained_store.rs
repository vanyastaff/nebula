//! Synchronous ownership store for topology-retained entries.
//!
//! Publication and terminal close share one lock so an entry can never be
//! published after the close fence. Entries rejected by that fence, displaced
//! by replacement, or explicitly retired remain framework-owned until cleanup
//! drains them.

use std::{
    collections::{BTreeMap, btree_map::Entry},
    num::NonZeroU64,
    ops::Deref,
    sync::Mutex,
};

use tokio::sync::Notify;

use crate::release_queue::{AbandonmentTracker, TaskLoss};

/// Opaque identity of an entry retained by a resource row.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RetainedId(NonZeroU64);

impl std::fmt::Debug for RetainedId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_tuple("RetainedId").field(&self.0).finish()
    }
}

/// Result of trying to publish a newly retained entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use = "the caller must observe whether the entry was published or retired"]
#[non_exhaustive]
pub enum RetainStatus {
    /// The entry became active under this identity.
    Published(RetainedId),
    /// The entry remains framework-owned in the retirement backlog.
    Retired(StoreRejection),
}

/// Result of replacing an existing retained entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use = "the caller must observe whether the replacement was published or retired"]
#[non_exhaustive]
pub enum ReplaceStatus {
    /// The new entry became active and the displaced entry was retired.
    Replaced,
    /// The supplied entry remains framework-owned in the retirement backlog.
    Retired(StoreRejection),
}

/// Result of explicitly retiring an existing entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use = "the caller must observe whether an active entry was retired"]
#[non_exhaustive]
pub enum RetireStatus {
    /// The active entry was moved to the retirement backlog.
    Retired,
    /// No active entry had the supplied identity.
    NotFound,
}

/// Why a supplied entry was retained for cleanup instead of publication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum StoreRejection {
    /// The terminal close fence has already been raised.
    Closed,
    /// No active entry had the supplied identity.
    UnknownId,
    /// The row exhausted its monotonic identity space.
    IdentifierExhausted,
}

/// Why retained owners cannot be transferred to cleanup yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DrainBlocked {
    live_lease_count: usize,
}

impl DrainBlocked {
    /// Returns the number of store-bound clones still using retained owners.
    pub(crate) fn live_lease_count(self) -> usize {
        self.live_lease_count
    }
}

/// One entry coupled to its abandonment accounting until cleanup completes.
#[must_use = "dropping a tracked retained entry records lifecycle abandonment"]
pub(crate) struct TrackedRetained<E> {
    entry: E,
    loss: TaskLoss,
}

impl<E> TrackedRetained<E> {
    fn new(entry: E, tracker: &AbandonmentTracker) -> Self {
        Self {
            entry,
            loss: tracker.track_entries(1),
        }
    }

    /// Transfers the entry and its original accounting guard to cleanup.
    pub(crate) fn into_parts(self) -> (E, TaskLoss) {
        (self.entry, self.loss)
    }
}

impl<E> std::fmt::Debug for TrackedRetained<E> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TrackedRetained")
            .field("entry", &"<redacted>")
            .finish_non_exhaustive()
    }
}

/// Store-bound clone of a retained entry.
///
/// The close fence rejects new leases. Terminal cleanup waits for existing
/// leases to drop before transferring retained owners to destruction.
/// Trusted in-process topology code must not clone strong aliases out of this
/// lease or forget the lease. Such escapes violate the lifecycle contract;
/// Rust borrowing does not prevent them and the framework cannot account for them.
#[must_use = "the retained lease keeps terminal cleanup from draining its owner"]
pub struct RetainedLease<'store, E> {
    // Field order is intentional: Rust drops fields in declaration order, so
    // the clone is gone before the reservation announces quiescence.
    entry: E,
    _reservation: LeaseReservation<'store, E>,
}

struct LeaseReservation<'store, E> {
    store: &'store RetainedStore<E>,
}

impl<E> Deref for RetainedLease<'_, E> {
    type Target = E;

    fn deref(&self) -> &Self::Target {
        &self.entry
    }
}

impl<E> std::fmt::Debug for RetainedLease<'_, E> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RetainedLease")
            .field("entry", &"<redacted>")
            .finish_non_exhaustive()
    }
}

impl<E> Drop for LeaseReservation<'_, E> {
    fn drop(&mut self) {
        self.store.release_lease();
    }
}

struct StoreState<E> {
    active: BTreeMap<RetainedId, TrackedRetained<E>>,
    retired: Vec<TrackedRetained<E>>,
    next_id: Option<NonZeroU64>,
    live_leases: usize,
    is_closed: bool,
}

/// Per-resource-row owner of active and retired topology entries.
///
/// The store is deliberately not cloneable. Its synchronous close fence and
/// private drain methods keep terminal ownership with the resource framework.
/// Accounting covers only entries published into this store. Trusted topology
/// implementations must publish every retained strong owner and must not keep
/// or forget untracked strong aliases outside it. The built-in Resident follows
/// this contract; the public lease API cannot enforce it for arbitrary plugins.
pub struct RetainedStore<E> {
    state: Mutex<StoreState<E>>,
    tracker: AbandonmentTracker,
    lease_released: Notify,
}

impl<E> RetainedStore<E> {
    #[cfg(test)]
    pub(crate) fn for_test() -> Self {
        Self::new(AbandonmentTracker::for_test())
    }

    pub(crate) fn new(tracker: AbandonmentTracker) -> Self {
        Self {
            state: Mutex::new(StoreState {
                active: BTreeMap::new(),
                retired: Vec::new(),
                next_id: NonZeroU64::new(1),
                live_leases: 0,
                is_closed: false,
            }),
            tracker,
            lease_released: Notify::new(),
        }
    }

    /// Absorbs an entry and publishes it only while admission is open.
    pub fn retain(&self, entry: E) -> RetainStatus {
        let tracked = TrackedRetained::new(entry, &self.tracker);
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.is_closed {
            state.retired.push(tracked);
            return RetainStatus::Retired(StoreRejection::Closed);
        }
        let Some(raw_id) = state.next_id.take() else {
            state.retired.push(tracked);
            return RetainStatus::Retired(StoreRejection::IdentifierExhausted);
        };
        state.next_id = raw_id.get().checked_add(1).and_then(NonZeroU64::new);
        let id = RetainedId(raw_id);
        let previous = state.active.insert(id, tracked);
        debug_assert!(
            previous.is_none(),
            "monotonic retained identifiers cannot replace an active entry"
        );
        RetainStatus::Published(id)
    }

    /// Absorbs a replacement, retiring both invalid inputs and displacement.
    pub fn replace(&self, id: RetainedId, entry: E) -> ReplaceStatus {
        let tracked = TrackedRetained::new(entry, &self.tracker);
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.is_closed {
            state.retired.push(tracked);
            return ReplaceStatus::Retired(StoreRejection::Closed);
        }
        match state.active.entry(id) {
            Entry::Occupied(mut active) => {
                let displaced = active.insert(tracked);
                state.retired.push(displaced);
                ReplaceStatus::Replaced
            },
            Entry::Vacant(_) => {
                state.retired.push(tracked);
                ReplaceStatus::Retired(StoreRejection::UnknownId)
            },
        }
    }

    /// Moves an active entry to the cleanup backlog without exposing it.
    pub fn retire(&self, id: RetainedId) -> RetireStatus {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(retired) = state.active.remove(&id) else {
            return RetireStatus::NotFound;
        };
        state.retired.push(retired);
        RetireStatus::Retired
    }

    /// Creates a store-bound tracked clone when the entry supports sharing.
    pub fn lease(&self, id: RetainedId) -> Option<RetainedLease<'_, E>>
    where
        E: Clone,
    {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.is_closed {
            return None;
        }
        let entry = state.active.get(&id)?.entry.clone();
        let Some(live_leases) = state.live_leases.checked_add(1) else {
            tracing::error!("retained lease counter exhausted; refusing a new lease");
            return None;
        };
        state.live_leases = live_leases;
        Some(RetainedLease {
            entry,
            _reservation: LeaseReservation { store: self },
        })
    }

    /// Closes publication and synchronously transfers active entries to backlog.
    pub(crate) fn begin_close(&self) -> usize {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.is_closed = true;
        let active = std::mem::take(&mut state.active);
        let retired_count = active.len();
        state.retired.extend(active.into_values());
        retired_count
    }

    /// Waits until the current lease set becomes empty.
    ///
    /// Terminal callers raise [`begin_close`](Self::begin_close) first so the
    /// empty observation is stable. Non-terminal callers must still handle a
    /// subsequent drain being refused if a new lease wins the intervening race.
    pub(crate) async fn wait_quiescent(&self) {
        loop {
            let released = self.lease_released.notified();
            tokio::pin!(released);
            released.as_mut().enable();
            let is_quiescent = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .live_leases
                == 0;
            if is_quiescent {
                return;
            }
            released.await;
        }
    }

    /// Transfers entries already selected for cleanup to the framework.
    pub(crate) fn drain_retired(&self) -> Result<Vec<TrackedRetained<E>>, DrainBlocked> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.live_leases != 0 {
            return Err(DrainBlocked {
                live_lease_count: state.live_leases,
            });
        }
        Ok(std::mem::take(&mut state.retired))
    }

    /// Closes the store and transfers every retained owner to the framework.
    pub(crate) fn drain_all(&self) -> Result<Vec<TrackedRetained<E>>, DrainBlocked> {
        self.begin_close();
        self.drain_retired()
    }

    fn release_lease(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(live_leases) = state.live_leases.checked_sub(1) else {
            tracing::error!("retained lease accounting underflow");
            return;
        };
        state.live_leases = live_leases;
        let is_quiescent = state.live_leases == 0;
        drop(state);
        if is_quiescent {
            self.lease_released.notify_waiters();
        }
    }
}

impl<E> std::fmt::Debug for RetainedStore<E> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        formatter
            .debug_struct("RetainedStore")
            .field("active_count", &state.active.len())
            .field("retired_count", &state.retired.len())
            .field("live_lease_count", &state.live_leases)
            .field("is_closed", &state.is_closed)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use super::{ReplaceStatus, RetainStatus, RetainedStore, StoreRejection};
    use crate::release_queue::ReleaseQueue;

    #[derive(Clone)]
    struct SecretEntry {
        value: &'static str,
        drops: Arc<AtomicUsize>,
    }

    impl Drop for SecretEntry {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn close_fence_rejects_publication_but_keeps_entry_for_retirement() {
        let (queue, workers) = ReleaseQueue::new(1);
        let store = RetainedStore::new(queue.abandonment_tracker());
        assert_eq!(store.begin_close(), 0);

        let drops = Arc::new(AtomicUsize::new(0));
        let status = store.retain(SecretEntry {
            value: "secret-after-close",
            drops: Arc::clone(&drops),
        });

        assert_eq!(status, RetainStatus::Retired(StoreRejection::Closed));
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        let retired = store.drain_retired().expect("no lease blocks retirement");
        assert_eq!(retired.len(), 1);
        let (entry, loss) = retired
            .into_iter()
            .next()
            .expect("one retained entry")
            .into_parts();
        assert_eq!(entry.value, "secret-after-close");
        drop(entry);
        drop(loss);
        assert_eq!(queue.dropped_count(), 1);

        queue.close();
        ReleaseQueue::shutdown(workers).await;
    }

    #[tokio::test]
    async fn replace_retires_displaced_entry_without_returning_it() {
        let (queue, workers) = ReleaseQueue::new(1);
        let store = RetainedStore::new(queue.abandonment_tracker());
        let drops = Arc::new(AtomicUsize::new(0));
        let RetainStatus::Published(id) = store.retain(SecretEntry {
            value: "old-secret",
            drops: Arc::clone(&drops),
        }) else {
            panic!("open store must publish its first entry");
        };

        assert_eq!(
            store.replace(
                id,
                SecretEntry {
                    value: "new-secret",
                    drops: Arc::clone(&drops),
                },
            ),
            ReplaceStatus::Replaced
        );
        assert_eq!(
            store
                .state
                .lock()
                .unwrap()
                .active
                .get(&id)
                .map(|tracked| tracked.entry.value),
            Some("new-secret")
        );
        assert_eq!(
            store
                .drain_retired()
                .expect("no lease blocks retirement")
                .len(),
            1
        );

        drop(store);
        assert_eq!(drops.load(Ordering::SeqCst), 2);
        assert_eq!(queue.dropped_count(), 2);
        queue.close();
        ReleaseQueue::shutdown(workers).await;
    }

    #[tokio::test]
    async fn dropping_store_accounts_for_every_undrained_owner() {
        let (queue, workers) = ReleaseQueue::new(1);
        let store = RetainedStore::new(queue.abandonment_tracker());
        let drops = Arc::new(AtomicUsize::new(0));
        let RetainStatus::Published(id) = store.retain(SecretEntry {
            value: "active-secret",
            drops: Arc::clone(&drops),
        }) else {
            panic!("open store must publish its first entry");
        };
        assert_eq!(super::RetireStatus::Retired, store.retire(id));
        let _ = store.retain(SecretEntry {
            value: "active-secret-two",
            drops: Arc::clone(&drops),
        });

        drop(store);
        assert_eq!(drops.load(Ordering::SeqCst), 2);
        assert_eq!(queue.dropped_count(), 2);
        queue.close();
        ReleaseQueue::shutdown(workers).await;
    }

    #[tokio::test]
    async fn debug_redacts_entries_and_tracker_state() {
        let (queue, workers) = ReleaseQueue::new(1);
        let store = RetainedStore::new(queue.abandonment_tracker());
        let _ = store.retain(SecretEntry {
            value: "never-print-this",
            drops: Arc::new(AtomicUsize::new(0)),
        });

        let rendered = format!("{store:?}");
        assert!(!rendered.contains("never-print-this"));
        assert!(rendered.contains("active_count"));

        drop(store);
        queue.close();
        ReleaseQueue::shutdown(workers).await;
    }

    #[tokio::test]
    async fn terminal_drain_waits_until_the_last_store_bound_clone_drops() {
        let (queue, workers) = ReleaseQueue::new(1);
        let store = RetainedStore::new(queue.abandonment_tracker());
        let drops = Arc::new(AtomicUsize::new(0));
        let RetainStatus::Published(id) = store.retain(SecretEntry {
            value: "hook-secret",
            drops: Arc::clone(&drops),
        }) else {
            panic!("open store must publish its first entry");
        };
        let lease = store.lease(id).expect("published entry has a lease");

        let blocked = store
            .drain_all()
            .expect_err("a live lease must block owner extraction");
        assert_eq!(blocked.live_lease_count(), 1);

        let wait = store.wait_quiescent();
        tokio::pin!(wait);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), &mut wait)
                .await
                .is_err(),
            "terminal cleanup must wait while the hook clone is live"
        );
        drop(lease);
        store.wait_quiescent().await;

        let terminal_entries = store
            .drain_all()
            .expect("quiescent store transfers terminal owners");
        assert_eq!(terminal_entries.len(), 1);
        let rendered = format!("{:?}", terminal_entries[0]);
        assert!(!rendered.contains("hook-secret"));

        drop(terminal_entries);
        assert_eq!(drops.load(Ordering::SeqCst), 2);
        assert_eq!(queue.dropped_count(), 1);
        queue.close();
        ReleaseQueue::shutdown(workers).await;
    }

    #[tokio::test]
    async fn close_race_keeps_the_entry_owned_on_both_linearization_orders() {
        let (queue, workers) = ReleaseQueue::new(1);
        let store = Arc::new(RetainedStore::new(queue.abandonment_tracker()));
        let barrier = Arc::new(tokio::sync::Barrier::new(3));
        let close_store = Arc::clone(&store);
        let close_barrier = Arc::clone(&barrier);
        let close = tokio::spawn(async move {
            close_barrier.wait().await;
            close_store.begin_close();
        });
        let retain_store = Arc::clone(&store);
        let retain_barrier = Arc::clone(&barrier);
        let retain = tokio::spawn(async move {
            retain_barrier.wait().await;
            retain_store.retain(SecretEntry {
                value: "racing-secret",
                drops: Arc::new(AtomicUsize::new(0)),
            })
        });

        barrier.wait().await;
        close.await.expect("close task must not panic");
        let status = retain.await.expect("retain task must not panic");
        std::assert_matches!(
            status,
            RetainStatus::Published(_) | RetainStatus::Retired(StoreRejection::Closed)
        );
        let terminal_entries = store
            .drain_all()
            .expect("no lease blocks terminal extraction");
        assert_eq!(terminal_entries.len(), 1);
        drop(terminal_entries);
        assert_eq!(queue.dropped_count(), 1);

        drop(store);
        queue.close();
        ReleaseQueue::shutdown(workers).await;
    }
}
