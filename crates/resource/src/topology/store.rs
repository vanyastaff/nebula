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

use tokio::sync::Mutex;
use tracing::debug;

// ─── InstanceStore ────────────────────────────────────────────────────────────

/// A timestamped slot entry: the slot value plus the revoke-epoch snapshot
/// taken when the slot was checked out.
///
/// The epoch is captured at checkout time so a `return_slot` after a
/// `bump_revoke_epoch()` detects the stale epoch and evicts rather than
/// re-pooling.
struct StoreEntry<S> {
    slot: S,
    /// The store's revoke-epoch as observed when this slot was **checked out**
    /// (via [`InstanceStore::checkout`] → stamps with the live counter).
    checkout_epoch: u64,
}

/// Framework-owned idle queue and revoke-epoch state for a
/// [`Topology`](crate::topology::Topology)'s slots.
///
/// An `InstanceStore<S>` is the storage the [`Manager`] owns; a
/// [`Topology`](crate::topology::Topology) implementation receives a borrowed
/// `&InstanceStore<Self::Slot>` in [`try_reserve`] / [`acquire`] /
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
/// [`acquire`]: crate::topology::Topology::acquire
/// [`on_release`]: crate::topology::Topology::on_release
/// [`phase`]: crate::topology::Topology::phase
/// [`load`]: crate::topology::Topology::load
/// [`return_slot`]: InstanceStore::return_slot
/// [`bump_revoke_epoch`]: InstanceStore::bump_revoke_epoch
pub struct InstanceStore<S> {
    /// Framework-held idle queue; slots are `(S, checkout_epoch)` pairs.
    idle: Arc<Mutex<VecDeque<StoreEntry<S>>>>,
    /// Monotonic credential-revoke counter. Bumped synchronously by the
    /// manager on credential revoke — before any async revoke hook dispatch.
    /// Every `return_slot` compares the slot's checkout epoch against this;
    /// an advanced counter evicts the slot instead of re-queuing it.
    revoke_epoch: Arc<AtomicU64>,
    /// Maximum number of slots the store will hold idle.
    /// `None` = unbounded (Resident / permit-only topologies).
    capacity: Option<usize>,
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

    /// Checks out an idle slot from the store.
    ///
    /// Returns `Some(slot)` stamped with the current epoch if an idle slot
    /// exists, `None` if the queue is empty. The epoch stamp is embedded in
    /// `StoreEntry` and is passed back to `return_slot` via the opaque
    /// [`CheckedOut<S>`] handle.
    ///
    /// Topology implementations call this inside `try_reserve` or `acquire` to
    /// pop an existing idle slot before creating a new one.
    pub async fn checkout(&self) -> Option<CheckedOut<S>> {
        let epoch = self.current_revoke_epoch();
        let slot = self.idle.lock().await.pop_front()?;
        Some(CheckedOut {
            slot: slot.slot,
            checkout_epoch: slot.checkout_epoch,
            store_epoch: epoch,
        })
    }

    /// Returns a slot to the idle queue, running the revoke-epoch fence.
    ///
    /// If the slot's `checkout_epoch` is behind the live revoke counter, the
    /// slot was leased under a since-revoked credential; `action` is called
    /// with `Return::Evict` and the slot is NOT re-queued. If the epoch is
    /// current and the optional capacity cap has not been reached, the slot
    /// is returned to the idle queue and `action` is called with
    /// `Return::Recycled`.
    ///
    /// The `action` callback lets the topology drive async eviction (e.g.
    /// calling `Provider::destroy`) without the store owning `Provider`.
    ///
    /// # Fence guarantee
    ///
    /// The epoch re-read and the push are performed while holding the idle
    /// lock, so a concurrent `bump_revoke_epoch` followed by an idle-walk
    /// cannot enqueue a stale slot: the walk holds the same lock and sees the
    /// already-bumped counter.
    pub async fn return_slot(&self, slot: S, checkout_epoch: u64) -> ReturnOutcome {
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
            return ReturnOutcome::Evict;
        }
        // Capacity check.
        if let Some(cap) = self.capacity
            && idle.len() >= cap
        {
            return ReturnOutcome::Evict;
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
    /// background maintenance reaper.
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
}

// ─── CheckedOut ───────────────────────────────────────────────────────────────

/// A slot that has been checked out of the [`InstanceStore`].
///
/// Carries the slot value and the epoch at checkout time so that
/// `return_slot` can run the revoke-fence check. Topology implementations
/// receive this from [`InstanceStore::checkout`].
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

/// The outcome of [`InstanceStore::return_slot`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReturnOutcome {
    /// The slot was returned to the idle queue — it is clean and ready to
    /// be leased again.
    Recycled,
    /// The slot was NOT returned because its checkout epoch is behind the
    /// live revoke counter, or the capacity cap was reached. The caller
    /// should destroy the slot.
    Evict,
}

impl ReturnOutcome {
    /// Returns `true` if the slot was evicted and must be destroyed.
    pub fn is_evict(self) -> bool {
        self == Self::Evict
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
        assert_eq!(
            outcome,
            ReturnOutcome::Evict,
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
        assert_eq!(outcome, ReturnOutcome::Evict);
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
        let checked_out = store.checkout().await.expect("should find idle slot");
        assert_eq!(checked_out.slot, "hello");
        assert_eq!(store.len().await, 0);
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
        assert_eq!(
            store.return_slot(3, epoch).await,
            ReturnOutcome::Evict,
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
