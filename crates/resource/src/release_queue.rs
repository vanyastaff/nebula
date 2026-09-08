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
//! [`Manager`](crate::Manager) keeps cleanup open while its guards drain,
//! then closes the queue and awaits workers within a cooperative budget.

use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

/// A boxed, pinned, sendable future that returns `()`.
type ReleaseTask = Pin<Box<dyn Future<Output = ()> + Send>>;

/// A fallible framework cleanup job; public unit-returning tasks are adapted here.
type JobFuture = Pin<Box<dyn Future<Output = Result<(), crate::Error>> + Send>>;
/// Future construction and polling both happen inside the worker fault boundary.
type TaskFactory = Box<dyn FnOnce() -> JobFuture + Send>;
pub(crate) type ReleaseReceipt = oneshot::Receiver<Result<(), crate::Error>>;

/// Tracks a submission until its future completes, including unpolled drops.
struct QueuedTask {
    factory: TaskFactory,
    loss: TaskLoss,
    receipt: Option<oneshot::Sender<Result<(), crate::Error>>>,
}

impl QueuedTask {
    fn abandon(mut self, reason: &'static str) {
        self.loss.reason = Some(reason);
        if let Some(receipt) = self.receipt.take() {
            let _ = receipt.send(Err(crate::Error::cancelled()));
        }
    }
}

struct TaskLoss {
    counter: Arc<AtomicUsize>,
    reason: Option<&'static str>,
}

impl Drop for TaskLoss {
    fn drop(&mut self) {
        if let Some(reason) = self.reason {
            record_drop(&self.counter, reason);
        }
    }
}

/// Maximum time a single release task may execute before being aborted.
///
/// This is the teardown-path catch-all backstop (ADR-0093): the effective bound
/// is the per-resource `timeout_at(cx.deadline)` the `Provider::destroy` future
/// already carries (composed from `Provider::teardown_budget`). This outer
/// ceiling must always sit above the largest composed deadline so it never
/// undercuts a resource declaring a budget larger than the legacy 30s — it only
/// trips on a truly wedged framework future. Mirrors the
/// [`MAX_TEARDOWN_CEILING`](crate::hook_guard::MAX_TEARDOWN_CEILING) used on the
/// awaited-`release()` path.
const TASK_EXECUTION_TIMEOUT: Duration = crate::hook_guard::MAX_TEARDOWN_CEILING;

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
}

/// Manager shutdown owns workers until completion or acknowledged abort.
struct AbortWorkers(ReleaseQueueHandle);

impl Drop for AbortWorkers {
    fn drop(&mut self) {
        for worker in &self.0.workers {
            worker.abort();
        }
    }
}

impl std::fmt::Debug for ReleaseQueueHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReleaseQueueHandle")
            .field("workers", &self.workers.len())
            .finish()
    }
}

/// Distributes async release tasks across a pool of background workers.
///
/// # Shutdown
///
/// There are two ways to shut down the queue:
///
/// 1. **Via cancellation token**: cancel the queue's
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
#[must_use = "dropping the queue closes its channels — keep it alive to submit work"]
pub struct ReleaseQueue {
    senders: Vec<mpsc::Sender<QueuedTask>>,
    fallback_tx: mpsc::Sender<QueuedTask>,
    next: AtomicUsize,
    cancel: CancellationToken,
    /// Tracks how many tasks have gone to the fallback channel.
    fallback_count: AtomicUsize,
    /// Tracks how many tasks were dropped due to full queues (rescue
    /// timeout or shutdown) **or** abandoned on the worker path when a
    /// release teardown timed out or panicked.
    ///
    /// Shared with rescue tasks and workers via `Arc` so they can record
    /// terminal drops from outside the queue.
    dropped_count: Arc<AtomicUsize>,
    /// Tracks how many tasks were sent down the rescue path
    /// (double-`Full` saturation). A non-zero value means the queue is
    /// saturated badly enough that `try_send` to both primary and fallback
    /// failed — operators should investigate worker capacity.
    rescued_count: Arc<AtomicUsize>,
}

impl std::fmt::Debug for ReleaseQueue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReleaseQueue")
            .field("worker_count", &self.senders.len())
            .field(
                "fallback_count",
                &self.fallback_count.load(Ordering::Relaxed),
            )
            .field("dropped_count", &self.dropped_count.load(Ordering::Relaxed))
            .field("rescued_count", &self.rescued_count.load(Ordering::Relaxed))
            .finish()
    }
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
    /// and exit without requiring the senders to be dropped. Keep this token
    /// independent from acquisition cancellation when late guards still need
    /// to submit cleanup.
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
        let mut workers = Vec::with_capacity(worker_count + 1);

        let dropped_count = Arc::new(AtomicUsize::new(0));

        for _ in 0..worker_count {
            let (tx, rx) = mpsc::channel::<QueuedTask>(CHANNEL_BUFFER);
            senders.push(tx);
            workers.push(tokio::spawn(Self::worker_loop(rx, cancel.clone())));
        }

        let (fallback_tx, fallback_rx) = mpsc::channel::<QueuedTask>(FALLBACK_BUFFER);
        workers.push(tokio::spawn(Self::worker_loop(fallback_rx, cancel.clone())));

        let queue = Self {
            senders,
            fallback_tx,
            next: AtomicUsize::new(0),
            cancel,
            fallback_count: AtomicUsize::new(0),
            dropped_count,
            rescued_count: Arc::new(AtomicUsize::new(0)),
        };
        let handle = ReleaseQueueHandle { workers };

        (queue, handle)
    }

    /// Submits a release task to the queue.
    ///
    /// The factory is called by a worker to produce the actual future.
    /// If the round-robin primary worker's channel is full, the task
    /// goes to the fallback channel.
    pub fn submit(&self, factory: impl FnOnce() -> ReleaseTask + Send + 'static) {
        self.enqueue(
            Box::new(move || {
                let task = factory();
                Box::pin(async move {
                    task.await;
                    Ok(())
                })
            }),
            None,
        );
    }

    pub(crate) fn submit_release(
        &self,
        factory: impl FnOnce() -> JobFuture + Send + 'static,
    ) -> ReleaseReceipt {
        let (receipt, receiver) = oneshot::channel();
        self.enqueue(Box::new(factory), Some(receipt));
        receiver
    }

    fn enqueue(
        &self,
        factory: TaskFactory,
        receipt: Option<oneshot::Sender<Result<(), crate::Error>>>,
    ) {
        let idx = self.next.fetch_add(1, Ordering::Relaxed) % self.senders.len();
        let factory = QueuedTask {
            factory,
            receipt,
            loss: TaskLoss {
                counter: Arc::clone(&self.dropped_count),
                reason: Some("abandoned"),
            },
        };
        if self.cancel.is_cancelled() {
            factory.abandon("queue_closed");
            return;
        }

        match self.senders[idx].try_send(factory) {
            Ok(()) => {},
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
                    Ok(()) => {},
                    Err(mpsc::error::TrySendError::Full(factory)) => {
                        // Both primary and fallback are full. Previously this
                        // path silently dropped the task. Now we hand it to a
                        // bounded-lifetime rescue task that awaits capacity on
                        // the fallback channel for up to `RESCUE_TIMEOUT`.
                        self.spawn_rescue(factory);
                    },
                    Err(mpsc::error::TrySendError::Closed(factory)) => {
                        // Fallback channel is closed — workers have exited.
                        // Record as a drop (with reason) instead of silently
                        // discarding. The factory is dropped here on purpose:
                        // there is nowhere to send it.
                        factory.abandon("fallback_channel_closed");
                    },
                }
            },
            Err(mpsc::error::TrySendError::Closed(factory)) => {
                // Primary worker exited (e.g., panic). Try the fallback
                // before recording a drop — fallback may still be alive.
                match self.fallback_tx.try_send(factory) {
                    Ok(()) => {},
                    Err(mpsc::error::TrySendError::Full(factory)) => {
                        self.spawn_rescue(factory);
                    },
                    Err(mpsc::error::TrySendError::Closed(factory)) => {
                        factory.abandon("primary_and_fallback_closed");
                    },
                }
            },
        }
    }

    /// Spawns a bounded-lifetime rescue task for a release that lost the
    /// `try_send` race on both primary and fallback channels.
    ///
    /// The rescue task asynchronously reserves fallback capacity for up to
    /// [`RESCUE_TIMEOUT`]. Cancellation, timeout, or receiver closure drops
    /// its owned submission and records the loss. It is detached by design:
    /// its purpose is to
    /// survive without a caller handle, and the timeout caps its total
    /// lifetime so it cannot leak indefinitely.
    fn spawn_rescue(&self, factory: QueuedTask) {
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

        tokio::spawn(async move {
            tokio::select! {
                biased;
                () = cancel.cancelled() => {
                    factory.abandon("rescue_shutdown");
                }
                res = tokio::time::timeout(RESCUE_TIMEOUT, fallback_tx.reserve()) => {
                    match res {
                        Ok(Ok(permit)) => { permit.send(factory); }
                        Ok(Err(_closed)) => factory.abandon("rescue_channel_closed"),
                        Err(_elapsed) => factory.abandon("rescue_timeout"),
                    }
                }
            }
        });
    }

    /// Returns the total number of tasks routed via the fallback channel.
    pub fn fallback_count(&self) -> usize {
        self.fallback_count.load(Ordering::Relaxed)
    }

    /// Returns the cumulative number of submitted futures abandoned before
    /// completion, including rejected submissions, rescue failure, discarded
    /// buffers, worker aborts, factory panics, future panics, and timeouts.
    /// A non-zero value warrants operator attention. Zero does not prove
    /// provider success: these futures return `()` and can handle errors
    /// internally. Detached rescue tasks may update this count later.
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
    /// Subsequent submissions are rejected and counted as dropped tasks.
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
    ///
    /// # Cancel safety
    ///
    /// This method is cancel safe. If dropped while awaiting a worker's
    /// `JoinHandle`, that worker (and any not yet reached) is not aborted —
    /// it keeps draining and exits on its own once the shared cancellation
    /// token fires. The caller only loses the "all workers finished"
    /// observation, not correctness.
    pub async fn shutdown(handle: ReleaseQueueHandle) {
        for worker in handle.workers {
            let _ = worker.await;
        }
    }

    /// Unlike public best-effort shutdown, the manager aborts unfinished work.
    #[tracing::instrument(skip(handle), fields(worker_count = handle.workers.len()))]
    pub(crate) async fn shutdown_bounded(
        handle: ReleaseQueueHandle,
        timeout: Duration,
    ) -> Result<(), crate::manager::ShutdownError> {
        let mut owned = AbortWorkers(handle);
        let mut joined_workers = 0;
        let join_workers = async {
            for worker in &mut owned.0.workers {
                let result = worker.await;
                // A JoinHandle must never be polled again after returning Ready,
                // including a failed join.
                joined_workers += 1;
                result?;
            }
            Ok::<(), tokio::task::JoinError>(())
        };
        // One timeout covers the entire join, and Tokio handles durations whose
        // deadline cannot be represented without overflowing Instant.
        let error = match tokio::time::timeout(timeout, join_workers).await {
            Ok(Ok(())) => return Ok(()),
            Ok(Err(_)) => crate::manager::ShutdownError::ReleaseQueueWorkerFailed,
            Err(_) => crate::manager::ShutdownError::ReleaseQueueTimeout { timeout },
        };
        tracing::warn!(joined_workers, error = %error, "release queue shutdown failed; aborting workers");
        for worker in &owned.0.workers {
            worker.abort();
        }
        for worker in &mut owned.0.workers[joined_workers..] {
            let _ = worker.await;
        }
        Err(error)
    }

    /// Worker loop for bounded primary channels.
    ///
    /// Uses `select!` with `biased` to prefer processing messages over
    /// checking cancellation — ensuring buffered tasks are drained before
    /// the worker exits.
    async fn worker_loop(mut rx: mpsc::Receiver<QueuedTask>, cancel: CancellationToken) {
        loop {
            tokio::select! {
                biased;
                msg = rx.recv() => {
                    match msg {
                        Some(factory) => Self::execute_task(factory).await,
                        None => break, // channel closed
                    }
                }
                () = cancel.cancelled() => {
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

    async fn execute_task(queued: QueuedTask) {
        let QueuedTask {
            factory,
            mut loss,
            receipt,
        } = queued;
        // Foolproofing: a release task runs a third-party topology's
        // `on_release` / `Provider::destroy`. The shared author-hook guard
        // bounds it (timeout) AND isolates a panic so one careless or hostile
        // hook can neither stall nor kill this worker — the queue keeps draining
        // every other slot.
        //
        // SAFETY (unwind): `factory()` builds a self-contained teardown future
        // that owns its slot; the worker holds no alias to it, so a caught panic
        // drops the owned slot and the worker loops to the next queued task — no
        // shared queue state is torn. The outer `catch_unwind` also catches a
        // panic in `factory()` itself (the closure that builds the future),
        // not just in polling the returned future.
        let task = if let Ok(task) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(factory))
        {
            task
        } else {
            tracing::error!("release task factory panicked during future construction — isolated");
            loss.reason = Some("factory_panic");
            if let Some(receipt) = receipt {
                let _ = receipt.send(Err(crate::Error::permanent(
                    "release task factory panicked",
                )));
            }
            return;
        };
        let result = match crate::hook_guard::guard_author_hook(TASK_EXECUTION_TIMEOUT, task).await
        {
            Ok(result) => {
                loss.reason = None;
                if let Err(error) = &result {
                    tracing::warn!(
                        error.kind = ?error.kind(),
                        resource.key = ?error.resource_key(),
                        "release task completed with a provider error"
                    );
                }
                result
            },
            Err(crate::hook_guard::HookFault::Panicked) => {
                tracing::error!(
                    "release task panicked — isolated; the release worker keeps draining"
                );
                // A teardown that unwound never returned its resource — count it
                // as a true drop so `dropped_count` keeps its leak invariant
                // honest instead of staying silently zero.
                loss.reason = Some("worker_panic");
                Err(crate::Error::permanent("release task panicked"))
            },
            Err(crate::hook_guard::HookFault::TimedOut) => {
                tracing::warn!(
                    "release task timed out after {}s, skipping",
                    TASK_EXECUTION_TIMEOUT.as_secs()
                );
                // A teardown abandoned on timeout may have leaked its resource —
                // count it as a true drop so the leak invariant stays honest.
                loss.reason = Some("worker_timeout");
                Err(crate::Error::cancelled())
            },
        };
        if let Some(receipt) = receipt {
            let _ = receipt.send(result);
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
            "release task dropped — resource may leak"
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
        let started = Arc::new(tokio::sync::Notify::new());
        let lease = Arc::clone(&ownership);
        let entered = Arc::clone(&started);
        queue.submit(move || {
            Box::pin(async move {
                entered.notify_one();
                std::future::pending::<()>().await;
                drop(lease);
            })
        });
        started.notified().await;
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
        assert_eq!(queue.dropped_count(), CHANNEL_BUFFER + 2);
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
}
