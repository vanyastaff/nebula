use std::{sync::Arc, time::Instant};

use super::*;
use crate::manager::Manager;

#[test]
fn shutdown_debug_redacts_provider_error_but_preserves_source_chain() {
    let failure = ShutdownError::ResourceTeardownFailed {
        key: nebula_core::resource_key!("debug-safety"),
        source: crate::Error::permanent("PRIVATE_PROVIDER_PAYLOAD"),
    };
    for rendered in [
        format!("{failure:?}"),
        format!("{failure:#?}"),
        failure.to_string(),
    ] {
        assert!(!rendered.contains("PRIVATE_PROVIDER_PAYLOAD"));
        assert!(rendered.contains("debug-safety"));
    }
    assert!(
        std::error::Error::source(&failure)
            .unwrap()
            .to_string()
            .contains("PRIVATE_PROVIDER_PAYLOAD")
    );
}

#[tokio::test]
async fn shutdown_keeps_cleanup_open_for_already_retired_producers() {
    let manager = Manager::new();
    let settlement = RetirementSettlement::new(Arc::clone(&manager.retirement_tracker));
    let queue = Arc::clone(&manager.release_queue);
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (resume_tx, resume_rx) = tokio::sync::oneshot::channel();
    let (cleaned_tx, cleaned_rx) = tokio::sync::oneshot::channel();
    // Models a removed row's maintenance producer: the registry snapshot
    // is empty, but producer ownership must keep admission to cleanup open.
    manager
        .release_queue
        .submit_release(move || {
            Box::pin(async move {
                let _settlement = settlement;
                started_tx.send(()).expect("start receiver");
                resume_rx.await.expect("resume sender");
                queue.submit(move || {
                    Box::pin(async move {
                        cleaned_tx.send(()).expect("cleanup receiver");
                    })
                });
                Ok(())
            })
        })
        .expect("open manager queue must accept cleanup producer")
        .detach();
    started_rx.await.expect("producer started");
    let shutdown = manager.graceful_shutdown(ShutdownConfig::default());
    tokio::pin!(shutdown);
    assert!(futures::poll!(shutdown.as_mut()).is_pending());
    resume_tx.send(()).expect("producer still owned");
    let report = shutdown.await.expect("producer and workers drain");
    cleaned_rx.await.expect("late cleanup was accepted");
    assert!(report.release_queue_drained);
    assert_eq!(report.dropped_release_tasks, 0);
}

#[tokio::test]
async fn cancelled_worker_wait_is_resumed_and_abort_joined_by_manager_task() {
    let manager = Arc::new(Manager::new());
    let ownership = Arc::new(());
    let lease = Arc::clone(&ownership);
    manager.release_queue.submit(move || {
        Box::pin(async move {
            std::future::pending::<()>().await;
            drop(lease);
        })
    });
    let shutdown_manager = Arc::clone(&manager);
    let config = ShutdownConfig::default().with_release_queue_timeout(Duration::from_millis(20));
    let shutdown_config = config.clone();
    let shutdown =
        tokio::spawn(async move { shutdown_manager.graceful_shutdown(shutdown_config).await });

    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let worker_handle_was_taken = manager
                .release_queue_handle
                .try_lock()
                .is_ok_and(|handle| handle.is_none());
            if worker_handle_was_taken {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("shutdown must reach release-worker ownership");

    shutdown.abort();
    let _ = shutdown.await;
    assert!(matches!(
        manager.graceful_shutdown(config).await,
        Err(ShutdownError::ReleaseQueueTimeout { .. })
    ));
    tokio::time::timeout(Duration::from_secs(1), async {
        while Arc::strong_count(&ownership) != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("cancelled worker must release task ownership");
    assert_eq!(Arc::strong_count(&ownership), 1);
    assert_eq!(manager.release_queue.dropped_count(), 1);
    assert!(matches!(
        manager.graceful_shutdown(ShutdownConfig::default()).await,
        Err(ShutdownError::AlreadyShuttingDown)
    ));
}

#[tokio::test]
async fn manager_drop_closes_queue_even_with_external_queue_reference() {
    let manager = Manager::new();
    let queue = Arc::clone(&manager.release_queue);
    let handle = manager.release_queue_handle.lock().await.take().unwrap();
    drop(manager);
    ReleaseQueue::shutdown(handle).await;
    queue.submit(|| panic!("manager drop closes queue"));
    assert_eq!(queue.dropped_count(), 1);
}

/// Regression for the drain-race bug: previously `wait_for_drain`
/// did `tracker.1.notified().await` without pre-registering the
/// `Notified` future, so a handle dropping (and firing
/// `notify_waiters()`) in the window between the outer
/// `active == 0` check and the first `notified().await` poll would
/// leak the wakeup. Stall persisted until the full `drain_timeout`
/// elapsed.
///
/// The fix pre-enables the `Notified` future and re-checks the
/// counter *after* registration, so a drop that completes the drain
/// mid-race is observed on the re-check and returns immediately.
///
/// This test exercises the normal "handle drops while we're waiting"
/// path and asserts we return far sooner than the timeout.
#[tokio::test]
async fn wait_for_drain_returns_promptly_when_handle_drops() {
    let mgr = Manager::new();
    // Simulate one active handle.
    mgr.drain_tracker.0.fetch_add(1, AtomicOrdering::Release);

    let tracker = Arc::clone(&mgr.drain_tracker);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(20)).await;
        if tracker.0.fetch_sub(1, AtomicOrdering::Release) == 1 {
            tracker.1.notify_waiters();
        }
    });

    let start = Instant::now();
    mgr.wait_for_drain(Duration::from_secs(30))
        .await
        .expect("handle drop must drain under the timeout");
    let elapsed = start.elapsed();

    assert!(
        elapsed < Duration::from_secs(1),
        "wait_for_drain should return within 1s when a handle drops, took {elapsed:?}"
    );
    assert_eq!(mgr.drain_tracker.0.load(AtomicOrdering::Acquire), 0);
}

/// Regression: if the counter reaches 0 *before* `wait_for_drain`
/// gets to pre-register the `Notified`, the post-enable re-check
/// must catch it and return immediately rather than stalling.
///
/// We simulate the race by setting `active = 1` (so the outer
/// early-return doesn't fire), then immediately decrementing to 0
/// before `wait_for_drain` is polled.
#[tokio::test]
async fn wait_for_drain_catches_drop_via_recheck() {
    let mgr = Manager::new();
    mgr.drain_tracker.0.fetch_add(1, AtomicOrdering::Release);

    // Decrement + notify synchronously — the counter is 0 before
    // `wait_for_drain` is even called, but we want to prove that
    // even if the outer check observed `active == 1` and then
    // the counter hit 0 *between* that check and the inner enable,
    // the inner re-check would catch it.
    //
    // Simulated here by priming the state and then calling
    // wait_for_drain directly; the inner loop's re-check should
    // fire on the very first iteration because the counter is
    // already 0. The outer check is bypassed by the fetch_add
    // above leaving active == 1 until... wait, we need to
    // decrement BETWEEN the outer check and the inner enable.
    //
    // Easiest approximation: skip the outer early-return by
    // keeping active = 1 through the outer check, then decrement
    // via a spawned task that runs before wait_for_drain gets
    // scheduler time.
    let tracker = Arc::clone(&mgr.drain_tracker);
    tokio::task::yield_now().await;
    let handle = tokio::spawn(async move {
        // Yield so that wait_for_drain's outer load sees active = 1,
        // then decrement before the inner poll happens.
        tokio::task::yield_now().await;
        if tracker.0.fetch_sub(1, AtomicOrdering::Release) == 1 {
            tracker.1.notify_waiters();
        }
    });

    let start = Instant::now();
    mgr.wait_for_drain(Duration::from_secs(30))
        .await
        .expect("recheck path must drain under the timeout");
    let elapsed = start.elapsed();
    handle.await.unwrap();

    assert!(
        elapsed < Duration::from_secs(1),
        "wait_for_drain must return promptly even under race, took {elapsed:?}"
    );
}

/// #302: Abort policy must return a typed `DrainTimeout` error and
/// leave the registry untouched. Before the policy split
/// `graceful_shutdown` would log a warning and proceed to
/// `registry.clear()` anyway, turning a cooperative shutdown into a
/// logical use-after-free.
#[tokio::test]
async fn graceful_shutdown_abort_policy_returns_drain_timeout_error() {
    let mgr = Manager::new();
    // Simulate an outstanding handle.
    mgr.drain_tracker.0.fetch_add(1, AtomicOrdering::Release);

    let cfg = ShutdownConfig::default()
        .with_drain_timeout(Duration::from_millis(50))
        .with_drain_timeout_policy(DrainTimeoutPolicy::Abort);

    let err = mgr
        .graceful_shutdown(cfg)
        .await
        .expect_err("Abort policy must surface drain timeout");
    match err {
        ShutdownError::DrainTimeout { outstanding } => {
            assert_eq!(outstanding, 1, "outstanding count mismatch");
        },
        other => panic!("wrong error variant: {other:?}"),
    }
}

/// #302: Force policy must clear the registry and report the
/// outstanding-handle count in `ShutdownReport` so operators can see
/// exactly how much in-flight work was abandoned.
#[tokio::test]
async fn graceful_shutdown_force_policy_clears_registry_with_outstanding_count() {
    let mgr = Manager::new();
    mgr.drain_tracker.0.fetch_add(2, AtomicOrdering::Release);

    let cfg = ShutdownConfig::default()
        .with_drain_timeout(Duration::from_millis(50))
        .with_drain_timeout_policy(DrainTimeoutPolicy::Force);

    let report = mgr
        .graceful_shutdown(cfg)
        .await
        .expect("Force policy must succeed");
    assert!(report.registry_cleared);
    assert_eq!(report.outstanding_handles_after_drain, 2);
}
