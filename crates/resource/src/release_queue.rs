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

use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

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

/// Maximum lifetime of a rescue task spawned on double-`Full` saturation.
///
/// When both primary and fallback channels are full, [`ReleaseQueue::submit`]
/// spawns a short-lived task that awaits capacity on the fallback channel
/// (blocking send) for up to this window. If no worker drains within
/// `RESCUE_TIMEOUT`, the task is recorded as truly dropped — an explicit,
/// metric-observable loss rather than a silent one. This bound also caps
/// the total lifetime of any rescue task so they cannot leak indefinitely.
const RESCUE_TIMEOUT: Duration = Duration::from_secs(30);

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
/// 1. **Via cancellation token** (preferred for [`Manager`](crate::Manager)): cancel the shared
///    token → workers drain buffered tasks and exit.
/// 2. **Via drop** (for standalone use): drop the `ReleaseQueue` → senders close → workers see
///    `None` and exit.
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
    /// Tracks how many tasks were dropped due to full queues
    /// (rescue timeout or shutdown).
    ///
    /// Shared with rescue tasks via `Arc` so they can record terminal
    /// drops from outside the queue.
    dropped_count: Arc<AtomicUsize>,
    /// Tracks how many tasks were sent down the rescue path
    /// (double-`Full` saturation). A non-zero value means the queue is
    /// saturated badly enough that `try_send` to both primary and fallback
    /// failed — operators should investigate worker capacity.
    rescued_count: Arc<AtomicUsize>,
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
            dropped_count: Arc::new(AtomicUsize::new(0)),
            rescued_count: Arc::new(AtomicUsize::new(0)),
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
                    Err(mpsc::error::TrySendError::Full(factory)) => {
                        // Both primary and fallback are full. Previously this
                        // path silently dropped the task. Now we hand it to a
                        // bounded-lifetime rescue task that awaits capacity on
                        // the fallback channel for up to `RESCUE_TIMEOUT`.
                        self.spawn_rescue(factory);
                    }
                    Err(mpsc::error::TrySendError::Closed(factory)) => {
                        // Fallback channel is closed — workers have exited.
                        // Record as a drop (with reason) instead of silently
                        // discarding. The factory is dropped here on purpose:
                        // there is nowhere to send it.
                        drop(factory);
                        record_drop(&self.dropped_count, "fallback_channel_closed");
                    }
                }
            }
            Err(mpsc::error::TrySendError::Closed(factory)) => {
                // Primary worker exited (e.g., panic). Try the fallback
                // before recording a drop — fallback may still be alive.
                match self.fallback_tx.try_send(factory) {
                    Ok(()) => {}
                    Err(mpsc::error::TrySendError::Full(factory)) => {
                        self.spawn_rescue(factory);
                    }
                    Err(mpsc::error::TrySendError::Closed(factory)) => {
                        drop(factory);
                        record_drop(&self.dropped_count, "primary_and_fallback_closed");
                    }
                }
            }
        }
    }

    /// Spawns a bounded-lifetime rescue task for a release that lost the
    /// `try_send` race on both primary and fallback channels.
    ///
    /// The rescue task awaits capacity on the fallback channel via blocking
    /// `send` for up to [`RESCUE_TIMEOUT`]. If the queue is cancelled or the
    /// timeout expires, the task is recorded via `dropped_count` — the only
    /// path that counts toward dropped tasks, and one that is bounded and
    /// observable. The task is fire-and-forget by design: its purpose is to
    /// survive without a caller handle, and the timeout caps its total
    /// lifetime so it cannot leak indefinitely.
    fn spawn_rescue(&self, factory: TaskFactory) {
        let rescued = self.rescued_count.fetch_add(1, Ordering::Relaxed) + 1;
        if rescued.is_power_of_two() {
            tracing::warn!(
                rescued_tasks = rescued,
                "release queue saturated (primary + fallback full); \
                 spawning bounded-lifetime rescue task"
            );
        }

        let fallback_tx = self.fallback_tx.clone();
        let cancel = self.cancel.clone();
        let dropped = Arc::clone(&self.dropped_count);

        tokio::spawn(async move {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => {
                    // Shutdown signalled while we were waiting for capacity.
                    // The drain loop in worker_loop will not see us — record
                    // the drop and exit.
                    record_drop(&dropped, "shutdown");
                }
                res = tokio::time::timeout(RESCUE_TIMEOUT, fallback_tx.send(factory)) => {
                    match res {
                        Ok(Ok(())) => {
                            // Rescued: a worker drained the fallback in time.
                        }
                        Ok(Err(_closed)) => {
                            record_drop(&dropped, "channel_closed");
                        }
                        Err(_elapsed) => {
                            record_drop(&dropped, "timeout");
                        }
                    }
                }
            }
        });
    }

    /// Returns the total number of tasks routed via the fallback channel.
    pub fn fallback_count(&self) -> usize {
        self.fallback_count.load(Ordering::Relaxed)
    }

    /// Returns the number of tasks that were truly dropped (after rescue
    /// failure or shutdown). A non-zero value means resources may have
    /// leaked and warrants operator attention.
    pub fn dropped_count(&self) -> usize {
        self.dropped_count.load(Ordering::Relaxed)
    }

    /// Returns the number of tasks that entered the rescue path due to
    /// double-`Full` saturation. A non-zero value means the queue was
    /// saturated badly enough that both primary and fallback `try_send`
    /// failed — operators should investigate worker capacity even if
    /// `dropped_count()` is still zero.
    pub fn rescued_count(&self) -> usize {
        self.rescued_count.load(Ordering::Relaxed)
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

/// Records a terminal task drop on the shared counter, with a structured
/// reason. Logs at ERROR level when the drop count crosses a power-of-two
/// boundary so log volume stays bounded under sustained loss.
fn record_drop(counter: &Arc<AtomicUsize>, reason: &'static str) {
    let n = counter.fetch_add(1, Ordering::Relaxed) + 1;
    if n.is_power_of_two() {
        tracing::error!(
            dropped_tasks = n,
            reason = reason,
            "release queue rescue failed — resource may leak"
        );
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    };

    use super::*;

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

    fn submit_gated(
        queue: &ReleaseQueue,
        gate: &Arc<tokio::sync::Notify>,
        counter: &Arc<AtomicU32>,
    ) {
        let g = gate.clone();
        let c = counter.clone();
        queue.submit(move || Box::pin(gated_increment(g, c)));
    }

    async fn gated_increment(gate: Arc<tokio::sync::Notify>, counter: Arc<AtomicU32>) {
        gate.notified().await;
        counter.fetch_add(1, Ordering::Relaxed);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn double_full_saturation_rescues_instead_of_dropping() {
        // Saturate both primary (256) and fallback (4096) by parking BOTH
        // the primary worker and the fallback worker on a gate. Once both
        // channels are full, any further submit must spawn a rescue task —
        // NOT silently drop.
        let (queue, handle) = ReleaseQueue::new(1);
        let counter = Arc::new(AtomicU32::new(0));
        let gate = Arc::new(tokio::sync::Notify::new());

        // Step 1: park the primary worker on the gate. The first submit
        // routes to senders[0] (round-robin with 1 worker). The primary
        // worker pulls it via `recv()` and blocks on `notified()`.
        submit_gated(&queue, &gate, &counter);
        // Yield long enough for the primary worker to actually receive
        // and start the gated task.
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Step 2: park the fallback worker too. Fill primary first with
        // near-instant tasks so the next submit overflows into the
        // fallback channel — and the gated task we send next is what
        // the fallback worker picks up and blocks on.
        for _ in 0..CHANNEL_BUFFER {
            submit_increment(&queue, &counter);
        }
        // Primary is now full (256 buffered, 1 in-flight on the worker).
        // Next submit overflows to the fallback channel.
        submit_gated(&queue, &gate, &counter);
        // Let the fallback worker pick up the gated task and block.
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Step 3: now both workers are blocked. Flood until both channels
        // are completely full and rescue must kick in. Capacity:
        //   primary buffer  = 256 (full from step 2)
        //   fallback buffer = 4096 (1 already used by the gated task that
        //                          the fallback worker is now holding;
        //                          the gated task is no longer in the
        //                          buffer, so 4096 free slots remain)
        // Submitting (256 already filled - we re-fill primary as workers
        // are gated) plus 4096 to fallback = 4352 buffered before rescue.
        // After step 2, primary buffer is still ~256 but the 257th went
        // to fallback. So available room: primary 0, fallback 4096.
        // Add a margin: 4096 + 300 forces 300 rescues.
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

        // Release both gated tasks so workers can drain.
        gate.notify_waiters();

        // Wait for the counter to settle. Total expected:
        //   2 gated tasks
        //   + CHANNEL_BUFFER near-instant tasks (step 2)
        //   + extras near-instant tasks (step 3)
        let expected: u32 = 2 + CHANNEL_BUFFER as u32 + extras;
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
