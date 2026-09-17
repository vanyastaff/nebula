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
    let RetainStatus::Published(id) = store.retain(SharedGeneration(Arc::new(GenerationOwner {
        serial: 1,
        drops: Arc::clone(&first_drops),
    }))) else {
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
    let RetainStatus::Published(id) = store.retain(SharedGeneration(Arc::new(GenerationOwner {
        serial: 1,
        drops: Arc::new(AtomicUsize::new(0)),
    }))) else {
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
