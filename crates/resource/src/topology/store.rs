//! Framework-owned instance storage for resource topologies.
//!
//! [`InstanceStore<S>`] is the framework-controlled holder for leased instances
//! that [`Topology`] implementations borrow through lifecycle hooks. It carries the
//! idle queue, the generation/revoke-epoch state, and the uniform revoke-epoch
//! fence that runs on every `return_entry` path — for both built-in and custom
//! topologies. A separate monotonic terminal fence prevents a retiring store
//! from admitting new idle entries or issuing fresh checkouts.
//!
//! # Vocabulary: slot vs entry vs lease
//!
//! Three words that sound interchangeable name three distinct things in this
//! crate — keeping them apart matters for reading the acquire pipeline:
//!
//! - **slot** — the **credential axis**. A `#[credential(key = "...")]` field
//!   on a resource struct, resolved into a [`SlotCell`](crate::SlotCell) and
//!   addressed by name (`refresh_slot`, `taint_slot`, `revoke_slot`,
//!   `dispatch_slot_hook`, the `SLOT_*` derive constants). Orthogonal to
//!   storage — a slot-less resource still has entries.
//! - **entry** — the **store axis**. The leasable unit [`Topology::Entry`]
//!   this module's [`InstanceStore`] holds and fences on revoke-epoch:
//!   `PoolEntry<R>` (framework-internal) for Pooled, `R::Instance` itself for
//!   Resident. `StoreEntry` (this module's internal queue wrapper — payload +
//!   checkout epoch) is never named by an author topology.
//! - **lease** — the **caller-held usage period** between an `acquire_*` call
//!   returning a [`ResourceGuard`](crate::guard::ResourceGuard) and that
//!   guard's drop. Prose only in this crate (not a type family — the typed
//!   `Lease` vocabulary belongs to `nebula-credential`'s rotation events).
//!
//! A fourth word shows up nearby but names none of these: "permit" /
//! "concurrency slot" in the guard/semaphore code (`OwnedSemaphorePermit`,
//! `Topology::try_reserve`) is the **concurrency axis** — how many leases may
//! be outstanding at once — unrelated to the credential axis above despite
//! sharing the word "slot".
//!
//! [`Topology`]: crate::topology::Topology
//! [`Topology::Entry`]: crate::topology::Topology::Entry

use std::{
    collections::VecDeque,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use tokio::sync::{Mutex, MutexGuard};
use tracing::debug;

use crate::release_queue::AbandonmentTracker;

// ─── PoolStrategy ─────────────────────────────────────────────────────────────

/// Idle-queue ordering strategy: which end of the queue a returned entry
/// re-enters.
///
/// Checkout always pops the **front** ([`InstanceStore::checkout`]); the
/// strategy chooses the **push side** on return
/// ([`return_entry`](InstanceStore::return_entry) /
/// [`deposit_fresh`](InstanceStore::deposit_fresh)):
///
/// - [`Lifo`](Self::Lifo) pushes to the front — the most recently returned
///   entry is reused first, keeping a hot working set warm while the queue's
///   tail ages out, so an `idle_timeout` reaper can actually shrink the pool
///   under falling load.
/// - [`Fifo`](Self::Fifo) pushes to the back — leases rotate through every
///   idle entry for even wear, keeping the whole pool warm at the cost of
///   never letting any entry idle long enough to be reaped.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum PoolStrategy {
    /// Last-in, first-out — reuses the most recently returned instance.
    #[default]
    Lifo,
    /// First-in, first-out — spreads load evenly across instances.
    Fifo,
}

// ─── InstanceStore ────────────────────────────────────────────────────────────

/// A timestamped queue entry: the leasable [`Topology::Entry`] value plus the
/// revoke-epoch snapshot taken when it was checked out.
///
/// The epoch is captured at checkout time so a `return_entry` after a
/// `bump_revoke_epoch()` detects the stale epoch and evicts rather than
/// re-pooling.
///
/// `pub(crate)` so the built-in [`Pooled`](crate::topology::Pooled)
/// pipeline can iterate the idle queue under [`InstanceStore::lock_idle`] and
/// read each item's `.entry` during rotation fan-out without copying it out of
/// the store. The fields stay crate-visible only — author topologies receive a
/// `&InstanceStore` and never name `StoreEntry`.
///
/// [`Topology::Entry`]: crate::topology::Topology::Entry
pub(crate) struct StoreEntry<S> {
    pub(crate) entry: S,
    /// The store's revoke-epoch as observed when this entry was **checked
    /// out** (via [`InstanceStore::checkout`] → stamps with the live
    /// counter).
    pub(crate) checkout_epoch: u64,
}

struct StoreInner<S> {
    idle: Mutex<VecDeque<StoreEntry<S>>>,
    revoke_epoch: AtomicU64,
    closed: AtomicBool,
    abandonment: Option<AbandonmentTracker>,
}

impl<S> Drop for StoreInner<S> {
    fn drop(&mut self) {
        let remaining = self.idle.get_mut().len();
        if remaining != 0
            && let Some(tracker) = &self.abandonment
        {
            // Record ownership loss before the mutex field drops its raw
            // entries. The loss guard is intentionally never disarmed here:
            // reaching final inner Drop means no framework cleanup owner
            // survived to receive these entries.
            drop(tracker.track_entries(remaining));
        }
    }
}

/// Framework-owned idle queue and revoke-epoch state for a
/// [`Topology`](crate::topology::Topology)'s entries.
///
/// An `InstanceStore<S>` is the storage the [`Manager`] owns; a
/// [`Topology`](crate::topology::Topology) implementation receives a borrowed
/// `&InstanceStore<Self::Entry>` in [`try_reserve`] /
/// [`on_release`] and [`phase`] / [`load`]. Topology implementations must leave
/// checkout, publication and destruction to the framework. This technical
/// store is not an authorization proof: its cloneable handles share state,
/// and do not independently establish tenant isolation.
///
/// # Revoke-epoch fence
///
/// The fence is uniform: every entry returned via [`return_entry`] is checked
/// against the live epoch (loaded with `Acquire` ordering); an entry whose
/// checkout epoch is *behind* the live counter was leased under a since-revoked
/// credential and is **evicted** (not re-pooled). [`bump_revoke_epoch`] is
/// called by the framework synchronously when a credential is revoked —
/// exactly as `PoolRuntime::bump_revoke_epoch` is called today.
///
/// # Examples
///
/// ```
/// use nebula_resource::InstanceStore;
///
/// # #[tokio::main(flavor = "current_thread")]
/// # async fn main() {
/// let store: InstanceStore<u32> = InstanceStore::new(Some(4));
/// let epoch = store.stamp_epoch();
///
/// // Deposit an entry.
/// store.return_entry(42u32, epoch).await;
/// assert_eq!(store.len().await, 1);
///
/// // Check out the entry (fenced on revoke).
/// let checkout = store.checkout().await;
/// assert!(checkout.stale.is_empty());
/// assert_eq!(checkout.fresh.map(|c| c.entry), Some(42u32));
///
/// // Simulate a credential revoke.
/// store.bump_revoke_epoch();
/// // The old epoch is now stale — returning it evicts.
/// let outcome = store.return_entry(99u32, epoch).await;
/// assert!(outcome.is_evict());
/// # }
/// ```
///
/// [`Manager`]: crate::Manager
/// [`try_reserve`]: crate::topology::Topology::try_reserve
/// [`on_release`]: crate::topology::Topology::on_release
/// [`phase`]: crate::topology::Topology::phase
/// [`load`]: crate::topology::Topology::load
/// [`return_entry`]: InstanceStore::return_entry
/// [`bump_revoke_epoch`]: InstanceStore::bump_revoke_epoch
pub struct InstanceStore<S> {
    /// One shared owner ensures abandonment is observed only at final-handle
    /// drop, never when an intermediate cloned handle goes away.
    inner: Arc<StoreInner<S>>,
    /// Maximum number of entries the store will hold idle.
    /// `None` = unbounded (Resident / permit-only topologies).
    capacity: Option<usize>,
    /// Which end of the idle queue a returned entry re-enters — see
    /// [`PoolStrategy`]. Checkout always pops the front.
    strategy: PoolStrategy,
}

impl<S> Clone for InstanceStore<S> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            capacity: self.capacity,
            strategy: self.strategy,
        }
    }
}

impl<S> std::fmt::Debug for InstanceStore<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InstanceStore")
            .field("capacity", &self.capacity)
            .field("strategy", &self.strategy)
            .field(
                "revoke_epoch",
                &self.inner.revoke_epoch.load(Ordering::Acquire),
            )
            .field("closed", &self.inner.closed.load(Ordering::Acquire))
            .finish()
    }
}

impl<S: Send + 'static> InstanceStore<S> {
    /// Creates a new store with an optional idle capacity cap.
    ///
    /// Pass `None` for unbounded (e.g., Resident or permit-only topologies);
    /// pass `Some(n)` for Pooled-like topologies that cap the idle queue.
    /// The idle queue defaults to FIFO ordering — see
    /// [`with_strategy`](Self::with_strategy).
    pub fn new(capacity: Option<usize>) -> Self {
        Self::from_parts(capacity, None)
    }

    /// Creates a manager-owned store whose final idle owners are loss-accounted.
    pub(crate) fn with_abandonment_tracker(
        capacity: Option<usize>,
        tracker: AbandonmentTracker,
    ) -> Self {
        Self::from_parts(capacity, Some(tracker))
    }

    fn from_parts(capacity: Option<usize>, abandonment: Option<AbandonmentTracker>) -> Self {
        Self {
            inner: Arc::new(StoreInner {
                idle: Mutex::new(VecDeque::new()),
                revoke_epoch: AtomicU64::new(0),
                closed: AtomicBool::new(false),
                abandonment,
            }),
            capacity,
            strategy: PoolStrategy::Fifo,
        }
    }

    /// Sets the idle-queue ordering strategy (see [`PoolStrategy`]).
    ///
    /// Ordering only matters when the store can hold more than one idle entry
    /// (Pooled); single-entry and permit-only topologies are unaffected by
    /// either choice.
    #[must_use]
    pub fn with_strategy(mut self, strategy: PoolStrategy) -> Self {
        self.strategy = strategy;
        self
    }

    /// The configured idle-queue ordering strategy.
    pub fn strategy(&self) -> PoolStrategy {
        self.strategy
    }

    /// Enqueues on the strategy's push side: back for FIFO (even wear),
    /// front for LIFO (hot-set reuse). Checkout always pops the front.
    fn enqueue(&self, idle: &mut VecDeque<StoreEntry<S>>, item: StoreEntry<S>) {
        match self.strategy {
            PoolStrategy::Fifo => idle.push_back(item),
            PoolStrategy::Lifo => idle.push_front(item),
        }
    }

    /// Reads the current revoke epoch.
    pub fn current_revoke_epoch(&self) -> u64 {
        self.inner.revoke_epoch.load(Ordering::Acquire)
    }

    /// Advances the revoke epoch by one.
    ///
    /// Called synchronously by the framework when a credential bound to this
    /// store's resource is revoked — before the revoke hook is dispatched.
    /// After this call every subsequent `return_entry` will evict any entry
    /// whose `checkout_epoch` is behind the new counter.
    pub fn bump_revoke_epoch(&self) {
        self.inner.revoke_epoch.fetch_add(1, Ordering::Release);
    }

    /// Publishes the terminal fence without waiting for the idle lock.
    ///
    /// An operation that already observed the open state can finish under its
    /// idle lock. `close_and_drain` subsequently takes that same lock and
    /// collects its deposit. The framework rechecks closure before issuing a
    /// lease whose checkout raced this signal.
    pub(crate) fn begin_close(&self) {
        if !self.inner.closed.swap(true, Ordering::AcqRel) {
            debug!("resource instance store closed to new leases and deposits");
        }
    }

    /// Whether retirement has begun. This state never transitions back to open.
    pub(crate) fn is_closed(&self) -> bool {
        self.inner.closed.load(Ordering::Acquire)
    }

    /// Closes the store and transfers all remaining idle entries to the owner.
    ///
    /// Cancellation while acquiring the lock leaves the store closed and its
    /// entries intact. Once acquired, collection has no cancellation point.
    /// The returned entries require framework-owned teardown, not plain drop.
    #[tracing::instrument(skip_all)]
    pub(crate) async fn close_and_drain(&self) -> Vec<S> {
        self.begin_close();
        let mut idle = self.inner.idle.lock().await;
        debug!(
            idle_count = idle.len(),
            "draining terminal resource instance store"
        );
        idle.drain(..).map(|item| item.entry).collect()
    }

    /// Checks out the first **fresh** idle entry, running the revoke-epoch
    /// fence on pop (framework-owned).
    ///
    /// The fence runs on **both** directions now (checkout and return): an
    /// idle entry whose `checkout_epoch` is behind the live revoke counter was
    /// leased under a since-revoked credential and must **never** be handed
    /// out again. This method pops idle entries under the idle lock; any entry
    /// whose epoch is stale is collected into [`Checkout::stale`] (for the
    /// framework to destroy via [`Provider::destroy`]) and is **never**
    /// returned as fresh. The first entry whose epoch is current is returned
    /// as [`Checkout::fresh`]; if the queue drains without a fresh entry,
    /// `fresh` is `None`. Once retirement is observed, every remaining idle
    /// entry is returned in `stale` for destruction, irrespective of its epoch.
    ///
    /// The framework acquire pipeline destroys every entry in `stale` before
    /// using `fresh`. The store cannot call `Provider::destroy` itself (it
    /// holds no `Provider`), so it returns the stale entries to the caller for
    /// destruction.
    ///
    /// # Fence guarantee
    ///
    /// The epoch comparison and the pop are performed while holding the idle
    /// lock — the same lock the credential-revoke idle-walk
    /// ([`evict_stale`](Self::evict_stale)) holds — so an entry revoked while
    /// idle is observed as stale here even if the revoke raced the checkout.
    ///
    /// # Cancel safety
    ///
    /// This method is cancel safe. If the future is dropped before the
    /// idle-queue lock is acquired, no entry is popped and the store is
    /// unchanged. Once the lock is held the method completes without any
    /// further await, so a drop cannot observe or leave behind a partial
    /// state. (The returned [`Checkout`] transfers ownership of live entries —
    /// the *caller* must not drop it across a cancellation point without
    /// destroying them.)
    ///
    /// [`Provider::destroy`]: crate::resource::Provider::destroy
    pub async fn checkout(&self) -> Checkout<S> {
        let mut idle = self.inner.idle.lock().await;
        // Revoke-epoch fence: read under the idle lock (same lock the
        // credential-revoke idle-walk holds) so the epoch snapshot is
        // atomic against a concurrent `bump_revoke_epoch`. Without this,
        // a revoke landing between the snapshot and the lock acquire would
        // let a stale entry escape as `fresh`.
        let live_epoch = self.current_revoke_epoch();
        let closed = self.is_closed();
        let mut stale = Vec::new();
        while let Some(item) = idle.pop_front() {
            if closed || item.checkout_epoch != live_epoch {
                // Retired or leased under a since-revoked credential — never hand out.
                debug!(
                    checkout_epoch = item.checkout_epoch,
                    live_epoch,
                    closed,
                    "InstanceStore::checkout: fenced entry requires destruction"
                );
                stale.push(item.entry);
                continue;
            }
            return Checkout {
                fresh: Some(CheckedOut {
                    entry: item.entry,
                    checkout_epoch: item.checkout_epoch,
                }),
                stale,
            };
        }
        Checkout { fresh: None, stale }
    }

    /// Returns an entry to the idle queue, running the revoke-epoch fence.
    ///
    /// If the entry's `checkout_epoch` is behind the live revoke counter, the
    /// entry was leased under a since-revoked credential and is **not**
    /// re-queued — it is handed back via [`ReturnOutcome::Evict`] for the
    /// caller to destroy. Same when the optional capacity cap is already
    /// reached or retirement has begun. Otherwise the entry is enqueued and [`ReturnOutcome::Recycled`]
    /// is returned.
    ///
    /// Returning the evicted entry (rather than swallowing it) lets the
    /// framework drive async eviction (e.g. calling `Provider::destroy`)
    /// without the store owning `Provider`.
    ///
    /// # Fence guarantee
    ///
    /// The epoch re-read and the push are performed while holding the idle
    /// lock, so a concurrent `bump_revoke_epoch` followed by an idle-walk
    /// cannot enqueue a stale entry: the walk holds the same lock and sees the
    /// already-bumped counter.
    ///
    /// # Cancel safety
    ///
    /// The lock-then-mutate shape is cancel safe (a drop before the lock is
    /// acquired mutates nothing; after, the method finishes without another
    /// await) — but the future *owns* `entry` while it waits for the lock, so
    /// a caller that can be cancelled must hold the entry in a destroy-on-drop
    /// guard (the framework acquire loop's `EntryCreateGuard` pattern) rather
    /// than rely on this method to place it.
    pub async fn return_entry(&self, entry: S, checkout_epoch: u64) -> ReturnOutcome<S> {
        let mut idle = self.inner.idle.lock().await;
        self.return_entry_locked(&mut idle, entry, checkout_epoch)
    }

    /// Synchronous core of [`return_entry`](Self::return_entry) for a caller
    /// that already holds the idle lock and still owns a cancellation guard.
    pub(crate) fn return_entry_locked(
        &self,
        idle: &mut VecDeque<StoreEntry<S>>,
        entry: S,
        checkout_epoch: u64,
    ) -> ReturnOutcome<S> {
        if self.is_closed() {
            debug!("InstanceStore::return_entry: terminal fence — evicting entry");
            return ReturnOutcome::Evict(entry);
        }
        // Revoke-epoch fence: re-read under the idle lock (same lock the
        // credential-revoke idle-walk holds) to make compare-then-push
        // atomic against a concurrent revoke.
        let live_epoch = self.inner.revoke_epoch.load(Ordering::Acquire);
        if checkout_epoch != live_epoch {
            // Entry was leased under a since-revoked credential — evict.
            debug!(
                checkout_epoch,
                live_epoch, "InstanceStore::return_entry: epoch mismatch — evicting entry"
            );
            return ReturnOutcome::Evict(entry);
        }
        // Capacity check.
        if let Some(cap) = self.capacity
            && idle.len() >= cap
        {
            return ReturnOutcome::Evict(entry);
        }
        self.enqueue(
            idle,
            StoreEntry {
                entry,
                checkout_epoch,
            },
        );
        ReturnOutcome::Recycled
    }

    /// Number of idle entries currently in the queue.
    pub async fn len(&self) -> usize {
        self.inner.idle.lock().await.len()
    }

    /// Returns `true` if the idle queue is empty.
    pub async fn is_empty(&self) -> bool {
        self.inner.idle.lock().await.is_empty()
    }

    /// The configured capacity cap, or `None` if unbounded.
    pub fn capacity(&self) -> Option<usize> {
        self.capacity
    }

    /// Drains all idle entries from the queue without running any hooks.
    ///
    /// Returns all entries collected without closing the store; subsequent
    /// deposits remain possible. Terminal shutdown uses a separate fenced drain.
    pub async fn drain_all(&self) -> Vec<S> {
        self.inner
            .idle
            .lock()
            .await
            .drain(..)
            .map(|e| e.entry)
            .collect()
    }

    /// Evicts all idle entries whose checkout epoch is behind the live counter.
    ///
    /// Returns the evicted entries so the caller can destroy them. Used by the
    /// background maintenance reaper. The revoke-epoch fence now runs on
    /// **all three** return-to-pool directions — [`checkout`](Self::checkout)
    /// (on pop), [`return_entry`](Self::return_entry) (on push), and this
    /// reaper sweep — so a stale entry can never be served regardless of which
    /// path observes it first.
    pub async fn evict_stale(&self) -> Vec<S> {
        let mut idle = self.inner.idle.lock().await;
        // Epoch read under the idle lock — the same discipline as `checkout` /
        // `return_entry` — so a revoke racing this sweep is either fully
        // observed (its entries evicted now) or fully deferred to the next
        // fence crossing, never half-applied against a pre-lock snapshot.
        let live_epoch = self.current_revoke_epoch();
        let mut evicted = Vec::new();
        let mut keep = VecDeque::with_capacity(idle.len());
        for item in idle.drain(..) {
            if item.checkout_epoch == live_epoch {
                keep.push_back(item);
            } else {
                evicted.push(item.entry);
            }
        }
        *idle = keep;
        evicted
    }

    /// Stamps an entry with the current epoch for returning to the store.
    ///
    /// Call this when a newly-created entry is being prepared for its first
    /// deposit into the idle queue. The epoch is captured at call time so
    /// a revoke that lands between entry creation and first checkout is
    /// detected on the `return_entry` path.
    pub fn stamp_epoch(&self) -> u64 {
        self.current_revoke_epoch()
    }

    /// Locks the idle queue and returns the guard for in-place iteration.
    ///
    /// Crate-internal: the built-in
    /// [`Pooled`](crate::topology::Pooled) rotation fan-out holds this
    /// guard across **every** `&R::Instance` credential hook `.await` so no
    /// checkout / return can interleave mid-rotation — the same lock
    /// [`checkout`](Self::checkout) / [`return_entry`](Self::return_entry) take.
    /// Author topologies cannot name this crate-private guard; only the
    /// framework can directly lock and mutate the queue.
    ///
    /// Holding this guard across an `.await` is a deliberate head-of-line
    /// block: rotation is rare and the alternative (drop-and-reacquire between
    /// entries) reopens the window for an entry to be checked out mid-rotation
    /// and miss its hook (a credential-isolation violation). Do not widen the
    /// unlocked window.
    pub(crate) async fn lock_idle(&self) -> MutexGuard<'_, VecDeque<StoreEntry<S>>> {
        self.inner.idle.lock().await
    }

    /// Removes and returns every idle entry for which `should_evict` is `true`,
    /// keeping the rest in original order.
    ///
    /// The eviction predicate is evaluated under the idle lock, atomic against
    /// concurrent checkout/return. Used by the background maintenance reaper
    /// for the fingerprint / max-lifetime / idle-timeout arms; the
    /// revoke-epoch arm runs through [`evict_stale`](Self::evict_stale).
    ///
    /// Complexity: O(n) over the idle queue (average and worst case), bounded
    /// by the configured idle capacity.
    pub(crate) async fn retain<F>(&self, mut should_evict: F) -> Vec<S>
    where
        F: FnMut(&S, u64) -> bool,
    {
        let mut idle = self.inner.idle.lock().await;
        let mut evicted = Vec::new();
        let mut keep = VecDeque::with_capacity(idle.len());
        for item in idle.drain(..) {
            if should_evict(&item.entry, item.checkout_epoch) {
                evicted.push(item.entry);
            } else {
                keep.push_back(item);
            }
        }
        *idle = keep;
        evicted
    }

    /// Deposits a freshly-created entry into the idle queue, stamping it with
    /// the live revoke epoch **under the idle lock**, fenced against a
    /// concurrent revoke.
    ///
    /// This is the first-deposit counterpart to [`return_entry`](Self::return_entry):
    /// an entry whose creation straddled a revoke is stamped with the live
    /// counter so a revoke that already landed evicts it immediately
    /// ([`ReturnOutcome::Evict`]); otherwise it is queued (capacity
    /// permitting). The `created_epoch` is the snapshot taken at the *start*
    /// of creation; if it is already behind the live counter the entry was
    /// built against a since-revoked credential and is rejected. A terminal
    /// store rejects the entry even when its credential epoch is current.
    ///
    /// # Fence guarantee
    ///
    /// The epoch read and the push happen under the idle lock — the same lock
    /// the revoke idle-walk holds — so the compare-then-push is atomic against
    /// a concurrent `bump_revoke_epoch` + reaper sweep.
    ///
    /// # Cancel safety
    ///
    /// Same contract as [`return_entry`](Self::return_entry): the store is
    /// never left half-mutated, but the future owns `entry` while awaiting the
    /// lock. Cancellation-guarded callers should use the crate-internal
    /// `lock_idle` + `deposit_fresh_locked` split so their destroy-on-drop
    /// guard stays armed across the lock acquisition — the warmup loop does
    /// exactly this.
    pub async fn deposit_fresh(&self, entry: S, created_epoch: u64) -> ReturnOutcome<S> {
        let mut idle = self.inner.idle.lock().await;
        self.deposit_fresh_locked(&mut idle, entry, created_epoch)
    }

    /// The synchronous core of [`deposit_fresh`](Self::deposit_fresh),
    /// against an already-held idle lock.
    ///
    /// Split out for cancellation-guarded callers (the warmup loop): they
    /// acquire the lock via [`lock_idle`](Self::lock_idle) while the entry is
    /// still armed in its cancel guard, then defuse and hand the entry over
    /// only once no await remains — so a caller cancellation can never drop
    /// a created-but-undeposited instance through a plain `Drop`.
    pub(crate) fn deposit_fresh_locked(
        &self,
        idle: &mut VecDeque<StoreEntry<S>>,
        entry: S,
        created_epoch: u64,
    ) -> ReturnOutcome<S> {
        if self.is_closed() {
            debug!("InstanceStore::deposit_fresh: terminal fence — rejecting fresh entry");
            return ReturnOutcome::Evict(entry);
        }
        let live_epoch = self.inner.revoke_epoch.load(Ordering::Acquire);
        if created_epoch != live_epoch {
            debug!(
                created_epoch,
                live_epoch, "InstanceStore::deposit_fresh: epoch mismatch — rejecting fresh entry"
            );
            return ReturnOutcome::Evict(entry);
        }
        if let Some(cap) = self.capacity
            && idle.len() >= cap
        {
            return ReturnOutcome::Evict(entry);
        }
        self.enqueue(
            idle,
            StoreEntry {
                entry,
                checkout_epoch: created_epoch,
            },
        );
        ReturnOutcome::Recycled
    }
}

// ─── Checkout ─────────────────────────────────────────────────────────────────

/// The outcome of [`InstanceStore::checkout`].
///
/// Carries the first **fresh** idle entry (if any) plus every **stale** entry
/// the fence discarded on the way to it. The framework acquire pipeline must
/// destroy each `stale` entry via [`Provider::destroy`] before leasing
/// `fresh`: an entry whose checkout epoch is behind the live revoke counter was
/// leased under a since-revoked credential and must never be re-handed-out
/// nor silently leaked.
///
/// [`Provider::destroy`]: crate::resource::Provider::destroy
#[must_use = "Checkout contains entries that must be processed (fresh used, stale destroyed)"]
pub struct Checkout<S> {
    /// The first idle entry whose checkout epoch is current, or `None` if the
    /// idle queue held no fresh entry.
    pub fresh: Option<CheckedOut<S>>,
    /// Idle entries fenced by credential revocation or terminal retirement.
    ///
    /// The framework destroys them and never returns them to a caller.
    pub stale: Vec<S>,
}

impl<S> std::fmt::Debug for Checkout<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Checkout")
            .field("has_fresh", &self.fresh.is_some())
            .field("stale_count", &self.stale.len())
            .finish()
    }
}

// ─── CheckedOut ───────────────────────────────────────────────────────────────

/// An entry that has been checked out of the [`InstanceStore`].
///
/// Carries the entry value and the epoch at checkout time so that
/// `return_entry` can run the revoke-fence check. Topology implementations
/// receive this from [`InstanceStore::checkout`] via [`Checkout::fresh`].
pub struct CheckedOut<S> {
    /// The leased entry.
    pub entry: S,
    /// Epoch captured at checkout time.
    pub(crate) checkout_epoch: u64,
}

impl<S> std::fmt::Debug for CheckedOut<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CheckedOut")
            .field("checkout_epoch", &self.checkout_epoch)
            .finish()
    }
}

impl<S> CheckedOut<S> {
    /// Consumes the `CheckedOut`, returning the entry and the checkout epoch
    /// for passing to [`InstanceStore::return_entry`].
    #[must_use]
    pub fn into_parts(self) -> (S, u64) {
        (self.entry, self.checkout_epoch)
    }
}

// ─── ReturnOutcome ─────────────────────────────────────────────────────────────

/// The outcome of [`InstanceStore::return_entry`] / [`InstanceStore::deposit_fresh`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReturnOutcome<S> {
    /// The entry was returned to the idle queue — it is clean and ready to
    /// be leased again.
    Recycled,
    /// The entry was NOT returned because its checkout epoch is behind the
    /// live revoke counter, the store is closing, or the capacity cap was reached. The entry is
    /// handed back for the caller to destroy.
    Evict(S),
}

impl<S> ReturnOutcome<S> {
    /// Returns `true` if the entry was evicted and must be destroyed.
    pub fn is_evict(&self) -> bool {
        matches!(self, Self::Evict(_))
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
