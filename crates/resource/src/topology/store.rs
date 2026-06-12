//! Framework-owned instance storage for resource topologies.
//!
//! [`InstanceStore<S>`] is the framework-controlled holder for leased instances
//! that [`Topology`] implementations borrow but cannot retain. It carries the
//! idle queue, the generation/revoke-epoch state, and the uniform revoke-epoch
//! fence that runs on every `return_slot` path — for both built-in and custom
//! topologies.
//!
//! [`Topology`]: crate::topology::Topology

use std::{
    collections::VecDeque,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use tokio::sync::{Mutex, MutexGuard};
use tracing::debug;

// ─── InstanceStore ────────────────────────────────────────────────────────────

/// A timestamped slot entry: the slot value plus the revoke-epoch snapshot
/// taken when the slot was checked out.
///
/// The epoch is captured at checkout time so a `return_slot` after a
/// `bump_revoke_epoch()` detects the stale epoch and evicts rather than
/// re-pooling.
///
/// `pub(crate)` so the built-in [`Pooled`](crate::topology::pooled::Pooled)
/// pipeline can iterate the idle queue under [`InstanceStore::lock_idle`] and
/// read each entry's `.slot` during rotation fan-out without copying it out of
/// the store. The fields stay crate-visible only — author topologies receive a
/// `&InstanceStore` and never name `StoreEntry`.
pub(crate) struct StoreEntry<S> {
    pub(crate) slot: S,
    /// The store's revoke-epoch as observed when this slot was **checked out**
    /// (via [`InstanceStore::checkout`] → stamps with the live counter).
    pub(crate) checkout_epoch: u64,
}

/// Framework-owned idle queue and revoke-epoch state for a
/// [`Topology`](crate::topology::Topology)'s slots.
///
/// An `InstanceStore<S>` is the storage the [`Manager`] owns; a
/// [`Topology`](crate::topology::Topology) implementation receives a borrowed
/// `&InstanceStore<Self::Slot>` in [`try_reserve`] /
/// [`on_release`] and [`phase`] / [`load`] but **cannot retain it** (it is a
/// `&` reference, not an `Arc`). This makes it structurally impossible for an
/// author topology to build a cross-scope instance cache that bypasses the
/// per-tenant `SlotIdentity` fence.
///
/// # Revoke-epoch fence
///
/// The fence is uniform: every slot returned via [`return_slot`] is checked
/// against the live epoch (loaded with `Acquire` ordering); a slot whose
/// checkout epoch is *behind* the live counter was leased under a since-revoked
/// credential and is **evicted** (not re-pooled). [`bump_revoke_epoch`] is
/// called by the framework synchronously when a credential is revoked —
/// exactly as `PoolRuntime::bump_revoke_epoch` is called today.
///
/// [`Manager`]: crate::Manager
/// [`try_reserve`]: crate::topology::Topology::try_reserve
/// [`on_release`]: crate::topology::Topology::on_release
/// [`phase`]: crate::topology::Topology::phase
/// [`load`]: crate::topology::Topology::load
/// [`return_slot`]: InstanceStore::return_slot
/// [`bump_revoke_epoch`]: InstanceStore::bump_revoke_epoch
pub struct InstanceStore<S> {
    /// Framework-held idle queue; slots are `(S, checkout_epoch)` pairs.
    idle: Arc<Mutex<VecDeque<StoreEntry<S>>>>,
    // `Clone` (below) is hand-written, not derived: it shares the same `Arc`
    // backing — a cloned handle returns slots into the *same* idle queue and
    // observes the *same* revoke counter. This is what lets the release
    // closure hold a cloned `InstanceStore` and recycle into the live store.
    /// Monotonic credential-revoke counter. Bumped synchronously by the
    /// manager on credential revoke — before any async revoke hook dispatch.
    /// Every `return_slot` compares the slot's checkout epoch against this;
    /// an advanced counter evicts the slot instead of re-queuing it.
    revoke_epoch: Arc<AtomicU64>,
    /// Maximum number of slots the store will hold idle.
    /// `None` = unbounded (Resident / permit-only topologies).
    capacity: Option<usize>,
}

impl<S> Clone for InstanceStore<S> {
    fn clone(&self) -> Self {
        Self {
            idle: Arc::clone(&self.idle),
            revoke_epoch: Arc::clone(&self.revoke_epoch),
            capacity: self.capacity,
        }
    }
}

impl<S: Send + 'static> InstanceStore<S> {
    /// Creates a new store with an optional idle capacity cap.
    ///
    /// Pass `None` for unbounded (e.g., Resident or permit-only topologies);
    /// pass `Some(n)` for Pooled-like topologies that cap the idle queue.
    pub fn new(capacity: Option<usize>) -> Self {
        Self {
            idle: Arc::new(Mutex::new(VecDeque::new())),
            revoke_epoch: Arc::new(AtomicU64::new(0)),
            capacity,
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
    /// After this call every subsequent `return_slot` will evict any slot
    /// whose `checkout_epoch` is behind the new counter.
    pub fn bump_revoke_epoch(&self) {
        self.revoke_epoch.fetch_add(1, Ordering::Release);
    }

    /// Checks out the first **fresh** idle slot, running the revoke-epoch
    /// fence on pop (framework-owned).
    ///
    /// The fence runs on **both** directions now (checkout and return): an
    /// idle slot whose `checkout_epoch` is behind the live revoke counter was
    /// leased under a since-revoked credential and must **never** be handed
    /// out again. This method pops idle entries under the idle lock; any entry
    /// whose epoch is stale is collected into [`Checkout::stale`] (for the
    /// framework to destroy via [`Provider::destroy`]) and is **never**
    /// returned as fresh. The first entry whose epoch is current is returned
    /// as [`Checkout::fresh`]; if the queue drains without a fresh slot,
    /// `fresh` is `None`.
    ///
    /// The framework acquire pipeline destroys every slot in `stale` before
    /// using `fresh`. The store cannot call `Provider::destroy` itself (it
    /// holds no `Provider`), so it returns the stale slots to the caller for
    /// destruction.
    ///
    /// # Fence guarantee
    ///
    /// The epoch comparison and the pop are performed while holding the idle
    /// lock — the same lock the credential-revoke idle-walk
    /// ([`evict_stale`](Self::evict_stale)) holds — so a slot revoked while
    /// idle is observed as stale here even if the revoke raced the checkout.
    ///
    /// [`Provider::destroy`]: crate::resource::Provider::destroy
    pub async fn checkout(&self) -> Checkout<S> {
        let live_epoch = self.current_revoke_epoch();
        let mut idle = self.idle.lock().await;
        let mut stale = Vec::new();
        while let Some(entry) = idle.pop_front() {
            if entry.checkout_epoch != live_epoch {
                // Leased under a since-revoked credential — never hand out.
                debug!(
                    checkout_epoch = entry.checkout_epoch,
                    live_epoch, "InstanceStore::checkout: epoch mismatch — discarding stale slot"
                );
                stale.push(entry.slot);
                continue;
            }
            return Checkout {
                fresh: Some(CheckedOut {
                    slot: entry.slot,
                    checkout_epoch: entry.checkout_epoch,
                    store_epoch: live_epoch,
                }),
                stale,
            };
        }
        Checkout { fresh: None, stale }
    }

    /// Returns a slot to the idle queue, running the revoke-epoch fence.
    ///
    /// If the slot's `checkout_epoch` is behind the live revoke counter, the
    /// slot was leased under a since-revoked credential and is **not**
    /// re-queued — it is handed back via [`ReturnOutcome::Evict`] for the
    /// caller to destroy. Same when the optional capacity cap is already
    /// reached. Otherwise the slot is enqueued and [`ReturnOutcome::Recycled`]
    /// is returned.
    ///
    /// Returning the evicted slot (rather than swallowing it) lets the
    /// topology drive async eviction (e.g. calling `Provider::destroy`)
    /// without the store owning `Provider`.
    ///
    /// # Fence guarantee
    ///
    /// The epoch re-read and the push are performed while holding the idle
    /// lock, so a concurrent `bump_revoke_epoch` followed by an idle-walk
    /// cannot enqueue a stale slot: the walk holds the same lock and sees the
    /// already-bumped counter.
    pub async fn return_slot(&self, slot: S, checkout_epoch: u64) -> ReturnOutcome<S> {
        let mut idle = self.idle.lock().await;
        // Revoke-epoch fence: re-read under the idle lock (same lock the
        // credential-revoke idle-walk holds) to make compare-then-push
        // atomic against a concurrent revoke.
        let live_epoch = self.revoke_epoch.load(Ordering::Acquire);
        if checkout_epoch != live_epoch {
            // Slot was leased under a since-revoked credential — evict.
            debug!(
                checkout_epoch,
                live_epoch, "InstanceStore::return_slot: epoch mismatch — evicting slot"
            );
            return ReturnOutcome::Evict(slot);
        }
        // Capacity check.
        if let Some(cap) = self.capacity
            && idle.len() >= cap
        {
            return ReturnOutcome::Evict(slot);
        }
        idle.push_back(StoreEntry {
            slot,
            checkout_epoch,
        });
        ReturnOutcome::Recycled
    }

    /// Number of idle slots currently in the queue.
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

    /// Drains all idle slots from the queue without running any hooks.
    ///
    /// Used by the framework during drain/shutdown to empty the store so
    /// slots can be destroyed by the caller. Returns all slots collected.
    pub async fn drain_all(&self) -> Vec<S> {
        self.idle.lock().await.drain(..).map(|e| e.slot).collect()
    }

    /// Evicts all idle slots whose checkout epoch is behind the live counter.
    ///
    /// Returns the evicted slots so the caller can destroy them. Used by the
    /// background maintenance reaper. The revoke-epoch fence now runs on
    /// **all three** return-to-pool directions — [`checkout`](Self::checkout)
    /// (on pop), [`return_slot`](Self::return_slot) (on push), and this
    /// reaper sweep — so a stale slot can never be served regardless of which
    /// path observes it first.
    pub async fn evict_stale(&self) -> Vec<S> {
        let live_epoch = self.current_revoke_epoch();
        let mut idle = self.idle.lock().await;
        let mut evicted = Vec::new();
        let mut keep = VecDeque::with_capacity(idle.len());
        for entry in idle.drain(..) {
            if entry.checkout_epoch == live_epoch {
                keep.push_back(entry);
            } else {
                evicted.push(entry.slot);
            }
        }
        *idle = keep;
        evicted
    }

    /// Stamps a slot with the current epoch for returning to the store.
    ///
    /// Call this when a newly-created slot is being prepared for its first
    /// deposit into the idle queue. The epoch is captured at call time so
    /// a revoke that lands between slot creation and first checkout is
    /// detected on the `return_slot` path.
    pub fn stamp_epoch(&self) -> u64 {
        self.current_revoke_epoch()
    }

    /// Locks the idle queue and returns the guard for in-place iteration.
    ///
    /// Crate-internal: the built-in
    /// [`Pooled`](crate::topology::pooled::Pooled) rotation fan-out holds this
    /// guard across **every** `&R::Instance` credential hook `.await` so no
    /// checkout / return can interleave mid-rotation — the same lock
    /// [`checkout`](Self::checkout) / [`return_slot`](Self::return_slot) take.
    /// Author topologies receive a `&InstanceStore` and can never name the
    /// guard, so the "cannot retain the store" rule still holds: only the
    /// framework can lock the queue, never the author.
    ///
    /// Holding this guard across an `.await` is a deliberate head-of-line
    /// block: rotation is rare and the alternative (drop-and-reacquire between
    /// entries) reopens the window for a slot to be checked out mid-rotation
    /// and miss its hook (a credential-isolation violation). Do not widen the
    /// unlocked window.
    pub(crate) async fn lock_idle(&self) -> MutexGuard<'_, VecDeque<StoreEntry<S>>> {
        self.idle.lock().await
    }

    /// Removes and returns every idle slot for which `should_evict` is `true`,
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
        for entry in idle.drain(..) {
            if should_evict(&entry.slot, entry.checkout_epoch) {
                evicted.push(entry.slot);
            } else {
                keep.push_back(entry);
            }
        }
        *idle = keep;
        evicted
    }

    /// Deposits a freshly-created slot into the idle queue, stamping it with
    /// the live revoke epoch **under the idle lock**, fenced against a
    /// concurrent revoke.
    ///
    /// This is the first-deposit counterpart to [`return_slot`](Self::return_slot):
    /// a slot whose creation straddled a revoke is stamped with the live
    /// counter so a revoke that already landed evicts it immediately
    /// ([`ReturnOutcome::Evict`]); otherwise it is queued (capacity
    /// permitting). The `created_epoch` is the snapshot taken at the *start*
    /// of creation; if it is already behind the live counter the slot was
    /// built against a since-revoked credential and is rejected.
    ///
    /// # Fence guarantee
    ///
    /// The epoch read and the push happen under the idle lock — the same lock
    /// the revoke idle-walk holds — so the compare-then-push is atomic against
    /// a concurrent `bump_revoke_epoch` + reaper sweep.
    pub async fn deposit_fresh(&self, slot: S, created_epoch: u64) -> ReturnOutcome<S> {
        let mut idle = self.idle.lock().await;
        let live_epoch = self.revoke_epoch.load(Ordering::Acquire);
        if created_epoch != live_epoch {
            debug!(
                created_epoch,
                live_epoch, "InstanceStore::deposit_fresh: epoch mismatch — rejecting fresh slot"
            );
            return ReturnOutcome::Evict(slot);
        }
        if let Some(cap) = self.capacity
            && idle.len() >= cap
        {
            return ReturnOutcome::Evict(slot);
        }
        idle.push_back(StoreEntry {
            slot,
            checkout_epoch: created_epoch,
        });
        ReturnOutcome::Recycled
    }
}

// ─── Checkout ─────────────────────────────────────────────────────────────────

/// The outcome of [`InstanceStore::checkout`].
///
/// Carries the first **fresh** idle slot (if any) plus every **stale** slot
/// the fence discarded on the way to it. The framework acquire pipeline must
/// destroy each `stale` slot via [`Provider::destroy`] before leasing
/// `fresh`: a slot whose checkout epoch is behind the live revoke counter was
/// leased under a since-revoked credential and must never be re-handed-out
/// nor silently leaked.
///
/// [`Provider::destroy`]: crate::resource::Provider::destroy
pub struct Checkout<S> {
    /// The first idle slot whose checkout epoch is current, or `None` if the
    /// idle queue held no fresh slot.
    pub fresh: Option<CheckedOut<S>>,
    /// Idle slots whose checkout epoch was behind the live revoke counter.
    ///
    /// These were leased under a since-revoked credential; the framework
    /// destroys them and never returns them to a caller.
    pub stale: Vec<S>,
}

// ─── CheckedOut ───────────────────────────────────────────────────────────────

/// A slot that has been checked out of the [`InstanceStore`].
///
/// Carries the slot value and the epoch at checkout time so that
/// `return_slot` can run the revoke-fence check. Topology implementations
/// receive this from [`InstanceStore::checkout`] via [`Checkout::fresh`].
pub struct CheckedOut<S> {
    /// The leased slot.
    pub slot: S,
    /// Epoch captured at checkout time.
    pub(crate) checkout_epoch: u64,
    /// Store's live epoch at checkout time (for informational use).
    #[allow(dead_code)]
    pub(crate) store_epoch: u64,
}

impl<S> CheckedOut<S> {
    /// Consumes the `CheckedOut`, returning the slot and the checkout epoch
    /// for passing to [`InstanceStore::return_slot`].
    pub fn into_parts(self) -> (S, u64) {
        (self.slot, self.checkout_epoch)
    }
}

// ─── ReturnOutcome ─────────────────────────────────────────────────────────────

/// The outcome of [`InstanceStore::return_slot`] / [`InstanceStore::deposit_fresh`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReturnOutcome<S> {
    /// The slot was returned to the idle queue — it is clean and ready to
    /// be leased again.
    Recycled,
    /// The slot was NOT returned because its checkout epoch is behind the
    /// live revoke counter, or the capacity cap was reached. The slot is
    /// handed back for the caller to destroy.
    Evict(S),
}

impl<S> ReturnOutcome<S> {
    /// Returns `true` if the slot was evicted and must be destroyed.
    pub fn is_evict(&self) -> bool {
        matches!(self, Self::Evict(_))
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // Slot returned before epoch bump → Recycled (re-pooled).
    #[tokio::test]
    async fn return_slot_current_epoch_is_recycled() {
        let store: InstanceStore<u32> = InstanceStore::new(Some(4));
        let epoch = store.stamp_epoch();
        let outcome = store.return_slot(42u32, epoch).await;
        assert_eq!(outcome, ReturnOutcome::Recycled);
        assert_eq!(store.len().await, 1);
    }

    // Slot returned AFTER epoch bump → Evict (revoke fence triggered).
    #[tokio::test]
    async fn return_slot_after_epoch_bump_is_evicted() {
        let store: InstanceStore<u32> = InstanceStore::new(Some(4));
        // Stamp the epoch BEFORE the bump (simulate checkout epoch).
        let checkout_epoch = store.stamp_epoch();
        // Simulate a credential revoke.
        store.bump_revoke_epoch();
        // Now return — the checkout_epoch is behind the live counter.
        let outcome = store.return_slot(42u32, checkout_epoch).await;
        assert!(
            outcome.is_evict(),
            "a slot checked out before a revoke must be evicted, not re-pooled"
        );
        assert_eq!(
            store.len().await,
            0,
            "evicted slot must not appear in the idle queue"
        );
    }

    // Multiple bumps: any advance evicts.
    #[tokio::test]
    async fn return_slot_multiple_epoch_bumps_evicts() {
        let store: InstanceStore<u32> = InstanceStore::new(None);
        let checkout_epoch = store.stamp_epoch();
        store.bump_revoke_epoch();
        store.bump_revoke_epoch();
        let outcome = store.return_slot(99u32, checkout_epoch).await;
        assert!(outcome.is_evict());
    }

    // Slot returned at the same epoch after a bump is recycled.
    #[tokio::test]
    async fn return_slot_same_epoch_after_bump_is_recycled() {
        let store: InstanceStore<u32> = InstanceStore::new(None);
        store.bump_revoke_epoch();
        // Stamp the epoch AFTER the bump → checkout_epoch == live_epoch.
        let checkout_epoch = store.stamp_epoch();
        let outcome = store.return_slot(7u32, checkout_epoch).await;
        assert_eq!(outcome, ReturnOutcome::Recycled);
        assert_eq!(store.len().await, 1);
    }

    // Checkout-return round trip preserves the slot value.
    #[tokio::test]
    async fn checkout_return_roundtrip() {
        let store: InstanceStore<String> = InstanceStore::new(None);
        let epoch = store.stamp_epoch();
        store.return_slot("hello".to_owned(), epoch).await;
        let checkout = store.checkout().await;
        assert!(checkout.stale.is_empty(), "no stale slots on a clean queue");
        let fresh_slot = checkout.fresh.map(|c| c.slot);
        assert_eq!(fresh_slot.as_deref(), Some("hello"));
        assert_eq!(store.len().await, 0);
    }

    // C1 fence-on-checkout (a): a slot that went idle, then had its credential
    // revoked, must land in `stale` — never `fresh`.
    #[tokio::test]
    async fn checkout_evicts_slot_revoked_while_idle() {
        let store: InstanceStore<u32> = InstanceStore::new(Some(4));
        // Slot goes idle at epoch 0.
        let epoch = store.stamp_epoch();
        store.return_slot(42u32, epoch).await;
        // Credential revoked while it sat idle.
        store.bump_revoke_epoch();

        let checkout = store.checkout().await;
        assert!(
            checkout.fresh.is_none(),
            "a slot revoked while idle must never be handed out as fresh"
        );
        assert_eq!(
            checkout.stale,
            vec![42u32],
            "the since-revoked slot must be collected for destruction"
        );
        assert_eq!(store.len().await, 0, "the idle queue is drained");
    }

    // C1 fence-on-checkout (b): a mix of stale and fresh slots returns only
    // the first fresh one, with every stale slot collected.
    #[tokio::test]
    async fn checkout_returns_fresh_after_collecting_stale() {
        let store: InstanceStore<u32> = InstanceStore::new(None);
        // Two slots go idle at epoch 0.
        let old_epoch = store.stamp_epoch();
        store.return_slot(1u32, old_epoch).await;
        store.return_slot(2u32, old_epoch).await;
        // Revoke — both are now stale.
        store.bump_revoke_epoch();
        // A fresh slot is returned at the new epoch and queued at the back.
        let new_epoch = store.stamp_epoch();
        store.return_slot(3u32, new_epoch).await;

        let checkout = store.checkout().await;
        assert_eq!(
            checkout.stale,
            vec![1u32, 2u32],
            "both pre-revoke slots are collected as stale, in FIFO order"
        );
        assert_eq!(
            checkout.fresh.map(|c| c.slot),
            Some(3u32),
            "only the current-epoch slot is fresh"
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
        // Push a slot with the initial epoch.
        let old_epoch = store.stamp_epoch();
        store.return_slot(1u32, old_epoch).await;
        // Bump epoch — the slot is now stale.
        store.bump_revoke_epoch();
        // Push a fresh slot with the new epoch.
        let new_epoch = store.stamp_epoch();
        store.return_slot(2u32, new_epoch).await;
        // Evict stale.
        let evicted = store.evict_stale().await;
        assert_eq!(evicted, vec![1u32], "only the pre-bump slot is evicted");
        assert_eq!(store.len().await, 1, "fresh slot remains");
    }

    // capacity cap: return beyond capacity is evicted.
    #[tokio::test]
    async fn capacity_cap_evicts_overflow() {
        let store: InstanceStore<u32> = InstanceStore::new(Some(2));
        let epoch = store.stamp_epoch();
        assert_eq!(store.return_slot(1, epoch).await, ReturnOutcome::Recycled);
        assert_eq!(store.return_slot(2, epoch).await, ReturnOutcome::Recycled);
        assert!(
            store.return_slot(3, epoch).await.is_evict(),
            "third slot exceeds cap of 2 → evicted"
        );
        assert_eq!(store.len().await, 2);
    }

    // drain_all empties the queue.
    #[tokio::test]
    async fn drain_all_empties_store() {
        let store: InstanceStore<u32> = InstanceStore::new(None);
        let epoch = store.stamp_epoch();
        store.return_slot(10, epoch).await;
        store.return_slot(20, epoch).await;
        let drained = store.drain_all().await;
        assert_eq!(drained.len(), 2);
        assert!(store.is_empty().await);
    }
}
