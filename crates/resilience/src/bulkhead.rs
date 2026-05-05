//! Bulkhead pattern — semaphore-based concurrency limit with injectable sink.

use std::{
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use tokio::sync::Semaphore;

use crate::{
    CallError, ConfigError, PolicyContext,
    sink::{MetricsSink, NoopSink, ResilienceEvent},
};

// ── Config ────────────────────────────────────────────────────────────────────

/// Configuration for the bulkhead pattern.
///
/// # Examples
///
/// ```rust,no_run
/// use std::time::Duration;
///
/// use nebula_resilience::{Bulkhead, BulkheadConfig};
///
/// // Fail-fast bulkhead: no queue, reject on saturation.
/// let cfg = BulkheadConfig {
///     max_concurrency: 8,
///     queue_size: 0,
///     timeout: Some(Duration::from_secs(5)),
/// };
///
/// let _bulkhead = Bulkhead::new(cfg).expect("config is valid");
/// ```
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BulkheadConfig {
    /// Maximum number of concurrent operations. Min: 1.
    pub max_concurrency: usize,
    /// Maximum number of operations allowed to queue while waiting for a permit.
    ///
    /// `0` means **no queue**: if no permit is free, [`Bulkhead::acquire`] returns
    /// [`CallError::BulkheadFull`] immediately (fail-fast) instead of waiting in line.
    pub queue_size: usize,
    /// Optional timeout while waiting for a permit.
    #[cfg_attr(feature = "serde", serde(default))]
    pub timeout: Option<std::time::Duration>,
}

impl Default for BulkheadConfig {
    fn default() -> Self {
        Self {
            max_concurrency: 10,
            queue_size: 100,
            timeout: Some(std::time::Duration::from_secs(30)),
        }
    }
}

impl BulkheadConfig {
    /// Validate configuration. Called by `Bulkhead::new()`.
    ///
    /// # Errors
    ///
    /// Returns `Err(ConfigError)` if `max_concurrency` is 0. `queue_size` may be `0` for a
    /// no-queue, fail-fast bulkhead (see [`BulkheadConfig::queue_size`](Self::queue_size)).
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.max_concurrency == 0 {
            return Err(ConfigError::new("max_concurrency", "must be >= 1"));
        }
        Ok(())
    }
}

// ── Bulkhead ──────────────────────────────────────────────────────────────────

/// Bulkhead — limits concurrent operations via a semaphore.
///
/// Shared state via `Arc<Bulkhead>`. Add a [`RecordingSink`](crate::RecordingSink) for test
/// observability.
///
/// # Examples
///
/// ```rust,no_run
/// use nebula_resilience::{Bulkhead, BulkheadConfig, CallError};
///
/// # #[tokio::main]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let bulkhead = Bulkhead::new(BulkheadConfig {
///     max_concurrency: 4,
///     queue_size: 8,
///     timeout: None,
/// })?;
///
/// let value: Result<&str, CallError<&str>> = bulkhead.call(|| async { Ok("ok") }).await;
/// assert_eq!(value.unwrap(), "ok");
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct Bulkhead {
    config: BulkheadConfig,
    semaphore: Arc<Semaphore>,
    waiting_count: Arc<AtomicUsize>,
    sink: Arc<dyn MetricsSink>,
}

impl std::fmt::Debug for Bulkhead {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bulkhead")
            .field("max_concurrency", &self.config.max_concurrency)
            .field("active", &self.active_operations())
            .finish_non_exhaustive()
    }
}

impl Bulkhead {
    /// Create a new bulkhead.
    ///
    /// # Errors
    ///
    /// Returns `Err(ConfigError)` if config is invalid.
    pub fn new(config: BulkheadConfig) -> Result<Self, ConfigError> {
        config.validate()?;
        Ok(Self {
            semaphore: Arc::new(Semaphore::new(config.max_concurrency)),
            waiting_count: Arc::new(AtomicUsize::new(0)),
            config,
            sink: Arc::new(NoopSink),
        })
    }

    /// Replace the metrics sink (builder-style).
    #[must_use]
    pub fn with_sink(mut self, sink: impl MetricsSink + 'static) -> Self {
        self.sink = Arc::new(sink);
        self
    }

    /// Current number of active (in-flight) operations.
    #[must_use]
    pub fn active_operations(&self) -> usize {
        self.config.max_concurrency - self.semaphore.available_permits()
    }

    /// Current number of available permits.
    #[must_use]
    pub fn available_permits(&self) -> usize {
        self.semaphore.available_permits()
    }

    /// Whether the bulkhead is at capacity (no permits available).
    #[must_use]
    pub fn is_at_capacity(&self) -> bool {
        self.semaphore.available_permits() == 0
    }

    /// Maximum concurrency limit.
    #[must_use]
    pub const fn max_concurrency(&self) -> usize {
        self.config.max_concurrency
    }

    /// Execute a closure under the bulkhead.
    ///
    /// # Errors
    ///
    /// Returns `Err(CallError::BulkheadFull)` when the queue is full,
    /// or `Err(CallError::Operation)` if the operation itself fails.
    pub async fn call<T, E, Fut>(&self, f: impl FnOnce() -> Fut) -> Result<T, CallError<E>>
    where
        Fut: Future<Output = Result<T, E>> + Send,
    {
        let _permit = self.acquire_permit().await?;
        f().await.map_err(CallError::Operation)
    }

    /// Execute a closure under the bulkhead with a shared policy context.
    ///
    /// The context cancellation/deadline bounds both waiting for a permit and
    /// the operation itself. If the returned future is cancelled or times out,
    /// the permit/queue slot is released by RAII.
    ///
    /// # Errors
    ///
    /// Returns `Err(CallError::Cancelled)` if the context is cancelled,
    /// `Err(CallError::Timeout)` if the context deadline or bulkhead queue timeout
    /// expires, `Err(CallError::BulkheadFull)` when capacity/queue is exhausted,
    /// or `Err(CallError::Operation)` if the operation itself fails.
    pub async fn call_with_policy_context<T, E, Fut>(
        &self,
        context: &PolicyContext,
        f: impl FnOnce() -> Fut + Send,
    ) -> Result<T, CallError<E>>
    where
        Fut: Future<Output = Result<T, E>> + Send,
    {
        let _permit = self.acquire_with_policy_context(context).await?;
        context
            .run_result(async { f().await.map_err(CallError::Operation) })
            .await
    }

    /// Acquire a permit directly. Use [`call`](Bulkhead::call) for the typical execute-and-release
    /// pattern.
    ///
    /// # Errors
    ///
    /// Returns `Err(CallError::BulkheadFull)` when the queue is full,
    /// or `Err(CallError::Timeout)` if a permit timeout is configured and exceeded.
    ///
    /// **Note:** Queue timeout returns `CallError::Timeout`, not `BulkheadFull`.
    /// When used in a pipeline alongside a `Timeout` step, callers cannot
    /// distinguish the two by variant alone — check the duration value if needed.
    pub async fn acquire<E>(&self) -> Result<BulkheadPermit, CallError<E>> {
        self.acquire_permit().await
    }

    /// Acquire a permit with cancellation/deadline from a shared policy context.
    ///
    /// # Errors
    ///
    /// Returns `Err(CallError::Cancelled)` if the context is cancelled,
    /// `Err(CallError::Timeout)` if the context deadline or configured queue
    /// timeout expires, or `Err(CallError::BulkheadFull)` when the queue is full.
    pub async fn acquire_with_policy_context<E>(
        &self,
        context: &PolicyContext,
    ) -> Result<BulkheadPermit, CallError<E>> {
        context.run_result(self.acquire_permit()).await
    }

    // ── internal ──────────────────────────────────────────────────────────────

    async fn acquire_permit<E>(&self) -> Result<BulkheadPermit, CallError<E>> {
        // Fast path — permit immediately available
        if let Ok(permit) = Arc::clone(&self.semaphore).try_acquire_owned() {
            return Ok(BulkheadPermit { _permit: permit });
        }

        if self.config.queue_size == 0 {
            self.sink.record(ResilienceEvent::BulkheadRejected);
            return Err(CallError::BulkheadFull);
        }

        // Try to enqueue via `try_update` (Rust 1.95).
        // Signature is `try_update(set_order, fetch_order, f)` — maps from
        // `fetch_update(success=AcqRel, failure=Acquire, f)` as
        // `set_order := success = AcqRel`, `fetch_order := failure = Acquire`.
        let enqueued = self
            .waiting_count
            .try_update(Ordering::AcqRel, Ordering::Acquire, |cur| {
                if cur < self.config.queue_size {
                    Some(cur + 1)
                } else {
                    None
                }
            });

        if enqueued.is_err() {
            // Queue full — reject
            self.sink.record(ResilienceEvent::BulkheadRejected);
            return Err(CallError::BulkheadFull);
        }

        // RAII guard: if this future is dropped while waiting for a permit,
        // decrement waiting_count so the queue slot isn't permanently leaked.
        let mut wait_guard = WaitCountGuard {
            count: &self.waiting_count,
            defused: false,
        };

        // Wait for a permit (with optional timeout)
        let result = if let Some(timeout_dur) = self.config.timeout {
            match tokio::time::timeout(timeout_dur, Arc::clone(&self.semaphore).acquire_owned())
                .await
            {
                Ok(Ok(permit)) => Ok(BulkheadPermit { _permit: permit }),
                Ok(Err(_closed)) => Err(CallError::BulkheadFull),
                Err(_elapsed) => Err(CallError::Timeout(timeout_dur)),
            }
        } else {
            Arc::clone(&self.semaphore)
                .acquire_owned()
                .await
                .map(|permit| BulkheadPermit { _permit: permit })
                .map_err(|_| CallError::BulkheadFull)
        };

        // Defuse the guard and decrement manually.
        wait_guard.defuse();
        self.waiting_count.fetch_sub(1, Ordering::AcqRel);
        result
    }
}

/// RAII guard that decrements `waiting_count` on drop.
///
/// Prevents the queue counter from leaking when the `acquire_permit` future
/// is dropped mid-wait (e.g. by `tokio::select!` or a pipeline timeout).
struct WaitCountGuard<'a> {
    count: &'a AtomicUsize,
    defused: bool,
}

impl WaitCountGuard<'_> {
    const fn defuse(&mut self) {
        self.defused = true;
    }
}

impl Drop for WaitCountGuard<'_> {
    fn drop(&mut self) {
        if !self.defused {
            self.count.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

// ── Permit ────────────────────────────────────────────────────────────────────

/// RAII permit — dropping it returns the semaphore slot synchronously.
#[derive(Debug)]
pub struct BulkheadPermit {
    _permit: tokio::sync::OwnedSemaphorePermit,
}

// ── Stats ─────────────────────────────────────────────────────────────────────

/// Snapshot of bulkhead state.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BulkheadStats {
    /// Maximum concurrency limit.
    pub max_concurrency: usize,
    /// Current active operations.
    pub active_operations: usize,
    /// Currently available permits.
    pub available_permits: usize,
    /// Whether bulkhead is at capacity.
    pub is_at_capacity: bool,
}

impl Bulkhead {
    /// Returns a snapshot of current bulkhead state.
    #[must_use]
    pub fn stats(&self) -> BulkheadStats {
        let available_permits = self.semaphore.available_permits();

        BulkheadStats {
            max_concurrency: self.config.max_concurrency,
            active_operations: self.config.max_concurrency - available_permits,
            available_permits,
            is_at_capacity: available_permits == 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::{CallError, PolicyContext, RecordingSink, ResilienceEventKind};

    fn cfg(max: usize) -> BulkheadConfig {
        BulkheadConfig {
            max_concurrency: max,
            queue_size: 10,
            timeout: None,
        }
    }

    #[tokio::test]
    async fn permits_and_capacity() {
        let bh = Bulkhead::new(cfg(3)).unwrap();
        assert_eq!(bh.max_concurrency(), 3);
        assert_eq!(bh.active_operations(), 0);
        assert!(!bh.is_at_capacity());
    }

    #[tokio::test]
    async fn call_succeeds_within_capacity() {
        let bh = Bulkhead::new(cfg(2)).unwrap();
        let result = bh.call::<_, &str, _>(|| Box::pin(async { Ok("ok") })).await;
        assert_eq!(result.unwrap(), "ok");
    }

    #[tokio::test]
    async fn rejects_immediately_when_queue_size_zero() {
        let bh = Bulkhead::new(BulkheadConfig {
            max_concurrency: 1,
            queue_size: 0,
            timeout: None,
        })
        .unwrap();

        let permit = bh.acquire::<&str>().await.unwrap();
        let err = bh.acquire::<&str>().await.unwrap_err();
        assert!(matches!(err, CallError::BulkheadFull));
        drop(permit);
    }

    #[tokio::test]
    async fn rejects_when_queue_full() {
        // queue_size=1: at most one waiter; third acquire while saturated fails.
        let bh = Bulkhead::new(BulkheadConfig {
            max_concurrency: 1,
            queue_size: 1,
            timeout: None,
        })
        .unwrap();

        // Hold the only permit
        let permit = bh.acquire::<&str>().await.unwrap();
        // Queue one waiter
        let bh2 = bh.clone();
        let waiter = tokio::spawn(async move { bh2.acquire::<&str>().await });
        tokio::time::sleep(Duration::from_millis(10)).await;

        // Now queue is full (1 waiter) — next call is rejected
        let err = bh.acquire::<&str>().await.unwrap_err();
        assert!(matches!(err, CallError::BulkheadFull));

        drop(permit);
        waiter.await.unwrap().unwrap(); // the queued one succeeds
    }

    #[tokio::test]
    async fn dropping_queued_acquire_releases_queue_slot() {
        let bh = Bulkhead::new(BulkheadConfig {
            max_concurrency: 1,
            queue_size: 1,
            timeout: None,
        })
        .unwrap();

        let permit = bh.acquire::<&str>().await.unwrap();
        let mut queued = Box::pin(bh.acquire::<&str>());

        tokio::select! {
            result = &mut queued => {
                panic!("queued acquire completed unexpectedly: {result:?}");
            },
            () = tokio::time::sleep(Duration::from_millis(10)) => {},
        }
        assert_eq!(bh.waiting_count.load(Ordering::Acquire), 1);

        drop(queued);
        tokio::task::yield_now().await;

        assert_eq!(bh.waiting_count.load(Ordering::Acquire), 0);
        drop(permit);
    }

    #[tokio::test]
    async fn policy_context_cancelled_acquire_releases_queue_slot() {
        let bh = Bulkhead::new(BulkheadConfig {
            max_concurrency: 1,
            queue_size: 1,
            timeout: None,
        })
        .unwrap();
        let context = PolicyContext::from_cancellation(crate::CancellationContext::new());

        let permit = bh.acquire::<&str>().await.unwrap();
        let mut queued = Box::pin(bh.acquire_with_policy_context::<&str>(&context));

        tokio::select! {
            result = &mut queued => {
                panic!("queued acquire completed unexpectedly: {result:?}");
            },
            () = tokio::time::sleep(Duration::from_millis(10)) => {},
        }
        assert_eq!(bh.waiting_count.load(Ordering::Acquire), 1);

        context.cancellation().unwrap().cancel();
        let err = queued.await.unwrap_err();

        assert!(matches!(err, CallError::Cancelled { .. }));
        assert_eq!(bh.waiting_count.load(Ordering::Acquire), 0);
        drop(permit);
    }

    #[tokio::test]
    async fn policy_context_deadline_releases_operation_permit() {
        let bh = Bulkhead::new(BulkheadConfig {
            max_concurrency: 1,
            queue_size: 0,
            timeout: None,
        })
        .unwrap();
        let context = PolicyContext::with_timeout(Duration::from_millis(1));

        let err = bh
            .call_with_policy_context::<(), &str, _>(&context, || {
                Box::pin(async {
                    tokio::time::sleep(Duration::from_mins(1)).await;
                    Ok(())
                })
            })
            .await
            .unwrap_err();

        assert!(matches!(err, CallError::Timeout(_)));
        assert_eq!(bh.available_permits(), 1);
    }

    #[tokio::test]
    async fn emits_rejected_event() {
        let sink = RecordingSink::new();
        let bh = Bulkhead::new(BulkheadConfig {
            max_concurrency: 1,
            queue_size: 1,
            timeout: None,
        })
        .unwrap()
        .with_sink(sink.clone());

        let _permit = bh.acquire::<&str>().await.unwrap();
        // Fill queue
        let bh2 = bh.clone();
        tokio::spawn(async move { bh2.acquire::<&str>().await });
        tokio::time::sleep(Duration::from_millis(10)).await;

        // This one should be rejected and emit an event
        let _ = bh.acquire::<&str>().await;

        assert!(sink.count(ResilienceEventKind::BulkheadRejected) > 0);
    }

    #[tokio::test]
    async fn config_error_on_zero_concurrency() {
        let result = Bulkhead::new(BulkheadConfig {
            max_concurrency: 0,
            queue_size: 10,
            timeout: None,
        });
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn active_operations_tracking() {
        let bh = Bulkhead::new(cfg(3)).unwrap();
        let p1 = bh.acquire::<()>().await.unwrap();
        assert_eq!(bh.active_operations(), 1);
        let p2 = bh.acquire::<()>().await.unwrap();
        assert_eq!(bh.active_operations(), 2);
        drop(p1);
        assert_eq!(bh.active_operations(), 1);
        drop(p2);
        assert_eq!(bh.active_operations(), 0);
    }
}
