//! Framework-owned instance storage for resource topologies.
//!
//! [`InstanceStore<S>`] is the framework-controlled holder for leased instances
//! that [`Topology`] implementations borrow but cannot retain. It carries the
//! idle queue, the generation/revoke-epoch state, and the uniform revoke-epoch
//! fence that runs on every `return_entry` path — for both built-in and custom
//! topologies.
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
//! [`Topology`]: crate::topology::Topology
//! [`Topology::Entry`]: crate::topology::Topology::Entry

use std::{
    collections::VecDeque,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use tokio::sync::{Mutex, MutexGuard};
use tracing::debug;

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

/// Framework-owned idle queue and revoke-epoch state for a
/// [`Topology`](crate::topology::Topology)'s entries.
///
/// An `InstanceStore<S>` is the storage the [`Manager`] owns; a
/// [`Topology`](crate::topology::Topology) implementation receives a borrowed
/// `&InstanceStore<Self::Entry>` in [`try_reserve`] /
/// [`on_release`] and [`phase`] / [`load`] but **cannot retain it** (it is a
/// `&` reference, not an `Arc`). This makes it structurally impossible for an
/// author topology to build a cross-scope instance cache that bypasses the
/// per-tenant `SlotIdentity` fence.
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
    /// Framework-held idle queue; entries are `(S, checkout_epoch)` pairs.
    idle: Arc<Mutex<VecDeque<StoreEntry<S>>>>,
    // `Clone` (below) is hand-written, not derived: it shares the same `Arc`
    // backing — a cloned handle returns entries into the *same* idle queue and
    // observes the *same* revoke counter. This is what lets the release
    // closure hold a cloned `InstanceStore` and recycle into the live store.
    /// Monotonic credential-revoke counter. Bumped synchronously by the
    /// manager on credential revoke — before any async revoke hook dispatch.
    /// Every `return_entry` compares the entry's checkout epoch against this;
    /// an advanced counter evicts the entry instead of re-queuing it.
    revoke_epoch: Arc<AtomicU64>,
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
            idle: Arc::clone(&self.idle),
            revoke_epoch: Arc::clone(&self.revoke_epoch),
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
            .field("revoke_epoch", &self.revoke_epoch.load(Ordering::Acquire))
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
        Self {
            idle: Arc::new(Mutex::new(VecDeque::new())),
            revoke_epoch: Arc::new(AtomicU64::new(0)),
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
        self.revoke_epoch.load(Ordering::Acquire)
    }

    /// Advances the revoke epoch by one.
    ///
    /// Called synchronously by the framework when a credential bound to this
    /// store's resource is revoked — before the revoke hook is dispatched.
    /// After this call every subsequent `return_entry` will evict any entry
    /// whose `checkout_epoch` is behind the new counter.
    pub fn bump_revoke_epoch(&self) {
        self.revoke_epoch.fetch_add(1, Ordering::Release);
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
    /// `fresh` is `None`.
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
        let mut idle = self.idle.lock().await;
        // Revoke-epoch fence: read under the idle lock (same lock the
        // credential-revoke idle-walk holds) so the epoch snapshot is
        // atomic against a concurrent `bump_revoke_epoch`. Without this,
        // a revoke landing between the snapshot and the lock acquire would
        // let a stale entry escape as `fresh`.
        let live_epoch = self.current_revoke_epoch();
        let mut stale = Vec::new();
        while let Some(item) = idle.pop_front() {
            if item.checkout_epoch != live_epoch {
                // Leased under a since-revoked credential — never hand out.
                debug!(
                    checkout_epoch = item.checkout_epoch,
                    live_epoch, "InstanceStore::checkout: epoch mismatch — discarding stale entry"
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
    /// reached. Otherwise the entry is enqueued and [`ReturnOutcome::Recycled`]
    /// is returned.
    ///
    /// Returning the evicted entry (rather than swallowing it) lets the
    /// topology drive async eviction (e.g. calling `Provider::destroy`)
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
        let mut idle = self.idle.lock().await;
        // Revoke-epoch fence: re-read under the idle lock (same lock the
        // credential-revoke idle-walk holds) to make compare-then-push
        // atomic against a concurrent revoke.
        let live_epoch = self.revoke_epoch.load(Ordering::Acquire);
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
            &mut idle,
            StoreEntry {
                entry,
                checkout_epoch,
            },
        );
        ReturnOutcome::Recycled
    }

    /// Number of idle entries currently in the queue.
    pub async fn len(&self) -> usize {
        self.idle.lock().await.len()
    }

    /// Returns `true` if the idle queue is empty.
    pub async fn is_empty(&self) -> bool {
        self.idle.lock().await.is_empty()
    }

    /// The configured capacity cap, or `None` if unbounded.
    pub fn capacity(&self) -> Option<usize> {
        self.capacity
    }

    /// Drains all idle entries from the queue without running any hooks.
    ///
    /// Used by the framework during drain/shutdown to empty the store so
    /// entries can be destroyed by the caller. Returns all entries collected.
    pub async fn drain_all(&self) -> Vec<S> {
        self.idle.lock().await.drain(..).map(|e| e.entry).collect()
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
        let mut idle = self.idle.lock().await;
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
    /// Author topologies receive a `&InstanceStore` and can never name the
    /// guard, so the "cannot retain the store" rule still holds: only the
    /// framework can lock the queue, never the author.
    ///
    /// Holding this guard across an `.await` is a deliberate head-of-line
    /// block: rotation is rare and the alternative (drop-and-reacquire between
    /// entries) reopens the window for an entry to be checked out mid-rotation
    /// and miss its hook (a credential-isolation violation). Do not widen the
    /// unlocked window.
    pub(crate) async fn lock_idle(&self) -> MutexGuard<'_, VecDeque<StoreEntry<S>>> {
        self.idle.lock().await
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
        let mut idle = self.idle.lock().await;
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
    /// built against a since-revoked credential and is rejected.
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
        let mut idle = self.idle.lock().await;
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
        let live_epoch = self.revoke_epoch.load(Ordering::Acquire);
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
    /// Idle entries whose checkout epoch was behind the live revoke counter.
    ///
    /// These were leased under a since-revoked credential; the framework
    /// destroys them and never returns them to a caller.
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
    /// live revoke counter, or the capacity cap was reached. The entry is
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
mod tests {
    use super::*;

    // Entry returned before epoch bump → Recycled (re-pooled).
    #[tokio::test]
    async fn return_entry_current_epoch_is_recycled() {
        let store: InstanceStore<u32> = InstanceStore::new(Some(4));
        let epoch = store.stamp_epoch();
        let outcome = store.return_entry(42u32, epoch).await;
        assert_eq!(outcome, ReturnOutcome::Recycled);
        assert_eq!(store.len().await, 1);
    }

    // Entry returned AFTER epoch bump → Evict (revoke fence triggered).
    #[tokio::test]
    async fn return_entry_after_epoch_bump_is_evicted() {
        let store: InstanceStore<u32> = InstanceStore::new(Some(4));
        // Stamp the epoch BEFORE the bump (simulate checkout epoch).
        let checkout_epoch = store.stamp_epoch();
        // Simulate a credential revoke.
        store.bump_revoke_epoch();
        // Now return — the checkout_epoch is behind the live counter.
        let outcome = store.return_entry(42u32, checkout_epoch).await;
        assert!(
            outcome.is_evict(),
            "an entry checked out before a revoke must be evicted, not re-pooled"
        );
        assert_eq!(
            store.len().await,
            0,
            "evicted entry must not appear in the idle queue"
        );
    }

    // Multiple bumps: any advance evicts.
    #[tokio::test]
    async fn return_entry_multiple_epoch_bumps_evicts() {
        let store: InstanceStore<u32> = InstanceStore::new(None);
        let checkout_epoch = store.stamp_epoch();
        store.bump_revoke_epoch();
        store.bump_revoke_epoch();
        let outcome = store.return_entry(99u32, checkout_epoch).await;
        assert!(outcome.is_evict());
    }

    // Entry returned at the same epoch after a bump is recycled.
    #[tokio::test]
    async fn return_entry_same_epoch_after_bump_is_recycled() {
        let store: InstanceStore<u32> = InstanceStore::new(None);
        store.bump_revoke_epoch();
        // Stamp the epoch AFTER the bump → checkout_epoch == live_epoch.
        let checkout_epoch = store.stamp_epoch();
        let outcome = store.return_entry(7u32, checkout_epoch).await;
        assert_eq!(outcome, ReturnOutcome::Recycled);
        assert_eq!(store.len().await, 1);
    }

    // Checkout-return round trip preserves the entry value.
    #[tokio::test]
    async fn checkout_return_roundtrip() {
        let store: InstanceStore<String> = InstanceStore::new(None);
        let epoch = store.stamp_epoch();
        store.return_entry("hello".to_owned(), epoch).await;
        let checkout = store.checkout().await;
        assert!(
            checkout.stale.is_empty(),
            "no stale entries on a clean queue"
        );
        let fresh_entry = checkout.fresh.map(|c| c.entry);
        assert_eq!(fresh_entry.as_deref(), Some("hello"));
        assert_eq!(store.len().await, 0);
    }

    // C1 fence-on-checkout (a): an entry that went idle, then had its credential
    // revoked, must land in `stale` — never `fresh`.
    #[tokio::test]
    async fn checkout_evicts_entry_revoked_while_idle() {
        let store: InstanceStore<u32> = InstanceStore::new(Some(4));
        // Entry goes idle at epoch 0.
        let epoch = store.stamp_epoch();
        store.return_entry(42u32, epoch).await;
        // Credential revoked while it sat idle.
        store.bump_revoke_epoch();

        let checkout = store.checkout().await;
        assert!(
            checkout.fresh.is_none(),
            "an entry revoked while idle must never be handed out as fresh"
        );
        assert_eq!(
            checkout.stale,
            vec![42u32],
            "the since-revoked entry must be collected for destruction"
        );
        assert_eq!(store.len().await, 0, "the idle queue is drained");
    }

    // C1 fence-on-checkout (b): a mix of stale and fresh entries returns only
    // the first fresh one, with every stale entry collected.
    #[tokio::test]
    async fn checkout_returns_fresh_after_collecting_stale() {
        let store: InstanceStore<u32> = InstanceStore::new(None);
        // Two entries go idle at epoch 0.
        let old_epoch = store.stamp_epoch();
        store.return_entry(1u32, old_epoch).await;
        store.return_entry(2u32, old_epoch).await;
        // Revoke — both are now stale.
        store.bump_revoke_epoch();
        // A fresh entry is returned at the new epoch and queued at the back.
        let new_epoch = store.stamp_epoch();
        store.return_entry(3u32, new_epoch).await;

        let checkout = store.checkout().await;
        assert_eq!(
            checkout.stale,
            vec![1u32, 2u32],
            "both pre-revoke entries are collected as stale, in FIFO order"
        );
        assert_eq!(
            checkout.fresh.map(|c| c.entry),
            Some(3u32),
            "only the current-epoch entry is fresh"
        );
        assert_eq!(store.len().await, 0, "the idle queue is now drained");
    }

    // C1 fence-on-checkout (c): an empty queue returns no fresh and no stale.
    #[tokio::test]
    async fn checkout_empty_queue_returns_none() {
        let store: InstanceStore<u32> = InstanceStore::new(None);
        let checkout = store.checkout().await;
        assert!(checkout.fresh.is_none());
        assert!(checkout.stale.is_empty());
    }

    // evict_stale removes only entries with stale epoch.
    #[tokio::test]
    async fn evict_stale_removes_old_epoch_entries() {
        let store: InstanceStore<u32> = InstanceStore::new(None);
        // Push an entry with the initial epoch.
        let old_epoch = store.stamp_epoch();
        store.return_entry(1u32, old_epoch).await;
        // Bump epoch — the entry is now stale.
        store.bump_revoke_epoch();
        // Push a fresh entry with the new epoch.
        let new_epoch = store.stamp_epoch();
        store.return_entry(2u32, new_epoch).await;
        // Evict stale.
        let evicted = store.evict_stale().await;
        assert_eq!(evicted, vec![1u32], "only the pre-bump entry is evicted");
        assert_eq!(store.len().await, 1, "fresh entry remains");
    }

    // capacity cap: return beyond capacity is evicted.
    #[tokio::test]
    async fn capacity_cap_evicts_overflow() {
        let store: InstanceStore<u32> = InstanceStore::new(Some(2));
        let epoch = store.stamp_epoch();
        assert_eq!(store.return_entry(1, epoch).await, ReturnOutcome::Recycled);
        assert_eq!(store.return_entry(2, epoch).await, ReturnOutcome::Recycled);
        assert!(
            store.return_entry(3, epoch).await.is_evict(),
            "third entry exceeds cap of 2 → evicted"
        );
        assert_eq!(store.len().await, 2);
    }

    // Default FIFO: checkout order matches return order (even wear).
    #[tokio::test]
    async fn fifo_default_checks_out_in_return_order() {
        let store: InstanceStore<u32> = InstanceStore::new(None);
        assert_eq!(store.strategy(), PoolStrategy::Fifo, "new() defaults FIFO");
        let epoch = store.stamp_epoch();
        store.return_entry(1, epoch).await;
        store.return_entry(2, epoch).await;
        let first = store.checkout().await.fresh.map(|c| c.entry);
        assert_eq!(first, Some(1), "FIFO hands out the oldest return first");
    }

    // LIFO: the most recently returned entry is reused first (hot-set reuse,
    // the tail ages out for the idle_timeout reaper).
    #[tokio::test]
    async fn lifo_checks_out_most_recent_return_first() {
        let store: InstanceStore<u32> = InstanceStore::new(None).with_strategy(PoolStrategy::Lifo);
        let epoch = store.stamp_epoch();
        store.return_entry(1, epoch).await;
        store.return_entry(2, epoch).await;
        let first = store.checkout().await.fresh.map(|c| c.entry);
        assert_eq!(first, Some(2), "LIFO hands out the hottest entry first");
        let second = store.checkout().await.fresh.map(|c| c.entry);
        assert_eq!(second, Some(1), "the colder entry is next");
    }

    // LIFO first-deposit: deposit_fresh honors the same push side.
    #[tokio::test]
    async fn lifo_deposit_fresh_lands_at_the_front() {
        let store: InstanceStore<u32> = InstanceStore::new(None).with_strategy(PoolStrategy::Lifo);
        let epoch = store.stamp_epoch();
        store.deposit_fresh(1, epoch).await;
        store.deposit_fresh(2, epoch).await;
        let first = store.checkout().await.fresh.map(|c| c.entry);
        assert_eq!(first, Some(2), "LIFO deposits land at the checkout end");
    }

    // A cloned handle shares queue AND strategy.
    #[tokio::test]
    async fn clone_preserves_strategy() {
        let store: InstanceStore<u32> = InstanceStore::new(None).with_strategy(PoolStrategy::Lifo);
        let cloned = store.clone();
        assert_eq!(cloned.strategy(), store.strategy());
        assert_eq!(cloned.strategy(), PoolStrategy::Lifo);
    }

    // drain_all empties the queue.
    #[tokio::test]
    async fn drain_all_empties_store() {
        let store: InstanceStore<u32> = InstanceStore::new(None);
        let epoch = store.stamp_epoch();
        store.return_entry(10, epoch).await;
        store.return_entry(20, epoch).await;
        let drained = store.drain_all().await;
        assert_eq!(drained.len(), 2);
        assert!(store.is_empty().await);
    }
}
