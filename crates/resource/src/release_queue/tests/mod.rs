#[path = "receipt.rs"]
mod receipt_tests;

use std::sync::{
    Arc,
    atomic::{AtomicU32, Ordering},
};

use super::*;

#[tokio::test(start_paused = true)]
async fn sealed_queue_accepts_descendant_of_already_owned_cleanup() {
    let (queue, handle) = ReleaseQueue::new(1);
    let queue = Arc::new(queue);
    let (entered, parent_entered) = oneshot::channel();
    let (resume, resumed) = oneshot::channel();
    let completed = Arc::new(AtomicUsize::new(0));
    let child_completed = Arc::clone(&completed);
    let nested_queue = Arc::clone(&queue);
    let parent = queue
        .submit_release(move || {
            Box::pin(async move {
                entered.send(()).unwrap();
                resumed.await.unwrap();
                let child_outcome = nested_queue
                    .submit_release(move || {
                        Box::pin(async move {
                            child_completed.fetch_add(1, Ordering::SeqCst);
                            Ok(())
                        })
                    })?
                    .wait()
                    .await?;
                assert_eq!(child_outcome, SubmissionOutcome::Completed);
                Ok(())
            })
        })
        .expect("open queue must accept parent cleanup");
    parent_entered.await.unwrap();
    queue.close();
    resume.send(()).unwrap();
    assert_eq!(
        parent.wait().await.expect(
            "a sealed queue must retain descendant cleanup admission until owned parents settle",
        ),
        SubmissionOutcome::Completed,
    );
    ReleaseQueue::shutdown_bounded(handle, Duration::from_secs(1))
        .await
        .unwrap();
    assert_eq!(completed.load(Ordering::SeqCst), 1);
    assert_eq!(queue.dropped_count(), 0);
}

#[tokio::test(start_paused = true)]
async fn cancelled_rescue_drains_every_buffered_owner_before_return() {
    let (sender, receiver) = mpsc::channel(8);
    let (fallback, _fallback_receiver) = mpsc::channel(1);
    let cancel = CancellationToken::new();
    let count = Arc::new(AtomicUsize::new(0));
    for _ in 0..8 {
        sender
            .try_send(RescueTask {
                queued: QueuedTask {
                    factory: Box::new(|| panic!("cancelled rescue must not execute factories")),
                    completion: TaskCompletion {
                        loss: TaskLoss {
                            counter: Arc::clone(&count),
                            reason: Some("test"),
                            remaining: 1,
                        },
                        receipt: None,
                        _permit: None,
                        capacity: None,
                    },
                    class: JobClass::Entry,
                },
                deadline: tokio::time::Instant::now() + RESCUE_TIMEOUT,
            })
            .unwrap_or_else(|_| panic!("test capacity"));
    }
    cancel.cancel();
    ReleaseQueue::rescue_loop(receiver, fallback, cancel).await;
    assert!(sender.is_closed());
    assert_eq!(count.load(Ordering::SeqCst), 8);
}

#[tokio::test(start_paused = true)]
async fn cancelled_reentrant_dispatcher_drains_accepted_work_before_return() {
    let (sender, receiver) = mpsc::channel(8);
    let capacity = Arc::new(Semaphore::new(8));
    let completed = Arc::new(AtomicUsize::new(0));
    let losses = Arc::new(AtomicUsize::new(0));
    let cancel = CancellationToken::new();
    for _ in 0..8 {
        let completed = Arc::clone(&completed);
        sender
            .try_send(QueuedTask {
                factory: Box::new(move || {
                    Box::pin(async move {
                        completed.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    })
                }),
                completion: TaskCompletion {
                    loss: TaskLoss {
                        counter: Arc::clone(&losses),
                        reason: Some("test"),
                        remaining: 1,
                    },
                    receipt: None,
                    _permit: None,
                    capacity: Some(Arc::clone(&capacity).try_acquire_owned().unwrap()),
                },
                class: JobClass::Entry,
            })
            .unwrap_or_else(|_| panic!("test capacity"));
    }
    cancel.cancel();
    ReleaseQueue::reentrant_loop(receiver, cancel, Arc::new(())).await;
    assert!(sender.is_closed());
    assert_eq!(completed.load(Ordering::SeqCst), 8);
    assert_eq!(losses.load(Ordering::SeqCst), 0);
    assert_eq!(capacity.available_permits(), 8);
}

#[tokio::test(start_paused = true)]
async fn bounded_shutdown_owns_pending_reentrant_children() {
    let (queue, handle) = ReleaseQueue::new(1);
    let queue = Arc::new(queue);
    let ownership = Arc::new(());
    let child_owner = Arc::clone(&ownership);
    let (entered, child_entered) = oneshot::channel();
    let nested = Arc::clone(&queue);
    let parent = queue
        .submit_release(move || {
            Box::pin(async move {
                let child_outcome = nested
                    .submit_release(move || {
                        Box::pin(async move {
                            entered.send(()).unwrap();
                            std::future::pending::<()>().await;
                            drop(child_owner);
                            Ok(())
                        })
                    })?
                    .wait()
                    .await?;
                match child_outcome {
                    SubmissionOutcome::Completed | SubmissionOutcome::Deferred => Ok(()),
                }
            })
        })
        .expect("open queue must accept parent cleanup");
    child_entered.await.unwrap();
    queue.close();
    ReleaseQueue::shutdown_bounded(handle, Duration::from_secs(1))
        .await
        .unwrap_err();
    match parent {
        ReleaseSubmission::Await(receipt) => assert!(receipt.await.is_err()),
        ReleaseSubmission::Deferred => {
            panic!("root cleanup must provide an awaitable receipt")
        },
    }
    assert_eq!(queue.dropped_count(), 2);
    assert_eq!(
        Arc::strong_count(&ownership),
        1,
        "child ownership must settle before abort acknowledgement"
    );
}

#[tokio::test(start_paused = true)]
async fn exhausted_reentrant_capacity_defers_accepted_cleanup_without_loss() {
    let (queue, handle) = ReleaseQueue::new(1);
    let queue = Arc::new(queue);
    let capacity = Arc::clone(&queue.reentrant_capacity)
        .try_acquire_many_owned(REENTRANT_CAPACITY as u32)
        .unwrap();
    let (finished, completion) = oneshot::channel();
    let nested_queue = Arc::clone(&queue);
    let parent = queue
        .submit_release(move || {
            Box::pin(async move {
                let submission = nested_queue.submit_release(move || {
                    Box::pin(async move {
                        finished.send(()).unwrap();
                        Ok(())
                    })
                })?;
                assert_eq!(
                    submission.wait().await?,
                    SubmissionOutcome::Deferred,
                    "a nested submission without cooperative capacity is accepted but not completed",
                );
                Ok(())
            })
        })
        .expect("open queue must accept parent cleanup");
    assert_eq!(parent.wait().await.unwrap(), SubmissionOutcome::Completed,);
    completion.await.unwrap();
    drop(capacity);
    queue.close();
    ReleaseQueue::shutdown_bounded(handle, Duration::from_secs(1))
        .await
        .unwrap();
    assert_eq!(queue.dropped_count(), 0);
}

#[tokio::test(start_paused = true)]
async fn closed_nested_target_is_rejected_not_reported_as_deferred() {
    let (queue, handle) = ReleaseQueue::new(1);
    queue.close();
    let rejection = CURRENT_QUEUE
        .scope(Arc::new(()), async {
            queue.submit_release(|| Box::pin(async { Ok(()) }))
        })
        .await;
    let Err(error) = rejection else {
        panic!("closed queue must reject nested cleanup");
    };
    assert_eq!(*error.kind(), crate::ErrorKind::Cancelled);
    ReleaseQueue::shutdown(handle).await;
    assert_eq!(queue.dropped_count(), 1);
}

#[tokio::test(start_paused = true)]
async fn saturated_rescue_has_bounded_publication_and_owned_shutdown() {
    let (queue, handle) = ReleaseQueue::new(1);
    let total = CHANNEL_BUFFER + FALLBACK_BUFFER + RESCUE_CAPACITY + 1;
    let ownership = Arc::new(());
    for _ in 0..total {
        let ownership = Arc::clone(&ownership);
        queue.submit(move || {
            Box::pin(async move {
                std::future::pending::<()>().await;
                drop(ownership);
            })
        });
    }
    assert_eq!(
        queue.dropped_count(),
        1,
        "only capacity overflow is rejected before workers poll"
    );
    queue.close();
    ReleaseQueue::shutdown_bounded(handle, Duration::from_secs(1))
        .await
        .unwrap_err();
    assert_eq!(
        queue.dropped_count(),
        total,
        "shutdown joins rescue ownership, not just primary workers"
    );
    assert_eq!(Arc::strong_count(&ownership), 1);
}

#[tokio::test]
async fn cancelling_public_shutdown_leaves_cleanup_running() {
    let (queue, handle) = ReleaseQueue::new(1);
    let (release, wait) = oneshot::channel();
    let (completed, completion) = oneshot::channel();
    queue.submit(move || {
        Box::pin(async move {
            wait.await.expect("test releases cleanup");
            completed.send(()).expect("test observes completion");
        })
    });
    queue.close();
    {
        let shutdown = ReleaseQueue::shutdown(handle);
        tokio::pin!(shutdown);
        assert!(futures::poll!(shutdown.as_mut()).is_pending());
    }
    release.send(()).expect("cleanup still owns receiver");
    completion
        .await
        .expect("cleanup completed after waiter cancellation");
    assert_eq!(queue.dropped_count(), 0);
}

#[tokio::test(start_paused = true)]
async fn bounded_shutdown_aborts_running_and_buffered_tasks_once() {
    let (queue, handle) = ReleaseQueue::new(1);
    let ownership = Arc::new(());
    let started = Arc::new(Semaphore::new(0));
    let submit_pending = |started: Option<Arc<Semaphore>>| {
        let lease = Arc::clone(&ownership);
        queue.submit(move || {
            Box::pin(async move {
                if let Some(started) = started {
                    started.add_permits(1);
                }
                std::future::pending::<()>().await;
                drop(lease);
            })
        });
    };
    // Running: every slot of the one primary worker.
    for _ in 0..JOBS_PER_WORKER {
        submit_pending(Some(Arc::clone(&started)));
    }
    started
        .acquire_many(JOBS_PER_WORKER as u32)
        .await
        .expect("the start semaphore is never closed")
        .forget();
    // Buffered: the whole primary channel, and one more that overflows to
    // the fallback worker — the abort covers all three alike.
    for _ in 0..=CHANNEL_BUFFER {
        submit_pending(None);
    }
    assert_eq!(queue.fallback_count(), 1, "the last job overflowed");
    queue.close();
    let error = ReleaseQueue::shutdown_bounded(handle, Duration::from_secs(1))
        .await
        .expect_err("pending tasks exhaust the budget");
    assert!(matches!(
        error,
        crate::manager::ShutdownError::ReleaseQueueTimeout { .. }
    ));
    assert_eq!(
        Arc::strong_count(&ownership),
        1,
        "abort acknowledges ownership release"
    );
    assert_eq!(queue.dropped_count(), JOBS_PER_WORKER + CHANNEL_BUFFER + 1);
}

#[tokio::test(start_paused = true)]
async fn bounded_shutdown_shares_one_budget_across_workers() {
    let (queue, handle) = ReleaseQueue::new(2);
    for seconds in [1, 2] {
        queue.submit(move || Box::pin(tokio::time::sleep(Duration::from_secs(seconds))));
    }
    queue.close();
    let budget = Duration::from_millis(1500);
    let started = tokio::time::Instant::now();
    let error = ReleaseQueue::shutdown_bounded(handle, budget)
        .await
        .expect_err("the second worker exceeds the shared budget");
    assert!(
        matches!(error, crate::manager::ShutdownError::ReleaseQueueTimeout { timeout } if timeout == budget)
    );
    assert_eq!(started.elapsed(), budget);
    assert_eq!(
        queue.dropped_count(),
        1,
        "only the unfinished task is abandoned"
    );
}

#[tokio::test]
async fn cancelling_bounded_shutdown_aborts_every_worker() {
    let (queue, handle) = ReleaseQueue::new(1);
    let ownership = Arc::new(());
    for _ in 0..=CHANNEL_BUFFER {
        let lease = Arc::clone(&ownership);
        queue.submit(move || {
            Box::pin(async move {
                std::future::pending::<()>().await;
                drop(lease);
            })
        });
    }
    queue.close();
    {
        let shutdown = ReleaseQueue::shutdown_bounded(handle, Duration::from_secs(1));
        tokio::pin!(shutdown);
        assert!(futures::poll!(shutdown.as_mut()).is_pending());
    }
    tokio::task::yield_now().await;
    assert_eq!(Arc::strong_count(&ownership), 1);
    assert_eq!(queue.dropped_count(), CHANNEL_BUFFER + 1);
}

#[tokio::test]
async fn worker_join_failure_is_typed_and_remaining_workers_are_aborted() {
    let (queue, handle) = ReleaseQueue::new(1);
    handle.workers[0].abort();
    queue.close();
    let error = ReleaseQueue::shutdown_bounded(handle, Duration::from_secs(1))
        .await
        .expect_err("aborted worker cannot report successful drain");
    assert!(matches!(
        error,
        crate::manager::ShutdownError::ReleaseQueueWorkerFailed
    ));
}

#[tokio::test(start_paused = true)]
async fn rescue_and_buffers_each_account_for_abandonment_once() {
    let (queue, handle) = ReleaseQueue::new(1);
    let ownership = Arc::new(());
    let total = CHANNEL_BUFFER + FALLBACK_BUFFER + 1;
    for _ in 0..total {
        let lease = Arc::clone(&ownership);
        queue.submit(move || {
            Box::pin(async move {
                std::future::pending::<()>().await;
                drop(lease);
            })
        });
    }
    assert_eq!(queue.rescued_count(), 1);
    queue.close();
    ReleaseQueue::shutdown_bounded(handle, Duration::from_secs(1))
        .await
        .expect_err("pending cleanup exceeds deadline");
    tokio::task::yield_now().await;
    assert_eq!(queue.dropped_count(), total);
    assert_eq!(Arc::strong_count(&ownership), 1);
}

#[tokio::test]
async fn submit_after_close_rejects_without_running_factory() {
    let (queue, handle) = ReleaseQueue::new(1);
    queue.close();
    queue.submit(|| panic!("closed queue must never call the factory"));
    ReleaseQueue::shutdown(handle).await;
    assert_eq!(queue.dropped_count(), 1);
}

#[tokio::test]
async fn factory_panic_is_counted_once_and_worker_continues() {
    let (queue, handle) = ReleaseQueue::new(1);
    queue.submit(|| panic!("factory fails before constructing a future"));
    let completed = Arc::new(AtomicU32::new(0));
    submit_increment(&queue, &completed);
    queue.close();
    ReleaseQueue::shutdown(handle).await;
    assert_eq!(queue.dropped_count(), 1);
    assert_eq!(completed.load(Ordering::Relaxed), 1);
}

async fn increment_counter(c: Arc<AtomicU32>) {
    c.fetch_add(1, Ordering::Relaxed);
}

fn submit_increment(queue: &ReleaseQueue, counter: &Arc<AtomicU32>) {
    let c = counter.clone();
    queue.submit(move || Box::pin(increment_counter(c)));
}

#[tokio::test]
async fn submit_and_execute() {
    let (queue, handle) = ReleaseQueue::new(2);
    let counter = Arc::new(AtomicU32::new(0));

    for _ in 0..10 {
        submit_increment(&queue, &counter);
    }

    // Give workers time to process.
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(counter.load(Ordering::Relaxed), 10);

    drop(queue);
    ReleaseQueue::shutdown(handle).await;
}

use std::sync::atomic::AtomicBool;

#[tokio::test]
async fn shutdown_completes_after_drop() {
    let (queue, handle) = ReleaseQueue::new(1);
    let done = Arc::new(AtomicBool::new(false));
    let done_clone = done.clone();

    queue.submit(move || {
        Box::pin(async move {
            done_clone.store(true, Ordering::Relaxed);
        })
    });

    tokio::time::sleep(Duration::from_millis(50)).await;
    drop(queue);
    ReleaseQueue::shutdown(handle).await;
    assert!(done.load(Ordering::Relaxed));
}

#[tokio::test]
async fn zero_workers_is_raised_to_one_and_still_runs_cleanup() {
    let (queue, handle) = ReleaseQueue::new(0);
    let counter = Arc::new(AtomicU32::new(0));
    submit_increment(&queue, &counter);
    drop(queue);
    ReleaseQueue::shutdown(handle).await;
    assert_eq!(counter.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn fallback_channel_handles_overflow() {
    // Use 1 worker so primary channel has 256 capacity.
    // Fallback has 4096 capacity. Total: 4352. 1500 < 4352 → no drops.
    let total_tasks: u32 = 1500;
    let (queue, handle) = ReleaseQueue::new(1);
    let counter = Arc::new(AtomicU32::new(0));

    for _ in 0..total_tasks {
        submit_increment(&queue, &counter);
    }

    // Give workers time to drain all tasks.
    tokio::time::sleep(Duration::from_secs(2)).await;
    drop(queue);
    ReleaseQueue::shutdown(handle).await;

    assert_eq!(
        counter.load(Ordering::Relaxed),
        total_tasks,
        "all {total_tasks} tasks must complete — none should be dropped"
    );
}

#[tokio::test]
async fn close_drains_buffered_tasks_before_exit() {
    let cancel = CancellationToken::new();
    let (queue, handle) = ReleaseQueue::with_cancel(1, cancel);
    let counter = Arc::new(AtomicU32::new(0));

    for _ in 0..5 {
        submit_increment(&queue, &counter);
    }

    // Signal drain via close() without dropping the queue.
    queue.close();
    ReleaseQueue::shutdown(handle).await;

    assert_eq!(
        counter.load(Ordering::Relaxed),
        5,
        "close() must drain all buffered tasks before workers exit"
    );
}

fn submit_ready_gated(
    queue: &ReleaseQueue,
    gate: &Arc<Notify>,
    ready: oneshot::Sender<()>,
    counter: &Arc<AtomicU32>,
) {
    let g = gate.clone();
    let c = counter.clone();
    queue.submit(move || {
        Box::pin(async move {
            // Register with the gate before reporting ready: a release that
            // follows the ready signal then cannot slip past this task.
            let released = g.notified();
            tokio::pin!(released);
            released.as_mut().enable();
            let _ = ready.send(());
            released.await;
            c.fetch_add(1, Ordering::Relaxed);
        })
    });
}

/// Submits `jobs` gated tasks and returns once every one of them runs.
async fn park_gated_jobs(
    queue: &ReleaseQueue,
    gate: &Arc<Notify>,
    counter: &Arc<AtomicU32>,
    jobs: usize,
) {
    let mut started = Vec::with_capacity(jobs);
    for _ in 0..jobs {
        let (ready_tx, ready_rx) = oneshot::channel();
        submit_ready_gated(queue, gate, ready_tx, counter);
        started.push(ready_rx);
    }
    for ready in started {
        ready.await.expect("a worker must start the gated task");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn double_full_saturation_rescues_instead_of_dropping() {
    // Saturate both primary (256) and fallback (4096) by parking BOTH
    // the primary worker and the fallback worker on a gate. Once both
    // channels are full, any further submit must spawn a rescue task —
    // NOT silently drop.
    let (queue, handle) = ReleaseQueue::new(1);
    let counter = Arc::new(AtomicU32::new(0));
    let gate = Arc::new(Notify::new());

    // Step 1: hold every slot of the primary worker on the gate. A worker
    // runs up to JOBS_PER_WORKER jobs at once, so that many gated jobs are
    // needed before it stops taking from the primary channel.
    park_gated_jobs(&queue, &gate, &counter, JOBS_PER_WORKER).await;

    // Step 2: hold the fallback worker too. Fill the primary channel with
    // near-instant tasks (the saturated worker leaves them buffered) so the
    // next submits overflow into the fallback channel — the gated tasks
    // sent next are what the fallback worker picks up and blocks on.
    for _ in 0..CHANNEL_BUFFER {
        submit_increment(&queue, &counter);
    }
    park_gated_jobs(&queue, &gate, &counter, JOBS_PER_WORKER).await;

    // Step 3: now both workers are saturated and the primary channel is
    // full. The fallback channel is empty (its worker holds the gated
    // jobs, they are no longer buffered), so it takes FALLBACK_BUFFER more
    // submits; the 300 after that must be rescued.
    let extras: u32 = FALLBACK_BUFFER as u32 + 300;
    for _ in 0..extras {
        submit_increment(&queue, &counter);
    }

    // Rescue path must have been exercised at least once.
    assert!(
        queue.rescued_count() > 0,
        "rescue path must be exercised under double-full saturation \
         (fallback={}, rescued={}, dropped={})",
        queue.fallback_count(),
        queue.rescued_count(),
        queue.dropped_count(),
    );
    assert_eq!(
        queue.dropped_count(),
        0,
        "no task should be dropped — they must all be rescued and run"
    );

    // Release the gated tasks so workers can drain.
    gate.notify_waiters();

    // Wait for the counter to settle. Total expected:
    //   2 * JOBS_PER_WORKER gated tasks (steps 1 and 2)
    //   + CHANNEL_BUFFER near-instant tasks (step 2)
    //   + extras near-instant tasks (step 3)
    let expected: u32 = 2 * JOBS_PER_WORKER as u32 + CHANNEL_BUFFER as u32 + extras;
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while counter.load(Ordering::Relaxed) < expected {
        if std::time::Instant::now() > deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    drop(queue);
    ReleaseQueue::shutdown(handle).await;

    assert_eq!(
        counter.load(Ordering::Relaxed),
        expected,
        "every submitted task (including gated and rescued) must \
         complete — none should be silently dropped"
    );
}

#[tokio::test(start_paused = true)]
async fn slow_task_is_aborted_after_execution_timeout() {
    let (queue, handle) = ReleaseQueue::new(1);
    let completed = Arc::new(AtomicBool::new(false));
    let c = completed.clone();

    queue.submit(move || {
        Box::pin(async move {
            // Sleep longer than TASK_EXECUTION_TIMEOUT (the teardown ceiling).
            tokio::time::sleep(Duration::from_secs(150)).await;
            c.store(true, Ordering::Relaxed);
        })
    });

    // Advance past the task timeout.
    tokio::time::sleep(Duration::from_secs(125)).await;

    drop(queue);
    ReleaseQueue::shutdown(handle).await;

    assert!(
        !completed.load(Ordering::Relaxed),
        "slow task should have been aborted by the execution timeout"
    );
}

#[tokio::test(start_paused = true)]
async fn a_hung_job_holds_a_slot_not_its_worker() {
    // One worker: under run-to-completion the job behind a hung one would
    // wait for the whole hang.
    let (queue, handle) = ReleaseQueue::new(1);
    let (started_tx, started_rx) = oneshot::channel();
    queue.submit(move || {
        Box::pin(async move {
            let _ = started_tx.send(());
            std::future::pending::<()>().await;
        })
    });
    started_rx.await.expect("the worker starts the hung job");

    let behind = queue
        .submit_release(|| Box::pin(async { Ok(()) }))
        .expect("an open queue accepts the next job");
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), behind.wait())
            .await
            .expect("the job behind a hung one must not wait for it")
            .expect("the job behind a hung one completes"),
        SubmissionOutcome::Completed,
    );

    queue.close();
    ReleaseQueue::shutdown_bounded(handle, Duration::from_secs(1))
        .await
        .expect_err("the hung job outlives the budget and is aborted");
    assert_eq!(queue.dropped_count(), 1, "only the hung job is abandoned");
}

#[tokio::test(start_paused = true)]
async fn a_saturated_lane_buffers_until_a_slot_frees() {
    // The concurrency is bounded: with every slot held, the next job waits
    // in the primary channel rather than growing the worker.
    let (queue, handle) = ReleaseQueue::new(2);
    let parked = queue.park_entry_lane().await;
    let ran = Arc::new(AtomicBool::new(false));
    let observed = Arc::clone(&ran);
    let waiting = queue
        .submit_release(move || {
            Box::pin(async move {
                observed.store(true, Ordering::SeqCst);
                Ok(())
            })
        })
        .expect("a saturated lane still accepts into its channel");

    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(
        !ran.load(Ordering::SeqCst),
        "every slot is held: the job must stay buffered"
    );
    assert_eq!(queue.fallback_count(), 0, "the primary channel had room");

    parked.release();
    assert_eq!(
        waiting.wait().await.expect("the buffered job completes"),
        SubmissionOutcome::Completed,
    );
    queue.close();
    ReleaseQueue::shutdown(handle).await;
    assert_eq!(queue.dropped_count(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_saturated_coordinator_lane_leaves_entry_slots_free() {
    let (queue, handle) = ReleaseQueue::new(1);
    let parked = queue.park_coordinator_lane().await;
    let entry = queue
        .submit_release(|| Box::pin(async { Ok(()) }))
        .expect("the entry lane accepts cleanup");
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), entry.wait())
            .await
            .expect("entry cleanup does not wait for coordinator slots")
            .expect("entry cleanup completes"),
        SubmissionOutcome::Completed,
    );
    parked.release();
    queue.close();
    ReleaseQueue::shutdown(handle).await;
    assert_eq!(queue.dropped_count(), 0);
}

#[tokio::test(start_paused = true)]
async fn coordinator_lane_cannot_delay_entry_cleanup() {
    let (queue, handle) = ReleaseQueue::new(2);
    let ownership = Arc::new(());
    let (first_started_tx, first_started_rx) = oneshot::channel();
    let (second_started_tx, second_started_rx) = oneshot::channel();
    let (first_lease_release, first_lease) = oneshot::channel();
    let (second_lease_release, second_lease) = oneshot::channel();

    let first_owner = Arc::clone(&ownership);
    let first = queue
        .submit_coordinator(move || {
            Box::pin(async move {
                first_started_tx
                    .send(())
                    .expect("test observes first coordinator");
                let _ = first_lease.await;
                drop(first_owner);
                Ok(())
            })
        })
        .expect("open queue accepts first coordinator");
    let second_owner = Arc::clone(&ownership);
    let second = queue
        .submit_coordinator(move || {
            Box::pin(async move {
                second_started_tx
                    .send(())
                    .expect("test observes second coordinator");
                let _ = second_lease.await;
                drop(second_owner);
                Ok(())
            })
        })
        .expect("open queue accepts second coordinator");
    first_started_rx
        .await
        .expect("first primary worker starts its coordinator");
    second_started_rx
        .await
        .expect("second primary worker starts its coordinator");

    let released = Arc::new(AtomicUsize::new(0));
    let first_release_observation = Arc::clone(&released);
    let first_entry = queue
        .submit_release(move || {
            Box::pin(async move {
                let _ = first_lease_release.send(());
                first_release_observation.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        })
        .expect("first external cleanup is accepted behind saturated coordinators");
    let second_release_observation = Arc::clone(&released);
    let second_entry = queue
        .submit_release(move || {
            Box::pin(async move {
                let _ = second_lease_release.send(());
                second_release_observation.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        })
        .expect("second external cleanup is accepted behind saturated coordinators");
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), first_entry.wait())
            .await
            .expect("first entry lane stays schedulable")
            .expect("first entry cleanup runs after containment"),
        SubmissionOutcome::Completed,
    );
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), second_entry.wait())
            .await
            .expect("second entry lane stays schedulable")
            .expect("second entry cleanup runs after containment"),
        SubmissionOutcome::Completed,
    );
    assert_eq!(released.load(Ordering::SeqCst), 2);

    assert_eq!(
        first.wait().await.expect("first coordinator is unblocked"),
        SubmissionOutcome::Completed,
    );
    assert_eq!(
        second
            .wait()
            .await
            .expect("second coordinator is unblocked"),
        SubmissionOutcome::Completed,
    );
    assert_eq!(
        Arc::strong_count(&ownership),
        1,
        "completed coordinators relinquish each captured owner exactly once"
    );

    queue.close();
    ReleaseQueue::shutdown_bounded(handle, Duration::from_secs(1))
        .await
        .expect("settled queue shuts down without false completion");
}

#[tokio::test(start_paused = true)]
async fn coordinator_is_not_abandoned_by_entry_execution_ceiling() {
    let (queue, handle) = ReleaseQueue::new(1);
    let (started_tx, started_rx) = oneshot::channel();
    let (resume_tx, resume_rx) = oneshot::channel();
    let coordinator = queue
        .submit_coordinator(move || {
            Box::pin(async move {
                started_tx.send(()).expect("test observes coordinator");
                resume_rx.await.expect("test resumes coordinator");
                Ok(())
            })
        })
        .expect("open queue accepts coordinator");
    started_rx.await.expect("coordinator starts");

    tokio::time::advance(TASK_EXECUTION_TIMEOUT).await;
    resume_tx
        .send(())
        .expect("coordinator still owns its receiver after entry ceiling");
    assert_eq!(
        coordinator
            .wait()
            .await
            .expect("coordinator completes after its mandatory tail resumes"),
        SubmissionOutcome::Completed,
    );

    queue.close();
    ReleaseQueue::shutdown_bounded(handle, Duration::from_secs(1))
        .await
        .expect("settled queue shuts down");
}

#[tokio::test(start_paused = true)]
async fn worker_timeout_increments_dropped_count() {
    // A worker-path teardown that exceeds TASK_EXECUTION_TIMEOUT must be
    // counted as a true drop — otherwise a leaked resource stays invisible
    // while `dropped_count()` falsely reports a clean zero.
    let (queue, handle) = ReleaseQueue::new(1);

    queue.submit(|| {
        Box::pin(async {
            // Never completes — the guard must abort it on timeout.
            std::future::pending::<()>().await;
        })
    });

    // Advance the paused clock past the execution timeout so the guard
    // fires and the worker records the drop. Same time-control technique
    // as `slow_task_is_aborted_after_execution_timeout`.
    tokio::time::sleep(Duration::from_secs(125)).await;

    // Yield once more so the worker that woke from the timeout finishes
    // recording the drop before we observe it.
    tokio::task::yield_now().await;

    assert_eq!(
        queue.dropped_count(),
        1,
        "a worker-path teardown timeout must count as exactly one drop"
    );

    queue.close();
    ReleaseQueue::shutdown(handle).await;
}

#[tokio::test]
async fn worker_panic_increments_dropped_count() {
    // A teardown that unwinds is caught by the hook guard so the worker
    // keeps draining — but the resource it was releasing never came back,
    // so the panic must be counted as a true drop.
    let (queue, handle) = ReleaseQueue::new(1);

    queue.submit(|| {
        Box::pin(async {
            panic!("release teardown blew up");
        })
    });

    // Wait for the worker to process the panicking task and record the
    // drop. The counter is the deterministic signal — poll it rather than
    // sleeping a fixed wall-clock duration.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while queue.dropped_count() == 0 {
        if std::time::Instant::now() > deadline {
            break;
        }
        tokio::task::yield_now().await;
    }

    assert_eq!(
        queue.dropped_count(),
        1,
        "a worker-path teardown panic must count as exactly one drop"
    );

    queue.close();
    ReleaseQueue::shutdown(handle).await;
}
