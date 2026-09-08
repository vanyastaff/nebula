//! Background release queue for async cleanup tasks.
//!
//! [`ReleaseQueue`] distributes cleanup work (e.g., returning connections to a
//! pool, destroying tainted leases) across N primary workers and one fallback
//! worker. Tasks are round-robin distributed to primary workers; if a primary
//! channel is full, the task falls back to the overflow channel.
//!
//! # Shutdown
//!
//! Closing, dropping, or cancelling the queue seals new root submissions.
//! Already accepted jobs may publish same-queue descendants until all owned
//! activity settles; only then are workers terminated and joined.
//!
//! [`Manager`](crate::Manager) keeps cleanup open while its guards drain,
//! then closes the queue and awaits workers within a cooperative budget.

use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use futures::{StreamExt, stream::FuturesUnordered};
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore, mpsc, oneshot};
use tokio_util::sync::CancellationToken;

/// A boxed, pinned, sendable future that returns `()`.
type ReleaseTask = Pin<Box<dyn Future<Output = ()> + Send>>;

/// A fallible framework cleanup job; public unit-returning tasks are adapted here.
type JobFuture = Pin<Box<dyn Future<Output = Result<(), crate::Error>> + Send>>;
/// Future construction and polling both happen inside the worker fault boundary.
type TaskFactory = Box<dyn FnOnce() -> JobFuture + Send>;
pub(crate) type ReleaseReceipt = oneshot::Receiver<Result<(), crate::Error>>;

tokio::task_local! {
    static CURRENT_QUEUE: Arc<()>;
}

#[derive(Clone, Copy)]
enum JobClass {
    Entry,
    Coordinator,
}

enum Submission {
    Await,
    Deferred,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum QueuePhase {
    Open,
    Sealed,
    Terminating,
}

struct AdmissionState {
    phase: QueuePhase,
    outstanding: usize,
}

struct Admission {
    state: Mutex<AdmissionState>,
    changed: Notify,
    terminate: CancellationToken,
}

impl Admission {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(AdmissionState {
                phase: QueuePhase::Open,
                outstanding: 0,
            }),
            changed: Notify::new(),
            terminate: CancellationToken::new(),
        })
    }

    fn acquire(self: &Arc<Self>, descendant: bool) -> Option<JobPermit> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match state.phase {
            QueuePhase::Open => {},
            QueuePhase::Sealed if descendant => {},
            QueuePhase::Sealed | QueuePhase::Terminating => return None,
        }
        state.outstanding += 1;
        Some(JobPermit(Arc::clone(self)))
    }

    fn seal(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.phase == QueuePhase::Open {
            state.phase = QueuePhase::Sealed;
            tracing::debug!(
                outstanding = state.outstanding,
                "release queue sealed against new roots"
            );
        }
        drop(state);
        self.changed.notify_waiters();
    }

    fn terminate(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.phase = QueuePhase::Terminating;
        tracing::debug!(outstanding = state.outstanding, "release queue terminating");
        drop(state);
        self.terminate.cancel();
        self.changed.notify_waiters();
    }

    async fn wait_for(&self, predicate: impl Fn(&AdmissionState) -> bool) {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if predicate(
                &self
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            ) {
                return;
            }
            changed.await;
        }
    }

    async fn supervise(self: Arc<Self>, external_cancel: CancellationToken) {
        tokio::select! {
            () = external_cancel.cancelled() => self.seal(),
            () = self.wait_for(|state| state.phase != QueuePhase::Open) => {},
        }
        self.wait_for(|state| state.outstanding == 0 || state.phase == QueuePhase::Terminating)
            .await;
        self.terminate();
    }
}

struct JobPermit(Arc<Admission>);

impl Drop for JobPermit {
    fn drop(&mut self) {
        let mut state = self
            .0
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        debug_assert!(
            state.outstanding != 0,
            "cleanup activity ownership must balance"
        );
        state.outstanding -= 1;
        drop(state);
        self.0.changed.notify_waiters();
    }
}

struct RescueTask {
    queued: QueuedTask,
    deadline: tokio::time::Instant,
}

/// Tracks a submission until its future completes, including unpolled drops.
struct QueuedTask {
    factory: TaskFactory,
    class: JobClass,
    // The factory's captures must drop before a cancelled receipt can wake.
    completion: TaskCompletion,
}

struct TaskCompletion {
    loss: TaskLoss,
    _permit: Option<JobPermit>,
    capacity: Option<OwnedSemaphorePermit>,
    // Fields drop in declaration order. Cancellation closes the receipt only
    // after accounting and both activity/admission permits have settled.
    receipt: Option<oneshot::Sender<Result<(), crate::Error>>>,
}

impl QueuedTask {
    fn abandon(self, reason: &'static str) {
        let Self {
            factory,
            mut completion,
            ..
        } = self;
        completion.loss.reason = Some(reason);
        drop(factory);
        completion.finish(Err(crate::Error::cancelled()));
    }
}

impl TaskCompletion {
    fn finish(self, result: Result<(), crate::Error>) {
        let Self {
            loss,
            _permit,
            capacity,
            receipt,
        } = self;
        drop(loss);
        drop(_permit);
        drop(capacity);
        if let Some(receipt) = receipt {
            let _ = receipt.send(result);
        }
    }
}

pub(crate) struct TaskLoss {
    counter: Arc<AtomicUsize>,
    reason: Option<&'static str>,
    remaining: usize,
}

#[derive(Clone)]
pub(crate) struct AbandonmentTracker {
    counter: Arc<AtomicUsize>,
}

impl AbandonmentTracker {
    #[cfg(test)]
    pub(crate) fn for_test() -> Self {
        Self {
            counter: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub(crate) fn track_entries(&self, count: usize) -> TaskLoss {
        TaskLoss {
            counter: Arc::clone(&self.counter),
            reason: Some("abandoned"),
            remaining: count,
        }
    }
}

impl TaskLoss {
    pub(crate) fn absorb(&mut self, mut other: Self) {
        debug_assert!(
            Arc::ptr_eq(&self.counter, &other.counter),
            "batch loss accounting must share one queue"
        );
        self.remaining += other.remaining;
        other.remaining = 0;
    }

    pub(crate) fn add(&mut self, count: usize) {
        self.remaining += count;
    }

    pub(crate) fn transfer_one(&mut self) -> Self {
        self.remaining -= 1;
        Self {
            counter: Arc::clone(&self.counter),
            reason: self.reason,
            remaining: 1,
        }
    }
}

impl Drop for TaskLoss {
    fn drop(&mut self) {
        if let Some(reason) = self.reason
            && self.remaining != 0
        {
            record_drop(&self.counter, reason, self.remaining);
        }
    }
}

/// Maximum time a single release task may execute before being aborted.
///
/// The provider's composed deadline may impose a shorter budget. This ceiling
/// bounds each individual entry, including its framework hooks; coordinators
/// have no whole-collection deadline. Manager shutdown bounds their aggregate wait.
const TASK_EXECUTION_TIMEOUT: Duration = crate::hook_guard::MAX_TEARDOWN_CEILING;

/// Channel buffer size per primary worker.
const CHANNEL_BUFFER: usize = 256;

/// Channel buffer size for the fallback worker.
///
/// Previously unbounded — now bounded to prevent OOM under sustained overload.
/// Tasks exceeding this capacity are dropped with a warning.
const FALLBACK_BUFFER: usize = 4096;

/// Maximum admission wait of rescue work on double-`Full` saturation.
///
/// When both primary and fallback channels are full, [`ReleaseQueue::submit`]
/// uses its bounded, owned dispatcher to await capacity on the fallback channel
/// (blocking send) for up to this window. If no worker drains within
/// `RESCUE_TIMEOUT`, the task is recorded as truly dropped — an explicit,
/// metric-observable loss rather than a silent one. This bound also caps
/// the total lifetime of any rescue task so they cannot leak indefinitely.
const RESCUE_TIMEOUT: Duration = Duration::from_secs(30);
const REENTRANT_CAPACITY: usize = 64;
const RESCUE_CAPACITY: usize = FALLBACK_BUFFER;

/// Handle to the running release queue workers.
///
/// Must be passed to [`ReleaseQueue::shutdown`] for graceful termination.
#[must_use = "dropping ReleaseQueueHandle without shutdown leaks worker tasks"]
pub struct ReleaseQueueHandle {
    workers: Vec<tokio::task::JoinHandle<()>>,
    admission: Arc<Admission>,
}

/// Manager shutdown owns workers until completion or acknowledged abort.
struct AbortWorkers(ReleaseQueueHandle);

impl Drop for AbortWorkers {
    fn drop(&mut self) {
        self.0.admission.terminate();
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
/// 1. **Via cancellation token or [`close`](Self::close)**: seal new roots.
/// 2. **Via drop** (for standalone use): seal the queue and close its senders.
///
/// Existing same-queue descendants remain admissible until every accepted job
/// settles. In both cases, call [`ReleaseQueue::shutdown`] to observe completion.
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
    admission: Arc<Admission>,
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
    identity: Arc<()>,
    reentrant_tx: mpsc::Sender<QueuedTask>,
    reentrant_capacity: Arc<Semaphore>,
    rescue_tx: mpsc::Sender<RescueTask>,
}

impl Drop for ReleaseQueue {
    fn drop(&mut self) {
        self.admission.seal();
    }
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
    /// Cancelling the token seals roots. Workers exit after accepted jobs and
    /// their same-queue descendants settle. Keep this token
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
        let mut workers = Vec::with_capacity(worker_count + 3);

        let dropped_count = Arc::new(AtomicUsize::new(0));
        let identity = Arc::new(());
        let admission = Admission::new();

        for _ in 0..worker_count {
            let (tx, rx) = mpsc::channel::<QueuedTask>(CHANNEL_BUFFER);
            senders.push(tx);
            workers.push(tokio::spawn(Self::worker_loop(
                rx,
                admission.terminate.clone(),
                Arc::clone(&identity),
            )));
        }

        let (fallback_tx, fallback_rx) = mpsc::channel::<QueuedTask>(FALLBACK_BUFFER);
        workers.push(tokio::spawn(Self::worker_loop(
            fallback_rx,
            admission.terminate.clone(),
            Arc::clone(&identity),
        )));
        let (reentrant_tx, reentrant_rx) = mpsc::channel(REENTRANT_CAPACITY);
        workers.push(tokio::spawn(Self::reentrant_loop(
            reentrant_rx,
            admission.terminate.clone(),
            Arc::clone(&identity),
        )));
        let (rescue_tx, rescue_rx) = mpsc::channel(RESCUE_CAPACITY);
        workers.push(tokio::spawn(Self::rescue_loop(
            rescue_rx,
            fallback_tx.clone(),
            admission.terminate.clone(),
        )));
        workers.push(tokio::spawn(
            Arc::clone(&admission).supervise(cancel.clone()),
        ));

        let queue = Self {
            senders,
            fallback_tx,
            next: AtomicUsize::new(0),
            cancel,
            admission: Arc::clone(&admission),
            fallback_count: AtomicUsize::new(0),
            dropped_count,
            rescued_count: Arc::new(AtomicUsize::new(0)),
            identity,
            reentrant_tx,
            reentrant_capacity: Arc::new(Semaphore::new(REENTRANT_CAPACITY)),
            rescue_tx,
        };
        let handle = ReleaseQueueHandle { workers, admission };

        (queue, handle)
    }

    /// Submits a release task to the queue.
    ///
    /// The factory is called by a worker to produce the actual future.
    /// If the round-robin primary worker's channel is full, the task
    /// goes to the fallback channel.
    pub fn submit(&self, factory: impl FnOnce() -> ReleaseTask + Send + 'static) {
        let _ = self.enqueue(
            Box::new(move || {
                let task = factory();
                Box::pin(async move {
                    task.await;
                    Ok(())
                })
            }),
            None,
            JobClass::Entry,
        );
    }

    pub(crate) fn submit_release(
        &self,
        factory: impl FnOnce() -> JobFuture + Send + 'static,
    ) -> ReleaseReceipt {
        let (receipt, receiver) = oneshot::channel();
        let _ = self.enqueue(Box::new(factory), Some(receipt), JobClass::Entry);
        receiver
    }

    pub(crate) fn submit_guard_release(
        &self,
        factory: impl FnOnce() -> JobFuture + Send + 'static,
    ) -> Result<ReleaseReceipt, crate::Error> {
        let (receipt, receiver) = oneshot::channel();
        match self.enqueue(Box::new(factory), Some(receipt), JobClass::Entry)? {
            Submission::Await => Ok(receiver),
            Submission::Deferred => Err(crate::Error::deferred_cleanup()),
        }
    }

    pub(crate) fn submit_coordinator(
        &self,
        factory: impl FnOnce() -> JobFuture + Send + 'static,
    ) -> ReleaseReceipt {
        let (receipt, receiver) = oneshot::channel();
        let _ = self.enqueue(Box::new(factory), Some(receipt), JobClass::Coordinator);
        receiver
    }

    pub(crate) fn submit_joined_coordinator(
        &self,
        factory: impl FnOnce() -> JobFuture + Send + 'static,
    ) -> Result<ReleaseReceipt, crate::Error> {
        let (receipt, receiver) = oneshot::channel();
        match self.enqueue(Box::new(factory), Some(receipt), JobClass::Coordinator)? {
            Submission::Await => Ok(receiver),
            Submission::Deferred => Err(crate::Error::deferred_cleanup()),
        }
    }

    pub(crate) fn entry_losses(&self, count: usize) -> TaskLoss {
        self.abandonment_tracker().track_entries(count)
    }

    pub(crate) fn abandonment_tracker(&self) -> AbandonmentTracker {
        AbandonmentTracker {
            counter: Arc::clone(&self.dropped_count),
        }
    }

    fn enqueue(
        &self,
        factory: TaskFactory,
        receipt: Option<oneshot::Sender<Result<(), crate::Error>>>,
        class: JobClass,
    ) -> Result<Submission, crate::Error> {
        if self.cancel.is_cancelled() {
            self.admission.seal();
        }
        let nested = CURRENT_QUEUE
            .try_with(|current| Arc::ptr_eq(current, &self.identity))
            .ok();
        let permit = self.admission.acquire(nested == Some(true));
        let accepted = permit.is_some();
        let idx = self.next.fetch_add(1, Ordering::Relaxed) % self.senders.len();
        let mut factory = QueuedTask {
            factory,
            completion: TaskCompletion {
                receipt,
                loss: TaskLoss {
                    counter: Arc::clone(&self.dropped_count),
                    reason: Some("abandoned"),
                    remaining: usize::from(matches!(class, JobClass::Entry)),
                },
                _permit: permit,
                capacity: None,
            },
            class,
        };
        if !accepted {
            factory.abandon("queue_closed");
            return Err(crate::Error::cancelled());
        }

        let factory = if nested == Some(true) {
            if let Ok(capacity) = Arc::clone(&self.reentrant_capacity).try_acquire_owned() {
                factory.completion.capacity = Some(capacity);
                match self.reentrant_tx.try_send(factory) {
                    Ok(()) => return Ok(Submission::Await),
                    Err(error) => {
                        let mut factory = error.into_inner();
                        drop(factory.completion.capacity.take());
                        factory
                    },
                }
            } else {
                factory
            }
        } else {
            factory
        };
        let submission = if nested.is_some() {
            Submission::Deferred
        } else {
            Submission::Await
        };

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
                        // The owned rescue dispatcher provides a bounded extra
                        // admission window; exhaustion is an observable loss.
                        self.rescue(factory)?;
                    },
                    Err(mpsc::error::TrySendError::Closed(factory)) => {
                        // Fallback channel is closed — workers have exited.
                        // Record as a drop (with reason) instead of silently
                        // discarding. The factory is dropped here on purpose:
                        // there is nowhere to send it.
                        factory.abandon("fallback_channel_closed");
                        return Err(crate::Error::cancelled());
                    },
                }
            },
            Err(mpsc::error::TrySendError::Closed(factory)) => {
                // Primary worker exited (e.g., panic). Try the fallback
                // before recording a drop — fallback may still be alive.
                match self.fallback_tx.try_send(factory) {
                    Ok(()) => {},
                    Err(mpsc::error::TrySendError::Full(factory)) => {
                        self.rescue(factory)?;
                    },
                    Err(mpsc::error::TrySendError::Closed(factory)) => {
                        factory.abandon("primary_and_fallback_closed");
                        return Err(crate::Error::cancelled());
                    },
                }
            },
        }
        if matches!(submission, Submission::Deferred) {
            tracing::debug!(
                same_queue = nested == Some(true),
                "cleanup accepted with deferred completion to avoid a dependency wait"
            );
        }
        Ok(submission)
    }

    /// Publishes to the owned rescue dispatcher after a release lost the
    /// `try_send` race on both primary and fallback channels.
    ///
    /// The rescue task asynchronously reserves fallback capacity for up to
    /// [`RESCUE_TIMEOUT`]. Cancellation, timeout, or receiver closure drops
    /// its owned submission and records the loss. Both the bounded buffer and
    /// the in-flight reservation belong to a handle-owned worker, so bounded
    /// shutdown also aborts and joins rescue ownership.
    fn rescue(&self, factory: QueuedTask) -> Result<(), crate::Error> {
        let rescued = self.rescued_count.fetch_add(1, Ordering::Relaxed) + 1;
        if rescued.is_power_of_two() {
            tracing::warn!(
                rescued_tasks = rescued,
                "release queue saturated (primary + fallback full); \
                 submitting to bounded owned rescue dispatcher"
            );
        }

        self.rescue_tx
            .try_send(RescueTask {
                queued: factory,
                deadline: tokio::time::Instant::now() + RESCUE_TIMEOUT,
            })
            .map_err(|error| {
                error
                    .into_inner()
                    .queued
                    .abandon("rescue_capacity_or_closed");
                crate::Error::cancelled()
            })
    }

    async fn rescue_loop(
        mut receiver: mpsc::Receiver<RescueTask>,
        fallback: mpsc::Sender<QueuedTask>,
        cancel: CancellationToken,
    ) {
        loop {
            let task = tokio::select! {
                biased;
                () = cancel.cancelled() => break,
                task = receiver.recv() => match task { Some(task) => task, None => break },
            };
            let RescueTask {
                queued: task,
                deadline,
            } = task;
            tokio::select! {
                biased;
                () = cancel.cancelled() => {
                    task.abandon("rescue_shutdown");
                    break;
                }
                reservation = tokio::time::timeout_at(deadline, fallback.reserve()) => {
                    match reservation {
                        Ok(Ok(permit)) => { permit.send(task); }
                        Ok(Err(_)) => task.abandon("rescue_channel_closed"),
                        Err(_) => task.abandon("rescue_timeout"),
                    }
                }
            }
        }
        receiver.close();
        while let Some(task) = receiver.recv().await {
            task.queued.abandon("rescue_shutdown");
        }
    }

    async fn reentrant_loop(
        mut receiver: mpsc::Receiver<QueuedTask>,
        cancel: CancellationToken,
        identity: Arc<()>,
    ) {
        let mut running = FuturesUnordered::new();
        loop {
            tokio::select! {
                biased;
                () = cancel.cancelled() => {
                    receiver.close();
                    while let Some(task) = receiver.recv().await {
                        running.push(Self::execute_reentrant(task, Arc::clone(&identity)));
                    }
                    break;
                }
                task = receiver.recv() => {
                    if let Some(task) = task {
                        running.push(Self::execute_reentrant(task, Arc::clone(&identity)));
                    } else { break; }
                }
                _ = running.next(), if !running.is_empty() => {}
            }
        }
        while running.next().await.is_some() {}
    }

    async fn execute_reentrant(queued: QueuedTask, identity: Arc<()>) {
        CURRENT_QUEUE
            .scope(identity, Self::execute_task(queued))
            .await;
    }

    /// Returns the total number of tasks routed via the fallback channel.
    pub fn fallback_count(&self) -> usize {
        self.fallback_count.load(Ordering::Relaxed)
    }

    /// Returns the cumulative number of submitted entries/tasks abandoned before
    /// completion, including weighted unstarted batch members, rejected submissions, rescue failure, discarded
    /// buffers, worker aborts, factory panics, future panics, and timeouts.
    /// A non-zero value warrants operator attention. Zero does not prove
    /// provider success: a completed task can return or internally handle a
    /// provider error. Open queues and late Force-policy releases may add losses later.
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

    /// Seals new root submissions while preserving accepted descendants.
    ///
    /// External and cross-queue submissions are rejected and counted as losses.
    /// Same-queue cleanup already running may publish descendants until all
    /// accepted activity settles; then every owned worker exits.
    ///
    /// This initiates the same admission transition as cancellation of the token
    /// passed to [`with_cancel`](Self::with_cancel), without cancelling that token.
    pub fn close(&self) {
        self.admission.seal();
    }

    /// Shuts down all workers gracefully, waiting for in-flight tasks.
    ///
    /// Seals admission, waits for accepted roots and same-queue descendants,
    /// then joins all owned workers, including rescue and reentrant dispatchers.
    ///
    /// # Cancel safety
    ///
    /// This method is cancel safe. If dropped while awaiting a worker's
    /// `JoinHandle`, that worker (and any not yet reached) is not aborted —
    /// the owned supervisor keeps observing quiescence and terminates workers
    /// afterward. The caller loses only the "all workers finished" observation.
    pub async fn shutdown(handle: ReleaseQueueHandle) {
        handle.admission.seal();
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
        handle.admission.seal();
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
        owned.0.admission.terminate();
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
    async fn worker_loop(
        mut rx: mpsc::Receiver<QueuedTask>,
        cancel: CancellationToken,
        identity: Arc<()>,
    ) {
        loop {
            tokio::select! {
                biased;
                msg = rx.recv() => {
                    match msg {
                        Some(factory) => CURRENT_QUEUE.scope(Arc::clone(&identity), Self::execute_task(factory)).await,
                        None => break, // channel closed
                    }
                }
                () = cancel.cancelled() => {
                    // Drain remaining buffered tasks, then exit.
                    rx.close();
                    while let Some(factory) = rx.recv().await {
                        CURRENT_QUEUE.scope(Arc::clone(&identity), Self::execute_task(factory)).await;
                    }
                    break;
                }
            }
        }
    }

    async fn execute_task(queued: QueuedTask) {
        let QueuedTask {
            factory,
            class,
            mut completion,
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
            completion.loss.reason = Some("factory_panic");
            completion.finish(Err(crate::Error::permanent(
                "release task factory panicked",
            )));
            return;
        };
        let guarded = match class {
            JobClass::Entry => {
                crate::hook_guard::guard_author_hook(TASK_EXECUTION_TIMEOUT, task).await
            },
            JobClass::Coordinator => {
                use futures::FutureExt;
                std::panic::AssertUnwindSafe(task)
                    .catch_unwind()
                    .await
                    .map_err(|_| crate::hook_guard::HookFault::Panicked)
            },
        };
        let result = Self::finish_task(guarded, &mut completion.loss);
        completion.finish(result);
    }

    pub(crate) async fn run_entry(
        future: impl Future<Output = Result<(), crate::Error>> + Send,
        mut loss: TaskLoss,
    ) -> Result<(), crate::Error> {
        let guarded = crate::hook_guard::guard_author_hook(TASK_EXECUTION_TIMEOUT, future).await;
        Self::finish_task(guarded, &mut loss)
    }

    fn finish_task(
        guarded: Result<Result<(), crate::Error>, crate::hook_guard::HookFault>,
        loss: &mut TaskLoss,
    ) -> Result<(), crate::Error> {
        match guarded {
            Ok(result) => {
                loss.reason = None;
                if let Err(error) = &result
                    && *error.kind() != crate::ErrorKind::DeferredCleanup
                {
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
        }
    }
}

/// Records a terminal task drop on the shared counter, with a structured
/// reason. Logs at ERROR level when the drop count crosses a power-of-two
/// boundary so log volume stays bounded under sustained loss.
fn record_drop(counter: &Arc<AtomicUsize>, reason: &'static str, count: usize) {
    let n = counter.fetch_add(count, Ordering::Relaxed) + count;
    if n.is_power_of_two() || count > 1 {
        tracing::error!(
            dropped_tasks = n,
            reason = reason,
            "release task dropped — resource may leak"
        );
    }
}

#[cfg(test)]
mod tests {
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
        let parent = queue.submit_release(move || {
            Box::pin(async move {
                entered.send(()).unwrap();
                resumed.await.unwrap();
                nested_queue
                    .submit_guard_release(move || {
                        Box::pin(async move {
                            child_completed.fetch_add(1, Ordering::SeqCst);
                            Ok(())
                        })
                    })?
                    .await
                    .unwrap()
            })
        });
        parent_entered.await.unwrap();
        queue.close();
        resume.send(()).unwrap();
        parent.await.unwrap().expect(
            "a sealed queue must retain descendant cleanup admission until owned parents settle",
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
        let parent = queue.submit_release(move || {
            Box::pin(async move {
                nested
                    .submit_guard_release(move || {
                        Box::pin(async move {
                            entered.send(()).unwrap();
                            std::future::pending::<()>().await;
                            drop(child_owner);
                            Ok(())
                        })
                    })?
                    .await
                    .unwrap()
            })
        });
        child_entered.await.unwrap();
        queue.close();
        ReleaseQueue::shutdown_bounded(handle, Duration::from_secs(1))
            .await
            .unwrap_err();
        assert!(parent.await.is_err());
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
        let parent = queue.submit_release(move || {
            Box::pin(async move {
                let error = nested_queue
                    .submit_guard_release(move || {
                        Box::pin(async move {
                            finished.send(()).unwrap();
                            Ok(())
                        })
                    })
                    .unwrap_err();
                assert_eq!(*error.kind(), crate::ErrorKind::DeferredCleanup);
                Ok(())
            })
        });
        parent.await.unwrap().unwrap();
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
        let error = CURRENT_QUEUE
            .scope(Arc::new(()), async {
                queue
                    .submit_guard_release(|| Box::pin(async { Ok(()) }))
                    .unwrap_err()
            })
            .await;
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
        let started = Arc::new(Notify::new());
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

    fn submit_gated(queue: &ReleaseQueue, gate: &Arc<Notify>, counter: &Arc<AtomicU32>) {
        let g = gate.clone();
        let c = counter.clone();
        queue.submit(move || Box::pin(gated_increment(g, c)));
    }

    async fn gated_increment(gate: Arc<Notify>, counter: Arc<AtomicU32>) {
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
        let gate = Arc::new(Notify::new());

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
