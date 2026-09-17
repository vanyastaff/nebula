use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use super::*;
use crate::release_queue::ReleaseQueue;

#[derive(Debug)]
struct DropProbe(Arc<AtomicUsize>);

impl Drop for DropProbe {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn terminal_fence_rejects_checkout_return_and_fresh_deposit() {
    let store = InstanceStore::new(Some(4));
    let sibling = store.clone();
    let epoch = store.stamp_epoch();
    assert_eq!(
        store.return_entry(1_u32, epoch).await,
        ReturnOutcome::Recycled
    );
    assert_eq!(store.deposit_fresh(2, epoch).await, ReturnOutcome::Recycled);

    store.begin_close();
    assert!(
        sibling.is_closed(),
        "closure is shared by every store handle"
    );
    let checkout = sibling.checkout().await;
    assert!(checkout.fresh.is_none());
    assert_eq!(checkout.stale, vec![1, 2]);
    assert_eq!(store.return_entry(3, epoch).await, ReturnOutcome::Evict(3));
    assert_eq!(store.deposit_fresh(4, epoch).await, ReturnOutcome::Evict(4));
    store.bump_revoke_epoch();
    assert_eq!(
        sibling.deposit_fresh(5, sibling.stamp_epoch()).await,
        ReturnOutcome::Evict(5),
        "a fresh credential epoch cannot reopen a retired store"
    );
    assert_eq!(store.len().await, 0);
}

#[tokio::test]
async fn terminal_fence_rechecks_return_waiting_for_idle_lock() {
    let store = InstanceStore::new(None);
    let epoch = store.stamp_epoch();
    assert_eq!(
        store.return_entry(1_u32, epoch).await,
        ReturnOutcome::Recycled
    );
    let idle = store.lock_idle().await;
    let returning = store.return_entry(2, epoch);
    tokio::pin!(returning);
    assert!(futures::poll!(returning.as_mut()).is_pending());

    store.begin_close();
    drop(idle);
    assert_eq!(returning.await, ReturnOutcome::Evict(2));
    assert_eq!(store.close_and_drain().await, vec![1]);
    assert!(store.close_and_drain().await.is_empty());
}

#[tokio::test]
async fn terminal_fence_rejects_deposit_under_already_held_lock() {
    let store = InstanceStore::new(None);
    let epoch = store.stamp_epoch();
    let mut idle = store.lock_idle().await;
    store.begin_close();
    assert_eq!(
        store.deposit_fresh_locked(&mut idle, 7_u32, epoch),
        ReturnOutcome::Evict(7)
    );
    assert!(idle.is_empty());
}

#[tokio::test]
async fn terminal_fence_survives_cancelled_drain_without_losing_entries() {
    let store = InstanceStore::new(None);
    let epoch = store.stamp_epoch();
    assert_eq!(
        store.return_entry(9_u32, epoch).await,
        ReturnOutcome::Recycled
    );
    let idle = store.lock_idle().await;
    {
        let closing = store.close_and_drain();
        tokio::pin!(closing);
        assert!(futures::poll!(closing.as_mut()).is_pending());
        assert!(store.is_closed(), "the fence precedes the first await");
    }
    assert!(store.is_closed());
    drop(idle);
    assert_eq!(store.close_and_drain().await, vec![9]);
    assert!(store.is_empty().await);
}

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

// Fence-on-checkout (a): an entry that went idle, then had its credential
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

// Fence-on-checkout (b): a mix of stale and fresh entries returns only
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

// Fence-on-checkout (c): an empty queue returns no fresh and no stale.
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

#[tokio::test]
async fn final_shared_store_drop_accounts_for_each_idle_owner_once() {
    let (queue, workers) = ReleaseQueue::new(1);
    let store = InstanceStore::with_abandonment_tracker(None, queue.abandonment_tracker());
    let raw_drops = Arc::new(AtomicUsize::new(0));
    let epoch = store.stamp_epoch();
    for _ in 0..3 {
        std::assert_matches!(
            store
                .return_entry(DropProbe(Arc::clone(&raw_drops)), epoch)
                .await,
            ReturnOutcome::Recycled
        );
    }
    let final_handle = store.clone();

    drop(store);
    assert_eq!(
        queue.dropped_count(),
        0,
        "an intermediate handle owns nothing alone"
    );
    assert_eq!(raw_drops.load(Ordering::SeqCst), 0);
    drop(final_handle);

    assert_eq!(
        queue.dropped_count(),
        3,
        "all three idle owners were abandoned once"
    );
    assert_eq!(raw_drops.load(Ordering::SeqCst), 3);
    queue.close();
    ReleaseQueue::shutdown(workers).await;
}

#[tokio::test]
async fn dropping_intermediate_clones_never_reports_shared_idle_owners() {
    let (queue, workers) = ReleaseQueue::new(1);
    let store = InstanceStore::with_abandonment_tracker(None, queue.abandonment_tracker());
    let epoch = store.stamp_epoch();
    let raw_drops = Arc::new(AtomicUsize::new(0));
    std::assert_matches!(
        store
            .return_entry(DropProbe(Arc::clone(&raw_drops)), epoch)
            .await,
        ReturnOutcome::Recycled
    );
    let first_clone = store.clone();
    let second_clone = store.clone();

    drop(first_clone);
    drop(second_clone);
    assert_eq!(queue.dropped_count(), 0);
    assert_eq!(raw_drops.load(Ordering::SeqCst), 0);
    drop(store);

    assert_eq!(queue.dropped_count(), 1);
    assert_eq!(raw_drops.load(Ordering::SeqCst), 1);
    queue.close();
    ReleaseQueue::shutdown(workers).await;
}

#[tokio::test]
async fn normal_drain_transfers_idle_owners_without_abandonment() {
    let (queue, workers) = ReleaseQueue::new(1);
    let store = InstanceStore::with_abandonment_tracker(None, queue.abandonment_tracker());
    let epoch = store.stamp_epoch();
    let raw_drops = Arc::new(AtomicUsize::new(0));
    for _ in 0..2 {
        std::assert_matches!(
            store
                .return_entry(DropProbe(Arc::clone(&raw_drops)), epoch)
                .await,
            ReturnOutcome::Recycled
        );
    }

    let drained = store.drain_all().await;
    drop(store);
    assert_eq!(queue.dropped_count(), 0, "drain transferred both owners");
    assert_eq!(raw_drops.load(Ordering::SeqCst), 0);
    drop(drained);

    assert_eq!(queue.dropped_count(), 0);
    assert_eq!(raw_drops.load(Ordering::SeqCst), 2);
    queue.close();
    ReleaseQueue::shutdown(workers).await;
}
