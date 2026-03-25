//! Background release queue for async cleanup tasks.
//!
//! [`ReleaseQueue`] distributes cleanup work (e.g., returning connections to a
//! pool, destroying tainted leases) across N primary workers and one fallback
//! worker. Tasks are round-robin distributed to primary workers; if a primary
//! channel is full, the task falls back to the overflow channel.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use tokio::sync::mpsc;

/// A boxed, pinned, sendable future that returns `()`.
type ReleaseTask = Pin<Box<dyn Future<Output = ()> + Send>>;

/// A factory that produces a [`ReleaseTask`] when called.
///
/// We use a factory so that the future is not polled until a worker picks
/// it up, keeping the submit path non-async.
type TaskFactory = Box<dyn FnOnce() -> ReleaseTask + Send>;

/// Worker timeout — how long a worker waits for the next task before
/// checking for shutdown.
const WORKER_RECV_TIMEOUT: Duration = Duration::from_secs(30);

/// Maximum time a single release task may execute before being aborted.
const TASK_EXECUTION_TIMEOUT: Duration = Duration::from_secs(30);

/// Channel buffer size per primary worker.
const CHANNEL_BUFFER: usize = 256;

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
/// # Examples
///
/// ```no_run
/// # async fn example() {
/// use nebula_resource::ReleaseQueue;
///
/// let (queue, handle) = ReleaseQueue::new(2);
/// queue.submit(|| Box::pin(async { /* cleanup */ }));
/// ReleaseQueue::shutdown(handle).await;
/// # }
/// ```
pub struct ReleaseQueue {
    senders: Vec<mpsc::Sender<TaskFactory>>,
    fallback_tx: mpsc::UnboundedSender<TaskFactory>,
    next: AtomicUsize,
}

impl ReleaseQueue {
    /// Creates a new release queue with `worker_count` primary workers.
    ///
    /// Returns the queue (for submitting tasks) and a handle (for shutdown).
    /// Panics if `worker_count` is zero.
    pub fn new(worker_count: usize) -> (Self, ReleaseQueueHandle) {
        assert!(worker_count > 0, "worker_count must be at least 1");

        let mut senders = Vec::with_capacity(worker_count);
        let mut workers = Vec::with_capacity(worker_count);

        for _ in 0..worker_count {
            let (tx, rx) = mpsc::channel::<TaskFactory>(CHANNEL_BUFFER);
            senders.push(tx);
            workers.push(tokio::spawn(Self::worker_loop(rx)));
        }

        let (fallback_tx, fallback_rx) = mpsc::unbounded_channel::<TaskFactory>();
        let fallback_worker = tokio::spawn(Self::worker_loop_unbounded(fallback_rx));

        let queue = Self {
            senders,
            fallback_tx,
            next: AtomicUsize::new(0),
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
                // Primary is full — use unbounded fallback (never drops).
                if let Err(e) = self.fallback_tx.send(factory) {
                    tracing::warn!(
                        "release queue fallback channel closed, \
                         dropping release task: {e}"
                    );
                }
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                tracing::warn!("release queue primary worker channel closed");
            }
        }
    }

    /// Shuts down all workers gracefully, waiting for in-flight tasks.
    pub async fn shutdown(handle: ReleaseQueueHandle) {
        // Drop senders is done by the caller dropping the ReleaseQueue.
        // Here we just await the workers.
        for worker in handle.workers {
            let _ = worker.await;
        }
        let _ = handle.fallback_worker.await;
    }

    async fn worker_loop(mut rx: mpsc::Receiver<TaskFactory>) {
        loop {
            match tokio::time::timeout(WORKER_RECV_TIMEOUT, rx.recv()).await {
                Ok(Some(factory)) => {
                    Self::execute_task(factory).await;
                }
                Ok(None) => break,  // channel closed
                Err(_) => continue, // timeout, loop back
            }
        }
    }

    async fn worker_loop_unbounded(mut rx: mpsc::UnboundedReceiver<TaskFactory>) {
        loop {
            match tokio::time::timeout(WORKER_RECV_TIMEOUT, rx.recv()).await {
                Ok(Some(factory)) => {
                    Self::execute_task(factory).await;
                }
                Ok(None) => break,  // channel closed
                Err(_) => continue, // timeout, loop back
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

    #[tokio::test]
    async fn submit_and_execute() {
        let (queue, handle) = ReleaseQueue::new(2);
        let counter = Arc::new(AtomicU32::new(0));

        for _ in 0..10 {
            let c = counter.clone();
            queue.submit(move || {
                Box::pin(async move {
                    c.fetch_add(1, Ordering::Relaxed);
                })
            });
        }

        // Give workers time to process.
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(counter.load(Ordering::Relaxed), 10);

        drop(queue);
        ReleaseQueue::shutdown(handle).await;
    }

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

    use std::sync::atomic::AtomicBool;

    #[tokio::test]
    #[should_panic(expected = "worker_count must be at least 1")]
    async fn zero_workers_panics() {
        let _ = ReleaseQueue::new(0);
    }

    #[tokio::test]
    async fn fallback_channel_never_drops_tasks() {
        // Use 1 worker so primary channel has 256 capacity.
        // Submit more tasks than old total capacity (256 + 1024 = 1280).
        let total_tasks: u32 = 1500;
        let (queue, handle) = ReleaseQueue::new(1);
        let counter = Arc::new(AtomicU32::new(0));

        for _ in 0..total_tasks {
            let c = counter.clone();
            queue.submit(move || {
                Box::pin(async move {
                    c.fetch_add(1, Ordering::Relaxed);
                })
            });
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
}
