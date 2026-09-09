//! Maintenance must transfer cleanup before retirement waits for its task.

use super::*;

#[tokio::test(start_paused = true)]
async fn cancelled_probe_accounts_failures_from_previous_batches() {
    let mut managed = managed(Mock::new(), PoolConfig::default());
    let fifo = managed
        .store
        .clone()
        .with_strategy(crate::PoolStrategy::Fifo);
    Arc::get_mut(&mut managed).unwrap().store = fifo;
    for _ in 0..=PROBE_CONCURRENCY {
        let entry = managed
            .topology
            .create_entry(&managed.resource, &PoolCfg, &test_ctx(), &managed.retained)
            .await
            .unwrap()
            .into_entry();
        assert!(matches!(
            managed.store.deposit_fresh(entry, 0).await,
            ReturnOutcome::Recycled
        ));
    }
    managed
        .resource
        .fail_probe_batch_then_park
        .store(true, Ordering::SeqCst);
    let mut probe = Box::pin(managed.probe_idle_entries());
    tokio::select! {
        () = managed.resource.check_started.notified() => {},
        _ = &mut probe => panic!("second probe batch must remain parked"),
    }
    drop(probe);
    assert_eq!(
        managed
            .release_queue
            .submit_release(|| Box::pin(async { Ok(()) }))
            .expect("open queue must accept cleanup checkpoint")
            .wait()
            .await
            .unwrap(),
        SubmissionOutcome::Completed,
    );
    assert_eq!(
        managed.resource.destroyed.load(Ordering::SeqCst),
        1,
        "the still-running check entry remains guarded"
    );
    assert_eq!(
        managed.release_queue.dropped_count(),
        PROBE_CONCURRENCY,
        "every earlier failed entry must remain loss-accounted across subsequent awaits"
    );
}

#[tokio::test(start_paused = true)]
async fn cancelling_probe_waiting_to_return_keeps_entry_cleanup_owned() {
    let managed = managed(Mock::new(), PoolConfig::default());
    let entry = managed
        .topology
        .create_entry(&managed.resource, &PoolCfg, &test_ctx(), &managed.retained)
        .await
        .unwrap()
        .into_entry();
    assert!(matches!(
        managed.store.deposit_fresh(entry, 0).await,
        ReturnOutcome::Recycled
    ));
    managed.resource.park_in_check.store(true, Ordering::SeqCst);
    let mut probe = Box::pin(managed.probe_idle_entries());
    tokio::select! {
        () = managed.resource.check_started.notified() => {},
        _ = &mut probe => panic!("check must remain parked"),
    }
    let idle = managed.store.lock_idle().await;
    managed.resource.release_check.notify_one();
    assert!(
        futures::poll!(&mut probe).is_pending(),
        "probe must now wait for idle return lock"
    );
    drop(probe);
    drop(idle);
    assert_eq!(
        managed
            .release_queue
            .submit_release(|| Box::pin(async { Ok(()) }))
            .expect("open queue must accept cleanup checkpoint")
            .wait()
            .await
            .unwrap(),
        SubmissionOutcome::Completed,
    );
    assert_eq!(
        managed.resource.destroyed.load(Ordering::SeqCst),
        1,
        "cancelled return must schedule destroy, not drop a raw entry"
    );
    assert_eq!(managed.release_queue.dropped_count(), 0);
}

#[tokio::test(start_paused = true)]
async fn retiring_maintenance_check_can_await_same_queue_guard_release() {
    let manager = Arc::new(crate::Manager::with_config(
        crate::ManagerConfig::default().with_release_queue_workers(1),
    ));
    let resource = Mock::new();
    manager
        .register(crate::RegistrationSpec {
            resource: resource.clone(),
            config: PoolCfg,
            scope: crate::ScopeLevel::Global,
            slot_identity: crate::SlotIdentity::Unbound,
            topology: Pooled::new(PoolConfig::default(), 0),
            recovery_gate: None,
        })
        .unwrap();
    async fn acquire(manager: &Arc<crate::Manager>) -> ResourceGuard<Mock> {
        *crate::Manager::acquire_any(
            Arc::clone(manager),
            &Mock::key(),
            &test_ctx(),
            &AcquireOptions::default(),
            &crate::SlotIdentity::Unbound,
        )
        .await
        .unwrap()
        .downcast::<ResourceGuard<Mock>>()
        .unwrap()
    }
    let parent = acquire(&manager).await;
    let mut child = acquire(&manager).await;
    child.taint();
    let _release_outcome = parent.release().await.unwrap();
    let row = manager.lookup::<Mock>(&crate::ScopeLevel::Global).unwrap();
    *resource.dependent_release.lock().unwrap() = Some(child);
    resource
        .release_dependency_in_check
        .store(true, Ordering::SeqCst);
    resource.park_in_check.store(true, Ordering::SeqCst);
    // The registered pool's owned reaper reaches this explicit hook barrier.
    resource.check_started.notified().await;
    manager.remove(&Mock::key()).unwrap();
    // Retirement was published before the check can submit its dependent release.
    resource.release_check.notify_one();
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        manager.graceful_shutdown(crate::ShutdownConfig::default()),
    )
    .await
    .expect("retirement must leave cleanup workers available to maintenance check dependencies")
    .unwrap();
    assert_eq!(resource.destroyed.load(Ordering::SeqCst), 2);
    assert_eq!(row.release_queue.dropped_count(), 0);
}

#[tokio::test(start_paused = true)]
async fn retirement_waiting_for_maintenance_does_not_block_dependent_release() {
    let mut managed = managed(Mock::new(), PoolConfig::default());
    let (queue, handle) = ReleaseQueue::new(1);
    let row = Arc::get_mut(&mut managed).unwrap();
    row.store = InstanceStore::with_abandonment_tracker(None, queue.abandonment_tracker());
    row.retained = crate::RetainedStore::new(queue.abandonment_tracker());
    row.release_queue = Arc::new(queue);
    let created = managed
        .topology
        .create_entry(&managed.resource, &PoolCfg, &test_ctx(), &managed.retained)
        .await
        .unwrap();
    let parent = created.into_entry();
    assert!(managed.retained.drain_retired().is_empty());
    let mut child = managed
        .run_acquire_loop(&test_ctx(), &AcquireOptions::default(), None)
        .await
        .unwrap();
    child.taint();
    *managed.resource.dependent_release.lock().unwrap() = Some(child);
    assert!(matches!(
        managed.store.deposit_fresh(parent, 0).await,
        ReturnOutcome::Recycled
    ));
    managed.resource.park_in_check.store(true, Ordering::SeqCst);
    let sweeping = Arc::clone(&managed);
    managed.maintenance.set_task(tokio::spawn(async move {
        sweeping.run_maintenance().await;
    }));
    managed.resource.check_started.notified().await;

    let closing = Arc::clone(&managed);
    let (entered, retirement_entered) = tokio::sync::oneshot::channel();
    let retirement = tokio::spawn(async move {
        closing.begin_close();
        entered.send(()).unwrap();
        let maintenance = closing.join_maintenance().await;
        let terminal = Arc::clone(&closing);
        let cleanup = closing
            .release_queue
            .submit_coordinator(move || Box::pin(async move { terminal.close_retained().await }))
            .expect("open queue must accept terminal cleanup")
            .wait()
            .await;
        maintenance.and(cleanup)
    });
    retirement_entered.await.unwrap();
    managed.resource.release_check.notify_one();

    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        assert_eq!(
            retirement.await.unwrap().unwrap(),
            SubmissionOutcome::Completed,
        );
        managed.resource.destroy_finished.notified().await;
    })
    .await
    .expect("retirement must not wait on a maintenance-to-own-queue dependency");
    assert_eq!(managed.resource.destroyed.load(Ordering::SeqCst), 2);
    managed.release_queue.close();
    ReleaseQueue::shutdown_bounded(handle, std::time::Duration::from_secs(1))
        .await
        .unwrap();
    assert_eq!(managed.release_queue.dropped_count(), 0);
}
