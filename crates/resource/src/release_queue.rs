//! Background release queue for async cleanup tasks.
//!
//! [`ReleaseQueue`] distributes cleanup work (e.g., returning connections to a
//! pool, destroying tainted leases) across N primary workers and one fallback
//! worker. Tasks are round-robin distributed to primary workers; if a primary
//! channel is full, the task falls back to the overflow channel.
//!
//! # Shutdown
//!
//! Workers exit when either:
//! - The [`CancellationToken`] is cancelled (drain remaining tasks, then exit).
//! - All senders are dropped (channel returns `None`).
//!
//! When used via [`Manager`](crate::Manager), the manager's cancellation token
//! is shared with workers, so `Manager::graceful_shutdown` automatically
//! signals workers to drain without needing to drop the queue.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// A boxed, pinned, sendable future that returns `()`.
type ReleaseTask = Pin<Box<dyn Future<Output = ()> + Send>>;

/// A factory that produces a [`ReleaseTask`] when called.
///
/// We use a factory so that the future is not polled until a worker picks
/// it up, keeping the submit path non-async.
type TaskFactory = Box<dyn FnOnce() -> ReleaseTask + Send>;

/// Maximum time a single release task may execute before being aborted.
const TASK_EXECUTION_TIMEOUT: Duration = Duration::from_secs(30);

/// Channel buffer size per primary worker.
const CHANNEL_BUFFER: usize = 256;

/// Channel buffer size for the fallback worker.
///
/// Previously unbounded — now bounded to prevent OOM under sustained overload.
/// Tasks exceeding this capacity are dropped with a warning.
const FALLBACK_BUFFER: usize = 4096;

/// Handle to the running release queue workers.
///
/// Must be passed to [`ReleaseQueue::shutdown`] for graceful termination.
#[must_use = "dropping ReleaseQueueHandle without shutdown leaks worker tasks"]
pub struct ReleaseQueueHandle {
    workers: Vec<tokio::task::JoinHandle<()>>,
    fallback_worker: tokio::task::JoinHandle<()>,
}

/// Distributes async release tasks across a pool of background workers.
///
/// # Shutdown
///
/// There are two ways to shut down the queue:
///
/// 1. **Via cancellation token** (preferred for [`Manager`](crate::Manager)):
///    cancel the shared token → workers drain buffered tasks and exit.
/// 2. **Via drop** (for standalone use): drop the `ReleaseQueue` → senders
///    close → workers see `None` and exit.
///
/// In both cases, call [`ReleaseQueue::shutdown`] afterward to await workers.
///
/// # Examples
///
/// ```no_run
/// # async fn example() {
/// use nebula_resource::ReleaseQueue;
///
/// let (queue, handle) = ReleaseQueue::new(2);
/// queue.submit(|| Box::pin(async { /* cleanup */ }));
/// drop(queue);
/// ReleaseQueue::shutdown(handle).await;
/// # }
/// ```
pub struct ReleaseQueue {
    senders: Vec<mpsc::Sender<TaskFactory>>,
    fallback_tx: mpsc::Sender<TaskFactory>,
    next: AtomicUsize,
    cancel: CancellationToken,
    /// Tracks how many tasks have gone to the fallback channel.
    fallback_count: AtomicUsize,
    /// Tracks how many tasks were dropped due to full queues.
    dropped_count: AtomicUsize,
}

impl ReleaseQueue {
    /// Creates a new release queue with `worker_count` primary workers
    /// and its own internal cancellation token.
    ///
    /// Returns the queue (for submitting tasks) and a handle (for shutdown).
    ///
    /// # Panics
    ///
    /// Panics if `worker_count` is zero.
    pub fn new(worker_count: usize) -> (Self, ReleaseQueueHandle) {
        Self::with_cancel(worker_count, CancellationToken::new())
    }

    /// Creates a new release queue with a shared cancellation token.
    ///
    /// When the token is cancelled, workers drain remaining buffered tasks
    /// and exit — without requiring the senders to be dropped. This is the
    /// mechanism used by [`Manager::graceful_shutdown`](crate::Manager::graceful_shutdown).
    ///
    /// # Panics
    ///
    /// Panics if `worker_count` is zero.
    pub fn with_cancel(
        worker_count: usize,
        cancel: CancellationToken,
    ) -> (Self, ReleaseQueueHandle) {
        assert!(worker_count > 0, "worker_count must be at least 1");

        let mut senders = Vec::with_capacity(worker_count);
        let mut workers = Vec::with_capacity(worker_count);

        for _ in 0..worker_count {
            let (tx, rx) = mpsc::channel::<TaskFactory>(CHANNEL_BUFFER);
            senders.push(tx);
            workers.push(tokio::spawn(Self::worker_loop(rx, cancel.clone())));
        }

        let (fallback_tx, fallback_rx) = mpsc::channel::<TaskFactory>(FALLBACK_BUFFER);
        let fallback_worker = tokio::spawn(Self::worker_loop(fallback_rx, cancel.clone()));

        let queue = Self {
            senders,
            fallback_tx,
            next: AtomicUsize::new(0),
            cancel,
            fallback_count: AtomicUsize::new(0),
            dropped_count: AtomicUsize::new(0),
        };
        let handle = ReleaseQueueHandle {
            workers,
            fallback_worker,
        };

        (queue, handle)
    }

    /// Submits a release task to the queue.
    ///
    /// The factory is called by a worker to produce the actual future.
    /// If the round-robin primary worker's channel is full, the task
    /// goes to the fallback channel.
    pub fn submit(&self, factory: impl FnOnce() -> ReleaseTask + Send + 'static) {
        let idx = self.next.fetch_add(1, Ordering::Relaxed) % self.senders.len();
        let factory: TaskFactory = Box::new(factory);

        match self.senders[idx].try_send(factory) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(factory)) => {
                // Primary is full — try bounded fallback.
                let count = self.fallback_count.fetch_add(1, Ordering::Relaxed) + 1;
                if count.is_power_of_two() {
                    tracing::warn!(
                        fallback_tasks = count,
                        "release queue primary channels full, using fallback"
                    );
                }
                match self.fallback_tx.try_send(factory) {
                    Ok(()) => {}
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        let dropped = self.dropped_count.fetch_add(1, Ordering::Relaxed) + 1;
                        if dropped.is_power_of_two() {
                            tracing::error!(
                                dropped_tasks = dropped,
                                "release queue fallback full — dropping release \
                                 task (resource may leak)"
                            );
                        }
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => {
                        tracing::warn!(
                            "release queue fallback channel closed, \
                             dropping release task"
                        );
                    }
                }
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                tracing::warn!("release queue primary worker channel closed");
            }
        }
    }

    /// Signals workers to drain remaining tasks and exit.
    ///
    /// This is equivalent to cancelling the token passed to
    /// [`with_cancel`](Self::with_cancel). Call before [`shutdown`](Self::shutdown)
    /// for prompt worker exit without needing to drop the queue.
    pub fn close(&self) {
        self.cancel.cancel();
    }

    /// Shuts down all workers gracefully, waiting for in-flight tasks.
    ///
    /// Workers must have been signaled to stop first — either by dropping
    /// the `ReleaseQueue` (closing channels) or by cancelling the token
    /// (via [`close`](Self::close) or external cancellation).
    pub async fn shutdown(handle: ReleaseQueueHandle) {
        for worker in handle.workers {
            let _ = worker.await;
        }
        let _ = handle.fallback_worker.await;
    }

    /// Worker loop for bounded primary channels.
    ///
    /// Uses `select!` with `biased` to prefer processing messages over
    /// checking cancellation — ensuring buffered tasks are drained before
    /// the worker exits.
    async fn worker_loop(mut rx: mpsc::Receiver<TaskFactory>, cancel: CancellationToken) {
        loop {
            tokio::select! {
                biased;
                msg = rx.recv() => {
                    match msg {
                        Some(factory) => Self::execute_task(factory).await,
                        None => break, // channel closed
                    }
                }
                _ = cancel.cancelled() => {
                    // Drain remaining buffered tasks, then exit.
                    rx.close();
                    while let Some(factory) = rx.recv().await {
                        Self::execute_task(factory).await;
                    }
                    break;
                }
            }
        }
    }

    async fn execute_task(factory: TaskFactory) {
        let task = factory();
        if tokio::time::timeout(TASK_EXECUTION_TIMEOUT, task)
            .await
            .is_err()
        {
            tracing::warn!(
                "release task timed out after {}s, skipping",
                TASK_EXECUTION_TIMEOUT.as_secs()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

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
    #[should_panic(expected = "worker_count must be at least 1")]
    async fn zero_workers_panics() {
        let _ = ReleaseQueue::new(0);
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

    #[tokio::test(start_paused = true)]
    async fn slow_task_is_aborted_after_execution_timeout() {
        let (queue, handle) = ReleaseQueue::new(1);
        let completed = Arc::new(AtomicBool::new(false));
        let c = completed.clone();

        queue.submit(move || {
            Box::pin(async move {
                // Sleep longer than TASK_EXECUTION_TIMEOUT (30s).
                tokio::time::sleep(Duration::from_secs(60)).await;
                c.store(true, Ordering::Relaxed);
            })
        });

        // Advance past the task timeout.
        tokio::time::sleep(Duration::from_secs(35)).await;

        drop(queue);
        ReleaseQueue::shutdown(handle).await;

        assert!(
            !completed.load(Ordering::Relaxed),
            "slow task should have been aborted by the execution timeout"
        );
    }
}
