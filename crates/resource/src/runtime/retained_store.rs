//! Synchronous ownership store for topology-retained entries.
//!
//! Publication and terminal close share one lock so an entry can never be
//! published after the close fence. Each retained generation has an independent
//! lease fence: a lease on one generation does not delay cleanup of an unrelated
//! ready generation. Entries rejected by the close fence, displaced by
//! replacement, or explicitly retired remain framework-owned until cleanup
//! drains them.

use std::{
    collections::{BTreeMap, btree_map::Entry},
    num::NonZeroU64,
    ops::Deref,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
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

/// Why one or more retained owners remain fenced from cleanup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DrainBlockReason {
    /// Every blocked owner has one or more live leases.
    LiveLeases {
        /// Total live leases across the blocked generations.
        live_lease_count: usize,
    },
    /// At least one generation has invalid lease accounting.
    AccountingPoisoned {
        /// Number of generations whose accounting failed closed.
        poisoned_entry_count: usize,
        /// Live leases on other blocked generations.
        live_lease_count: usize,
    },
}

/// Summary of owners which remain fenced after a retirement drain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DrainBlocked {
    reason: DrainBlockReason,
}

impl DrainBlocked {
    pub(crate) fn reason(self) -> DrainBlockReason {
        self.reason
    }
}

/// One retirement drain which owns every ready generation it removed.
///
/// Blocked generations remain in the store. Callers must transfer `ready` to
/// teardown even when `blocked` reports poisoned or live lease accounting.
#[must_use = "ready retained owners must be transferred to teardown"]
pub(crate) struct RetiredDrain<E> {
    ready: Vec<TrackedRetained<E>>,
    blocked: Option<DrainBlocked>,
}

impl<E> RetiredDrain<E> {
    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.ready.is_empty() && self.blocked.is_none()
    }

    pub(crate) fn into_parts(self) -> (Vec<TrackedRetained<E>>, Option<DrainBlocked>) {
        (self.ready, self.blocked)
    }
}

/// One entry coupled to its abandonment accounting until cleanup completes.
#[must_use = "dropping a tracked retained entry records lifecycle abandonment"]
pub(crate) struct TrackedRetained<E> {
    entry: E,
    leases: Arc<LeaseCounter>,
    loss: TaskLoss,
}

impl<E> TrackedRetained<E> {
    fn new(entry: E, tracker: &AbandonmentTracker, lease_released: &Arc<Notify>) -> Self {
        Self {
            entry,
            leases: Arc::new(LeaseCounter::new(Arc::clone(lease_released))),
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

#[derive(Default)]
struct DrainBlockCounts {
    live_lease_count: usize,
    poisoned_entry_count: usize,
}

impl DrainBlockCounts {
    fn observe(&mut self, lease_state: LeaseState) {
        match lease_state {
            LeaseState::Ready => {},
            LeaseState::Leased(live) => {
                self.live_lease_count = self.live_lease_count.saturating_add(live);
            },
            LeaseState::Poisoned => {
                self.poisoned_entry_count = self.poisoned_entry_count.saturating_add(1);
            },
        }
    }

    fn into_blocked(self) -> Option<DrainBlocked> {
        if self.poisoned_entry_count != 0 {
            Some(DrainBlocked {
                reason: DrainBlockReason::AccountingPoisoned {
                    poisoned_entry_count: self.poisoned_entry_count,
                    live_lease_count: self.live_lease_count,
                },
            })
        } else if self.live_lease_count != 0 {
            Some(DrainBlocked {
                reason: DrainBlockReason::LiveLeases {
                    live_lease_count: self.live_lease_count,
                },
            })
        } else {
            None
        }
    }
}

fn summarize_blocked<'entry, E: 'entry>(
    entries: impl Iterator<Item = &'entry TrackedRetained<E>>,
) -> Option<DrainBlocked> {
    let mut counts = DrainBlockCounts::default();
    for retained in entries {
        counts.observe(retained.leases.lease_state());
    }
    counts.into_blocked()
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
    leases: Arc<LeaseCounter>,
}

struct LeaseCounter {
    state: AtomicUsize,
    lease_released: Arc<Notify>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LeaseState {
    Ready,
    Leased(usize),
    Poisoned,
}

impl LeaseCounter {
    const POISONED: usize = usize::MAX;

    fn new(lease_released: Arc<Notify>) -> Self {
        Self {
            state: AtomicUsize::new(0),
            lease_released,
        }
    }

    fn try_reserve(&self) -> bool {
        match self
            .state
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |state| {
                if state == Self::POISONED {
                    None
                } else {
                    Some(
                        state
                            .checked_add(1)
                            .filter(|next| *next != Self::POISONED)?,
                    )
                }
            }) {
            Ok(_) => true,
            Err(Self::POISONED) => {
                tracing::error!("retained lease accounting is poisoned; refusing a new lease");
                false
            },
            Err(live_lease_count) => {
                let poisoned = self.state.compare_exchange(
                    live_lease_count,
                    Self::POISONED,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                );
                tracing::error!(
                    live_leases = live_lease_count,
                    "retained lease counter exhausted; owner remains fenced"
                );
                if poisoned.is_ok() {
                    self.lease_released.notify_waiters();
                }
                false
            },
        }
    }

    fn release(&self) {
        match self
            .state
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |state| match state {
                Self::POISONED => None,
                0 => Some(Self::POISONED),
                live => Some(live - 1),
            }) {
            Ok(1) => self.lease_released.notify_waiters(),
            Ok(0) => {
                tracing::error!("retained lease accounting underflow; owner remains fenced");
                self.lease_released.notify_waiters();
            },
            Ok(_) => {},
            Err(Self::POISONED) => {
                tracing::error!("retained lease release observed poisoned accounting");
            },
            Err(state) => {
                let _ = self.state.compare_exchange(
                    state,
                    Self::POISONED,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                );
                tracing::error!(
                    lease_state = state,
                    "retained lease release failed; owner remains fenced"
                );
                self.lease_released.notify_waiters();
            },
        }
    }

    fn is_drainable(&self) -> bool {
        self.lease_state() == LeaseState::Ready
    }

    fn lease_state(&self) -> LeaseState {
        // `RetainedLease` drops its cloned entry before `LeaseReservation` performs
        // the AcqRel decrement. AcqRel RMWs chain concurrent lease releases, so an
        // Acquire load which observes the final zero happens after every clone
        // destructor. The store mutex simultaneously prevents a new reservation from
        // racing extraction: `lease` increments while holding it, and both drain
        // methods remove entries while holding it.
        match self.state.load(Ordering::Acquire) {
            0 => LeaseState::Ready,
            Self::POISONED => LeaseState::Poisoned,
            live_lease_count => LeaseState::Leased(live_lease_count),
        }
    }
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
        self.store.release_lease(&self.leases);
    }
}

struct StoreState<E> {
    active: BTreeMap<RetainedId, TrackedRetained<E>>,
    retired: Vec<TrackedRetained<E>>,
    next_id: Option<NonZeroU64>,
    is_closed: bool,
}

/// Per-resource-row owner of active and retired topology entries.
///
/// The store is deliberately not cloneable. Its synchronous close fence and
/// private drain methods keep terminal ownership with the resource framework.
/// Lease accounting is generation-local: cleanup may extract each ready retired
/// generation while unrelated active, leased, or poisoned generations remain
/// fenced. Retirement publication and readiness observation share the state
/// mutex; publishers notify waiters only after releasing that mutex.
/// Accounting covers only entries published into this store. Trusted topology
/// implementations must publish every retained strong owner and must not keep
/// or forget untracked strong aliases outside it. The built-in Resident follows
/// this contract; the public lease API cannot enforce it for arbitrary plugins.
pub struct RetainedStore<E> {
    state: Mutex<StoreState<E>>,
    tracker: AbandonmentTracker,
    lease_released: Arc<Notify>,
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
                is_closed: false,
            }),
            tracker,
            lease_released: Arc::new(Notify::new()),
        }
    }

    /// Absorbs an entry and publishes it only while admission is open.
    pub fn retain(&self, entry: E) -> RetainStatus {
        let tracked = TrackedRetained::new(entry, &self.tracker, &self.lease_released);
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.is_closed {
            state.retired.push(tracked);
            drop(state);
            self.lease_released.notify_waiters();
            return RetainStatus::Retired(StoreRejection::Closed);
        }
        let Some(raw_id) = state.next_id.take() else {
            state.retired.push(tracked);
            drop(state);
            self.lease_released.notify_waiters();
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
        let tracked = TrackedRetained::new(entry, &self.tracker, &self.lease_released);
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.is_closed {
            state.retired.push(tracked);
            drop(state);
            self.lease_released.notify_waiters();
            return ReplaceStatus::Retired(StoreRejection::Closed);
        }
        let (status, retired_is_ready) = match state.active.entry(id) {
            Entry::Occupied(mut active) => {
                let displaced = active.insert(tracked);
                let retired_is_ready = displaced.leases.is_drainable();
                state.retired.push(displaced);
                (ReplaceStatus::Replaced, retired_is_ready)
            },
            Entry::Vacant(_) => {
                state.retired.push(tracked);
                (ReplaceStatus::Retired(StoreRejection::UnknownId), true)
            },
        };
        drop(state);
        if retired_is_ready {
            self.lease_released.notify_waiters();
        }
        status
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
        let retired_is_ready = retired.leases.is_drainable();
        state.retired.push(retired);
        drop(state);
        if retired_is_ready {
            self.lease_released.notify_waiters();
        }
        RetireStatus::Retired
    }

    /// Creates a store-bound tracked clone when the entry supports sharing.
    pub fn lease(&self, id: RetainedId) -> Option<RetainedLease<'_, E>>
    where
        E: Clone,
    {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.is_closed {
            return None;
        }
        let retained = state.active.get(&id)?;
        let entry = retained.entry.clone();
        if !retained.leases.try_reserve() {
            return None;
        }
        Some(RetainedLease {
            entry,
            _reservation: LeaseReservation {
                store: self,
                leases: Arc::clone(&retained.leases),
            },
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
        let has_ready = active
            .values()
            .any(|retained| retained.leases.is_drainable());
        state.retired.extend(active.into_values());
        drop(state);
        if has_ready {
            self.lease_released.notify_waiters();
        }
        retired_count
    }

    /// Waits until every retained entry generation becomes drainable.
    ///
    /// Terminal callers raise [`begin_close`](Self::begin_close) first so the
    /// empty observation is stable. Non-terminal callers must still handle a
    /// subsequent drain being refused if a new lease wins the intervening race.
    pub(crate) async fn wait_terminal_quiescent(&self) -> Result<(), DrainBlocked> {
        loop {
            let released = self.lease_released.notified();
            tokio::pin!(released);
            released.as_mut().enable();
            let blocked = {
                let state = self
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                summarize_blocked(state.active.values().chain(&state.retired))
            };
            match blocked {
                None => return Ok(()),
                Some(blocked)
                    if matches!(
                        blocked.reason(),
                        DrainBlockReason::AccountingPoisoned { .. }
                    ) =>
                {
                    return Err(blocked);
                },
                Some(_) => released.await,
            }
        }
    }

    /// Waits for at least one retired generation to become ready.
    ///
    /// Active leases are deliberately excluded: incremental cleanup only needs
    /// progress in the retirement backlog. Registering the notification before
    /// inspecting the backlog prevents a last-release wakeup from being lost.
    pub(crate) async fn wait_retired_ready(&self) -> Result<(), DrainBlocked> {
        loop {
            let released = self.lease_released.notified();
            tokio::pin!(released);
            released.as_mut().enable();
            let (has_ready, blocked) = {
                let state = self
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                (
                    state
                        .retired
                        .iter()
                        .any(|retained| retained.leases.is_drainable()),
                    summarize_blocked(state.retired.iter()),
                )
            };
            if has_ready || blocked.is_none() {
                return Ok(());
            }
            if let Some(blocked) = blocked
                && matches!(
                    blocked.reason(),
                    DrainBlockReason::AccountingPoisoned {
                        live_lease_count: 0,
                        ..
                    }
                )
            {
                return Err(blocked);
            }
            released.await;
        }
    }

    /// Transfers drainable retired entries and keeps leased generations fenced.
    pub(crate) fn drain_retired(&self) -> RetiredDrain<E> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let retired = std::mem::take(&mut state.retired);
        let mut ready = Vec::new();
        let mut blocked = Vec::new();
        let mut blocked_counts = DrainBlockCounts::default();
        for retained in retired {
            let lease_state = retained.leases.lease_state();
            if lease_state == LeaseState::Ready {
                ready.push(retained);
            } else {
                blocked_counts.observe(lease_state);
                blocked.push(retained);
            }
        }
        let blocked_summary = blocked_counts.into_blocked();
        state.retired = blocked;
        RetiredDrain {
            ready,
            blocked: blocked_summary,
        }
    }

    /// Closes the store and transfers every drainable owner.
    ///
    /// A poisoned or leased generation remains fenced, but does not prevent
    /// healthy siblings from being transferred to teardown.
    pub(crate) fn drain_all(&self) -> RetiredDrain<E> {
        self.begin_close();
        self.drain_retired()
    }

    #[cfg(test)]
    pub(crate) fn poison_for_test(&self, id: RetainedId) {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(retained) = state.active.get(&id) else {
            panic!("test retained generation must be active");
        };
        retained
            .leases
            .state
            .store(LeaseCounter::POISONED, Ordering::Release);
        drop(state);
        self.lease_released.notify_waiters();
    }

    fn release_lease(&self, leases: &LeaseCounter) {
        leases.release();
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
            .field(
                "lease_block",
                &summarize_blocked(state.active.values().chain(&state.retired)),
            )
            .field("is_closed", &state.is_closed)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc, Barrier,
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

    #[derive(Clone)]
    struct SharedGeneration(Arc<GenerationOwner>);

    struct GenerationOwner {
        serial: u64,
        drops: Arc<AtomicUsize>,
    }

    impl Drop for GenerationOwner {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::SeqCst);
        }
    }

    struct DropFencedEntry {
        serial: u64,
        is_lease_clone: bool,
        clone_drop_started: Arc<Barrier>,
        allow_clone_drop: Arc<Barrier>,
        owner_drops: Arc<AtomicUsize>,
    }

    impl Clone for DropFencedEntry {
        fn clone(&self) -> Self {
            Self {
                serial: self.serial,
                is_lease_clone: true,
                clone_drop_started: Arc::clone(&self.clone_drop_started),
                allow_clone_drop: Arc::clone(&self.allow_clone_drop),
                owner_drops: Arc::clone(&self.owner_drops),
            }
        }
    }

    impl Drop for DropFencedEntry {
        fn drop(&mut self) {
            if self.is_lease_clone {
                self.clone_drop_started.wait();
                self.allow_clone_drop.wait();
            } else {
                self.owner_drops.fetch_add(1, Ordering::SeqCst);
            }
        }
    }

    #[test]
    fn repeated_empty_retirement_drain_is_stable() {
        let store = RetainedStore::<SharedGeneration>::for_test();

        assert!(store.drain_retired().is_empty());
        assert!(store.drain_retired().is_empty());
    }

    #[test]
    fn terminal_extraction_waits_for_the_last_clone_destructor() {
        let store = RetainedStore::for_test();
        let clone_drop_started = Arc::new(Barrier::new(2));
        let allow_clone_drop = Arc::new(Barrier::new(2));
        let owner_drops = Arc::new(AtomicUsize::new(0));
        let RetainStatus::Published(id) = store.retain(DropFencedEntry {
            serial: 7,
            is_lease_clone: false,
            clone_drop_started: Arc::clone(&clone_drop_started),
            allow_clone_drop: Arc::clone(&allow_clone_drop),
            owner_drops: Arc::clone(&owner_drops),
        }) else {
            panic!("open store must publish the test generation");
        };
        let lease = store.lease(id).expect("published generation is leasable");
        assert_eq!(store.begin_close(), 1);

        std::thread::scope(|scope| {
            let lease_drop = scope.spawn(move || drop(lease));
            clone_drop_started.wait();

            let drain_while_clone_drops = store.drain_all();
            assert_eq!(owner_drops.load(Ordering::SeqCst), 0);

            allow_clone_drop.wait();
            lease_drop.join().expect("lease drop thread must finish");

            let (ready, blocked) = drain_while_clone_drops.into_parts();
            assert!(
                ready.is_empty(),
                "the leased owner must remain in the store until clone destruction finishes"
            );
            let blocked = blocked.expect("clone destruction must fence owner extraction");
            std::assert_matches!(
                blocked.reason(),
                super::DrainBlockReason::LiveLeases {
                    live_lease_count: 1
                }
            );
        });

        let (mut terminal_entries, blocked) = store.drain_all().into_parts();
        assert_eq!(blocked, None);
        assert_eq!(terminal_entries.len(), 1);
        let (entry, loss) = terminal_entries
            .pop()
            .expect("the retained owner must be extracted exactly once")
            .into_parts();
        assert_eq!(entry.serial, 7);
        drop(entry);
        drop(loss);
        assert_eq!(owner_drops.load(Ordering::SeqCst), 1);
        assert!(store.drain_retired().is_empty());
    }

    #[test]
    fn replace_retire_and_close_preserve_each_generation_exactly_once() {
        let store = RetainedStore::for_test();
        let first_drops = Arc::new(AtomicUsize::new(0));
        let second_drops = Arc::new(AtomicUsize::new(0));
        let RetainStatus::Published(id) =
            store.retain(SharedGeneration(Arc::new(GenerationOwner {
                serial: 1,
                drops: Arc::clone(&first_drops),
            })))
        else {
            panic!("open store must publish the first generation");
        };
        let first_lease = store.lease(id).expect("first generation is leasable");
        assert_eq!(
            store.replace(
                id,
                SharedGeneration(Arc::new(GenerationOwner {
                    serial: 2,
                    drops: Arc::clone(&second_drops),
                })),
            ),
            ReplaceStatus::Replaced
        );
        assert_eq!(super::RetireStatus::Retired, store.retire(id));

        let (ready, blocked) = store.drain_retired().into_parts();
        assert_eq!(ready.len(), 1);
        std::assert_matches!(
            blocked.map(super::DrainBlocked::reason),
            Some(super::DrainBlockReason::LiveLeases {
                live_lease_count: 1
            })
        );
        let (second, second_loss) = ready
            .into_iter()
            .next()
            .expect("unleased second generation is ready")
            .into_parts();
        assert_eq!(second.0.serial, 2);
        drop(second);
        drop(second_loss);
        assert_eq!(second_drops.load(Ordering::SeqCst), 1);
        assert_eq!(first_drops.load(Ordering::SeqCst), 0);

        assert_eq!(store.begin_close(), 0);
        assert!(store.lease(id).is_none());
        drop(first_lease);
        let (first, blocked) = store.drain_all().into_parts();
        assert_eq!(blocked, None);
        assert_eq!(first.len(), 1);
        let (first, first_loss) = first
            .into_iter()
            .next()
            .expect("first generation remained framework-owned")
            .into_parts();
        assert_eq!(first.0.serial, 1);
        drop(first);
        drop(first_loss);
        assert_eq!(first_drops.load(Ordering::SeqCst), 1);
        assert_eq!(second_drops.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn poisoned_retirement_reports_typed_block_reason() {
        let store = RetainedStore::for_test();
        let RetainStatus::Published(id) =
            store.retain(SharedGeneration(Arc::new(GenerationOwner {
                serial: 1,
                drops: Arc::new(AtomicUsize::new(0)),
            })))
        else {
            panic!("open store must publish the test generation");
        };
        let leases = Arc::clone(
            &store
                .state
                .lock()
                .expect("test store lock is healthy")
                .active
                .get(&id)
                .expect("published generation is active")
                .leases,
        );
        leases
            .state
            .store(super::LeaseCounter::POISONED, Ordering::Release);
        assert_eq!(super::RetireStatus::Retired, store.retire(id));

        let retirement = store.drain_retired();
        assert!(retirement.ready.is_empty());
        std::assert_matches!(
            retirement.blocked.map(super::DrainBlocked::reason),
            Some(super::DrainBlockReason::AccountingPoisoned {
                poisoned_entry_count: 1,
                live_lease_count: 0
            })
        );
        let blocked = store
            .wait_retired_ready()
            .await
            .expect_err("poisoned accounting cannot become ready by notification");
        std::assert_matches!(
            blocked.reason(),
            super::DrainBlockReason::AccountingPoisoned {
                poisoned_entry_count: 1,
                live_lease_count: 0
            }
        );
    }

    #[tokio::test(start_paused = true)]
    async fn publishing_ready_retirement_wakes_a_waiter_blocked_by_another_generation() {
        let store = RetainedStore::for_test();
        let drops = Arc::new(AtomicUsize::new(0));
        let RetainStatus::Published(blocked_id) = store.retain(SecretEntry {
            value: "blocked-generation",
            drops: Arc::clone(&drops),
        }) else {
            panic!("open store must publish the blocked generation");
        };
        let blocked_lease = store
            .lease(blocked_id)
            .expect("published generation is leasable");
        assert_eq!(super::RetireStatus::Retired, store.retire(blocked_id));

        let readiness = store.wait_retired_ready();
        tokio::pin!(readiness);
        assert!(
            futures::poll!(readiness.as_mut()).is_pending(),
            "the waiter must park while the only retired generation is leased"
        );

        let RetainStatus::Published(ready_id) = store.retain(SecretEntry {
            value: "ready-generation",
            drops,
        }) else {
            panic!("open store must publish the ready generation");
        };
        assert_eq!(super::RetireStatus::Retired, store.retire(ready_id));

        tokio::time::timeout(std::time::Duration::from_secs(1), readiness)
            .await
            .expect("publishing a ready sibling must wake the parked waiter")
            .expect("ready retirement is observable despite the leased sibling");
        drop(blocked_lease);
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
        let (retired, blocked) = store.drain_retired().into_parts();
        assert_eq!(blocked, None);
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
        let (retired, blocked) = store.drain_retired().into_parts();
        assert_eq!(retired.len(), 1);
        assert_eq!(blocked, None);

        drop(retired);
        drop(store);
        assert_eq!(drops.load(Ordering::SeqCst), 2);
        assert_eq!(queue.dropped_count(), 2);
        queue.close();
        ReleaseQueue::shutdown(workers).await;
    }

    #[tokio::test]
    async fn lease_on_one_entry_does_not_block_unrelated_replacement_retirement() {
        let (queue, workers) = ReleaseQueue::new(1);
        let store = RetainedStore::new(queue.abandonment_tracker());
        let drops = Arc::new(AtomicUsize::new(0));
        let RetainStatus::Published(leased_id) = store.retain(SecretEntry {
            value: "leased-secret",
            drops: Arc::clone(&drops),
        }) else {
            panic!("open store must publish the leased entry");
        };
        let RetainStatus::Published(replaced_id) = store.retain(SecretEntry {
            value: "retired-secret",
            drops: Arc::clone(&drops),
        }) else {
            panic!("open store must publish the replaceable entry");
        };
        let lease = store
            .lease(leased_id)
            .expect("published entry must be leasable");
        assert_eq!(super::RetireStatus::Retired, store.retire(leased_id));

        assert_eq!(
            store.replace(
                replaced_id,
                SecretEntry {
                    value: "replacement-secret",
                    drops: Arc::clone(&drops),
                },
            ),
            ReplaceStatus::Replaced
        );

        let retirement = store.drain_retired();
        assert_eq!(retirement.ready.len(), 1);
        std::assert_matches!(
            retirement.blocked.map(|blocked| blocked.reason),
            Some(super::DrainBlockReason::LiveLeases {
                live_lease_count: 1
            })
        );
        let (entry, loss) = retirement
            .ready
            .into_iter()
            .next()
            .expect("the displaced entry must be drained")
            .into_parts();
        assert_eq!(entry.value, "retired-secret");

        drop(entry);
        drop(loss);
        drop(lease);
        let formerly_blocked = store.drain_retired();
        assert_eq!(formerly_blocked.ready.len(), 1);
        assert_eq!(formerly_blocked.blocked, None);
        let (entry, loss) = formerly_blocked
            .ready
            .into_iter()
            .next()
            .expect("the formerly leased entry must remain owned")
            .into_parts();
        assert_eq!(entry.value, "leased-secret");
        drop(entry);
        drop(loss);
        drop(store);
        queue.close();
        ReleaseQueue::shutdown(workers).await;
    }

    #[tokio::test]
    async fn retired_readiness_ignores_unrelated_active_lease() {
        let store = RetainedStore::for_test();
        let drops = Arc::new(AtomicUsize::new(0));
        let RetainStatus::Published(active_id) = store.retain(SecretEntry {
            value: "active-secret",
            drops: Arc::clone(&drops),
        }) else {
            panic!("open store must publish the active entry");
        };
        let RetainStatus::Published(retired_id) = store.retain(SecretEntry {
            value: "retired-secret",
            drops,
        }) else {
            panic!("open store must publish the retired entry");
        };
        let active_lease = store.lease(active_id).expect("active entry is leasable");
        assert_eq!(super::RetireStatus::Retired, store.retire(retired_id));

        store
            .wait_retired_ready()
            .await
            .expect("active lease must not block retired readiness");
        let (ready, blocked) = store.drain_retired().into_parts();
        assert_eq!(ready.len(), 1);
        assert_eq!(blocked, None);

        drop(ready);
        drop(active_lease);
    }

    #[tokio::test]
    async fn replacement_uses_a_fresh_lease_counter() {
        let (queue, workers) = ReleaseQueue::new(1);
        let store = RetainedStore::new(queue.abandonment_tracker());
        let drops = Arc::new(AtomicUsize::new(0));
        let RetainStatus::Published(id) = store.retain(SecretEntry {
            value: "old-generation",
            drops: Arc::clone(&drops),
        }) else {
            panic!("open store must publish the original generation");
        };
        let old_lease = store.lease(id).expect("original generation is leasable");
        assert_eq!(
            store.replace(
                id,
                SecretEntry {
                    value: "new-generation",
                    drops: Arc::clone(&drops),
                },
            ),
            ReplaceStatus::Replaced
        );
        let new_lease = store.lease(id).expect("replacement generation is leasable");

        drop(old_lease);
        let (retired, blocked) = store.drain_retired().into_parts();
        assert_eq!(blocked, None);
        assert_eq!(retired.len(), 1);
        let (entry, loss) = retired
            .into_iter()
            .next()
            .expect("the predecessor must be drained")
            .into_parts();
        assert_eq!(entry.value, "old-generation");
        let (ready, blocked) = store.drain_all().into_parts();
        assert!(ready.is_empty());
        let blocked = blocked.expect("the replacement lease must fence terminal extraction");
        std::assert_matches!(
            blocked.reason(),
            super::DrainBlockReason::LiveLeases {
                live_lease_count: 1
            }
        );

        drop(entry);
        drop(loss);
        drop(new_lease);
        let (released, blocked) = store.drain_all().into_parts();
        assert_eq!(blocked, None);
        drop(released);
        drop(store);
        queue.close();
        ReleaseQueue::shutdown(workers).await;
    }

    #[test]
    fn lease_counter_overflow_poison_fences_the_owner() {
        let counter = super::LeaseCounter::new(Arc::new(tokio::sync::Notify::new()));
        counter
            .state
            .store(super::LeaseCounter::POISONED - 1, Ordering::Release);

        assert!(!counter.try_reserve());
        assert!(!counter.is_drainable());
    }

    #[test]
    fn lease_counter_underflow_poison_fences_the_owner() {
        let counter = super::LeaseCounter::new(Arc::new(tokio::sync::Notify::new()));

        counter.release();

        assert!(!counter.is_drainable());
        assert!(!counter.try_reserve());
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

    #[tokio::test(start_paused = true)]
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

        let (ready, blocked) = store.drain_all().into_parts();
        assert!(ready.is_empty());
        let blocked = blocked.expect("a live lease must block owner extraction");
        std::assert_matches!(
            blocked.reason(),
            super::DrainBlockReason::LiveLeases {
                live_lease_count: 1
            }
        );
        assert!(
            store.lease(id).is_none(),
            "terminal close must reject leases even while an issued lease remains"
        );

        let wait = store.wait_terminal_quiescent();
        tokio::pin!(wait);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), &mut wait)
                .await
                .is_err(),
            "terminal cleanup must wait while the hook clone is live"
        );
        drop(lease);
        store
            .wait_terminal_quiescent()
            .await
            .expect("released terminal store reaches quiescence");

        let (terminal_entries, blocked) = store.drain_all().into_parts();
        assert_eq!(blocked, None);
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
        let (terminal_entries, blocked) = store.drain_all().into_parts();
        assert_eq!(blocked, None);
        assert_eq!(terminal_entries.len(), 1);
        drop(terminal_entries);
        assert_eq!(queue.dropped_count(), 1);

        drop(store);
        queue.close();
        ReleaseQueue::shutdown(workers).await;
    }
}
