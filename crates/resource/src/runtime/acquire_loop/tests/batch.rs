//! Batch ownership, per-entry fault boundaries, and cancellation regressions.

use super::*;

#[tokio::test(start_paused = true)]
async fn destroy_batch_timeout_continues_error_and_success_siblings() {
    let resource = Mock::new();
    resource.batch_failures.store(true, Ordering::SeqCst);
    let managed = managed(resource, PoolConfig::default());
    let mut entries = Vec::new();
    for _ in 0..3 {
        let created = managed
            .topology
            .create_entry(&managed.resource, &PoolCfg, &test_ctx(), &managed.retained)
            .await
            .unwrap();
        let entry = created.into_entry();
        assert!(managed.retained.drain_retired().is_empty());
        entries.push(entry);
    }
    let result = managed
        .queue_destroy_batch(entries, TeardownReason::Evicted)
        .unwrap()
        .unwrap()
        .wait()
        .await;
    assert!(
        result.is_err(),
        "batch must retain the first teardown failure"
    );
    assert_eq!(
        managed.resource.destroyed.load(Ordering::SeqCst),
        3,
        "a wedged member must not discard untouched siblings"
    );
    assert_eq!(
        managed.release_queue.dropped_count(),
        1,
        "only the timed-out entry is abandoned"
    );
    managed.release_queue.close();
}

async fn batch_entries(managed: &ManagedResource<Mock>, count: usize) -> Vec<EntryOf<Mock>> {
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        let created = managed
            .topology
            .create_entry(&managed.resource, &PoolCfg, &test_ctx(), &managed.retained)
            .await
            .unwrap();
        let entry = created.into_entry();
        assert!(managed.retained.drain_retired().is_empty());
        entries.push(entry);
    }
    entries
}

#[tokio::test(start_paused = true)]
async fn rejected_large_destroy_batch_accounts_every_unpolled_entry() {
    let managed = managed(Mock::new(), PoolConfig::default());
    let entries = batch_entries(&managed, 10_000).await;
    managed.release_queue.close();
    let result = managed
        .queue_destroy_batch(entries, TeardownReason::Evicted)
        .unwrap()
        .unwrap_err();
    assert_eq!(*result.kind(), crate::ErrorKind::Cancelled);
    assert_eq!(managed.release_queue.dropped_count(), 10_000);
    assert_eq!(managed.resource.destroyed.load(Ordering::SeqCst), 0);
}

#[tokio::test(start_paused = true)]
async fn large_destroy_batch_uses_one_queue_message_and_completes_all_entries() {
    let managed = managed(Mock::new(), PoolConfig::default());
    let entries = batch_entries(&managed, 10_000).await;
    assert_eq!(
        managed
            .queue_destroy_batch(entries, TeardownReason::Evicted)
            .unwrap()
            .unwrap()
            .wait()
            .await
            .unwrap(),
        SubmissionOutcome::Completed,
    );
    assert_eq!(managed.resource.destroyed.load(Ordering::SeqCst), 10_000);
    assert_eq!(managed.release_queue.dropped_count(), 0);
    assert_eq!(managed.release_queue.fallback_count(), 0);
    assert_eq!(managed.release_queue.rescued_count(), 0);
    managed.release_queue.close();
}

#[tokio::test(start_paused = true)]
async fn cancelled_destroy_batch_accounts_running_and_untouched_entries() {
    let resource = Mock::new();
    resource.batch_failures.store(true, Ordering::SeqCst);
    let mut managed = managed(resource, PoolConfig::default());
    let (queue, handle) = ReleaseQueue::new(1);
    let row = Arc::get_mut(&mut managed).unwrap();
    row.store = InstanceStore::with_abandonment_tracker(None, queue.abandonment_tracker());
    row.retained = crate::RetainedStore::new(queue.abandonment_tracker());
    row.release_queue = Arc::new(queue);
    let entries = batch_entries(&managed, 10).await;
    let receipt = managed
        .queue_destroy_batch(entries, TeardownReason::Evicted)
        .unwrap()
        .unwrap();
    managed.release_queue.close();
    ReleaseQueue::shutdown_bounded(handle, std::time::Duration::from_secs(1))
        .await
        .unwrap_err();
    assert!(
        receipt.wait().await.is_err(),
        "aborted coordinator cannot report completion"
    );
    assert_eq!(managed.release_queue.dropped_count(), 10);
    assert_eq!(managed.resource.destroyed.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn terminal_teardown_preserves_error_and_attempts_remaining_idle_siblings() {
    let resource = Mock::new();
    resource.batch_failures.store(true, Ordering::SeqCst);
    resource.created.store(1, Ordering::SeqCst);
    let managed = managed(resource, PoolConfig::default());
    for entry in batch_entries(&managed, 2).await {
        assert!(matches!(
            managed.store.deposit_fresh(entry, 0).await,
            ReturnOutcome::Recycled
        ));
    }
    let closing = Arc::clone(&managed);
    let error = managed
        .release_queue
        .submit_coordinator(move || Box::pin(async move { closing.close_retained().await }))
        .expect("open queue must accept terminal cleanup")
        .wait()
        .await
        .unwrap_err();
    assert_eq!(*error.kind(), crate::ErrorKind::Permanent);
    assert_eq!(managed.resource.destroyed.load(Ordering::SeqCst), 2);
    assert_eq!(
        managed.release_queue.dropped_count(),
        0,
        "completed provider Err is not abandoned cleanup"
    );
    assert!(managed.store.is_closed());
    managed.release_queue.close();
}

#[tokio::test(start_paused = true)]
async fn destroy_batch_member_panic_does_not_discard_success_sibling() {
    let resource = Mock::new();
    resource.batch_failures.store(true, Ordering::SeqCst);
    resource.created.store(3, Ordering::SeqCst);
    let managed = managed(resource, PoolConfig::default());
    let entries = batch_entries(&managed, 2).await;
    let error = managed
        .queue_destroy_batch(entries, TeardownReason::Evicted)
        .unwrap()
        .unwrap()
        .wait()
        .await
        .unwrap_err();
    assert_eq!(*error.kind(), crate::ErrorKind::Permanent);
    assert_eq!(managed.resource.destroyed.load(Ordering::SeqCst), 2);
    assert_eq!(managed.release_queue.dropped_count(), 1);
    managed.release_queue.close();
}
