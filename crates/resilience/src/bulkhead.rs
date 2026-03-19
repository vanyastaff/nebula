//! Bulkhead pattern — semaphore-based concurrency limit with injectable sink.

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::Semaphore;

use crate::{
    CallError, ConfigError,
    sink::{MetricsSink, NoopSink, ResilienceEvent},
};

// ── Config ────────────────────────────────────────────────────────────────────

/// Configuration for the bulkhead pattern.
#[derive(Debug, Clone)]
pub struct BulkheadConfig {
    /// Maximum number of concurrent operations. Min: 1.
    pub max_concurrency: usize,
    /// Maximum number of operations allowed to queue waiting for a permit.
    pub queue_size: usize,
    /// Optional timeout while waiting for a permit.
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
    /// Returns `Err(ConfigError)` if `max_concurrency` or `queue_size` is 0.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.max_concurrency == 0 {
            return Err(ConfigError::new("max_concurrency", "must be >= 1"));
        }
        if self.queue_size == 0 {
            return Err(ConfigError::new("queue_size", "must be >= 1"));
        }
        Ok(())
    }
}

// ── Bulkhead ──────────────────────────────────────────────────────────────────

/// Bulkhead — limits concurrent operations via a semaphore.
///
/// Shared state via `Arc<Bulkhead>`. Add a [`RecordingSink`] for test observability.
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
    pub async fn call<T, E>(
        &self,
        f: impl FnOnce() -> std::pin::Pin<Box<dyn Future<Output = Result<T, E>> + Send>>,
    ) -> Result<T, CallError<E>> {
        let _permit = self.acquire_permit().await?;
        f().await.map_err(CallError::Operation)
    }

    /// Acquire a permit directly. Use [`call`](Bulkhead::call) for the typical execute-and-release pattern.
    ///
    /// # Errors
    ///
    /// Returns `Err(CallError::BulkheadFull)` when the queue is full,
    /// or `Err(CallError::Timeout)` if a permit timeout is configured and exceeded.
    pub async fn acquire<E>(&self) -> Result<BulkheadPermit, CallError<E>> {
        self.acquire_permit().await
    }

    // ── internal ──────────────────────────────────────────────────────────────

    async fn acquire_permit<E>(&self) -> Result<BulkheadPermit, CallError<E>> {
        // Fast path — permit immediately available
        if let Ok(permit) = Arc::clone(&self.semaphore).try_acquire_owned() {
            return Ok(BulkheadPermit { _permit: permit });
        }

        // Try to enqueue
        let enqueued =
            self.waiting_count
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |cur| {
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

        self.waiting_count.fetch_sub(1, Ordering::AcqRel);
        result
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
#[derive(Debug, Clone)]
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
        BulkheadStats {
            max_concurrency: self.config.max_concurrency,
            active_operations: self.active_operations(),
            available_permits: self.available_permits(),
            is_at_capacity: self.is_at_capacity(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CallError, RecordingSink};
    use std::time::Duration;

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
        let result = bh.call::<_, &str>(|| Box::pin(async { Ok("ok") })).await;
        assert_eq!(result.unwrap(), "ok");
    }

    #[tokio::test]
    async fn rejects_when_queue_full() {
        // queue_size=0 is invalid, use 1 with saturation
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

        assert!(sink.count("bulkhead_rejected") > 0);
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
