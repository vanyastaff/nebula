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
#[path = "retained_store_tests.rs"]
mod tests;
