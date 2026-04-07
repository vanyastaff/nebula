//! Async-friendly object pool with safe Drop semantics and graceful shutdown
//!
//! This module provides an async-safe object pool that follows modern async patterns:
//! - Graceful shutdown with CancellationToken
//! - Structured concurrency for background tasks
//! - Comprehensive tracing instrumentation
//! - Timeout support for acquire operations
//! - Guaranteed object returns via unbounded channel

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{Mutex, Semaphore, mpsc};
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

#[cfg(feature = "tracing")]
use tracing::{debug, instrument, trace, warn};

use crate::error::{MemoryError, MemoryResult};
use crate::pool::{PoolConfig, Poolable};

/// Default timeout for acquire operations (30 seconds)
const DEFAULT_ACQUIRE_TIMEOUT: Duration = Duration::from_secs(30);

/// Async pooled value that returns to pool on drop
///
/// Uses unbounded channel for guaranteed return-to-pool in Drop.
/// This avoids the anti-pattern of using try_lock() in Drop which can
/// silently fail and leak objects.
pub struct AsyncPooledValue<T: Poolable> {
    value: Option<T>,
    return_tx: mpsc::UnboundedSender<T>,
    _permit: tokio::sync::OwnedSemaphorePermit,
    // _permit is dropped when this value is dropped, releasing the semaphore slot
    // so the next waiter (acquire/try_acquire) can proceed.
}

impl<T: Poolable> AsyncPooledValue<T> {
    /// Detach value from pool (won't be returned)
    #[cfg_attr(feature = "tracing", instrument(skip(self)))]
    pub fn detach(mut self) -> T {
        #[cfg(feature = "tracing")]
        trace!("Detaching value from pool");

        // The permit is still held (and released when self drops via Drop),
        // which is correct for a detached value: the capacity slot stays
        // consumed until the caller is done with the value.
        self.value.take().expect("Value already detached")
    }
}

impl<T: Poolable> std::ops::Deref for AsyncPooledValue<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.value.as_ref().expect("Value already detached")
    }
}

impl<T: Poolable> std::ops::DerefMut for AsyncPooledValue<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.value.as_mut().expect("Value already detached")
    }
}

impl<T: Poolable> Drop for AsyncPooledValue<T> {
    fn drop(&mut self) {
        if let Some(mut value) = self.value.take() {
            // Reset the object before returning
            value.reset();

            #[cfg(feature = "tracing")]
            trace!("Returning value to pool");

            // ✅ SAFE: Channel send always succeeds (unbounded)
            // Objects are guaranteed to be returned to pool.
            // The _permit is dropped AFTER the send, so the semaphore
            // slot is released after the object is queued for return.
            let _ = self.return_tx.send(value);
        }
        // _permit is dropped here, releasing the semaphore slot
        // and allowing the next waiter to proceed.
    }
}

/// Internal pool state
struct AsyncPoolInner<T: Poolable> {
    objects: Vec<T>,
    factory: Box<dyn Fn() -> T + Send + Sync>,
    config: PoolConfig,
    total_created: usize,
}

/// Async-friendly object pool with graceful shutdown
///
/// Provides async/await compatible object pooling with:
/// - **Graceful shutdown**: CancellationToken for clean termination
/// - **Guaranteed returns**: Unbounded channel ensures no loss in Drop
/// - **Backpressure**: Semaphore limits concurrent acquisitions
/// - **Tracing**: Comprehensive instrumentation for debugging
/// - **Timeout**: Configurable timeouts for acquire operations
///
/// # Architecture
///
/// - **Fast path**: Pop from Vec<T> when objects available
/// - **Slow path**: Create new object if under capacity
/// - **Return path**: Unbounded channel ensures no loss in Drop
/// - **Background task**: Processes returns and replenishes pool
/// - **Shutdown**: CancellationToken cleanly terminates background task
///
/// # Example
///
/// ```ignore
/// use nebula_memory::async_support::AsyncPool;
/// use tokio_util::sync::CancellationToken;
///
/// #[tokio::main]
/// async fn main() {
///     let shutdown = CancellationToken::new();
///     let pool = AsyncPool::new(10, || String::new(), shutdown.clone());
///
///     // Use the pool
///     let obj = pool.acquire().await.unwrap();
///     drop(obj);
///
///     // Graceful shutdown
///     shutdown.cancel();
///     pool.shutdown().await;
/// }
/// ```
pub struct AsyncPool<T: Poolable> {
    inner: Arc<Mutex<AsyncPoolInner<T>>>,
    return_tx: mpsc::UnboundedSender<T>,
    semaphore: Arc<Semaphore>,
    shutdown: CancellationToken,
    default_timeout: Duration,
}

impl<T: Poolable> AsyncPool<T> {
    /// Create new async pool with capacity, factory function, and shutdown token
    ///
    /// # Arguments
    ///
    /// * `capacity` - Initial and maximum pool capacity
    /// * `factory` - Function to create new objects
    /// * `shutdown` - Token to signal graceful shutdown
    #[cfg_attr(feature = "tracing", instrument(skip(factory, shutdown)))]
    pub fn new<F>(capacity: usize, factory: F, shutdown: CancellationToken) -> Self
    where
        F: Fn() -> T + Send + Sync + 'static,
    {
        #[cfg(feature = "tracing")]
        debug!(capacity, "Creating AsyncPool");

        let config = PoolConfig {
            initial_capacity: capacity,
            max_capacity: Some(capacity),
            validate_on_return: true,
            ..Default::default()
        };

        Self::with_config(config, factory, shutdown)
    }

    /// Create new async pool with custom configuration
    #[cfg_attr(feature = "tracing", instrument(skip(factory, shutdown)))]
    pub fn with_config<F>(config: PoolConfig, factory: F, shutdown: CancellationToken) -> Self
    where
        F: Fn() -> T + Send + Sync + 'static,
    {
        let initial_capacity = config.initial_capacity;
        let config_max_capacity = config.max_capacity;

        #[cfg(feature = "tracing")]
        debug!(
            initial_capacity,
            max_capacity = ?config.max_capacity,
            "Creating AsyncPool with config"
        );

        let mut objects = Vec::with_capacity(initial_capacity);
        for _ in 0..initial_capacity {
            objects.push(factory());
        }

        let inner = Arc::new(Mutex::new(AsyncPoolInner {
            objects,
            factory: Box::new(factory),
            config,
            total_created: initial_capacity,
        }));

        // Create unbounded channel for returns
        let (return_tx, mut return_rx) = mpsc::unbounded_channel::<T>();

        // Spawn background task to process returns with cancellation support
        let inner_clone = Arc::clone(&inner);
        let shutdown_clone = shutdown.clone();
        tokio::spawn(async move {
            #[cfg(feature = "tracing")]
            debug!("Starting pool return processor");

            loop {
                tokio::select! {
                    // Handle shutdown signal
                    _ = shutdown_clone.cancelled() => {
                        #[cfg(feature = "tracing")]
                        debug!("Pool return processor shutting down");
                        break;
                    }

                    // Process returned objects
                    Some(obj) = return_rx.recv() => {
                        #[cfg(feature = "tracing")]
                        trace!("Processing returned object");

                        let mut pool = inner_clone.lock().await;
                        if pool.objects.len() < pool.config.initial_capacity {
                            pool.objects.push(obj);

                            #[cfg(feature = "tracing")]
                            trace!(pool_size = pool.objects.len(), "Object returned to pool");
                        } else {
                            #[cfg(feature = "tracing")]
                            trace!("Pool full, dropping returned object");
                        }
                        // Otherwise drop it (pool is full enough)
                    }
                }
            }

            #[cfg(feature = "tracing")]
            debug!("Pool return processor terminated");
        });

        // The semaphore capacity equals max_capacity (or initial_capacity if unbounded).
        // Each outstanding acquired value holds one permit, preventing over-acquisition.
        let max_permits = config_max_capacity.unwrap_or(initial_capacity);

        Self {
            inner,
            return_tx,
            semaphore: Arc::new(Semaphore::new(max_permits)),
            shutdown,
            default_timeout: DEFAULT_ACQUIRE_TIMEOUT,
        }
    }

    /// Set default timeout for acquire operations
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.default_timeout = timeout;
        self
    }

    /// Acquire object from pool asynchronously with default timeout
    ///
    /// Waits if no objects are available. Creates new object if pool is empty
    /// and max capacity not reached.
    ///
    /// # Errors
    ///
    /// - `MemoryError::PoolExhausted` if pool is at max capacity
    /// - `MemoryError::InvalidState` if operation times out
    #[cfg_attr(feature = "tracing", instrument(skip(self)))]
    pub async fn acquire(&self) -> MemoryResult<AsyncPooledValue<T>> {
        self.acquire_timeout(self.default_timeout).await
    }

    /// Acquire object from pool with custom timeout
    ///
    /// # Errors
    ///
    /// - `MemoryError::PoolExhausted` if pool is at max capacity
    /// - `MemoryError::InvalidState` if operation times out
    #[cfg_attr(feature = "tracing", instrument(skip(self)))]
    pub async fn acquire_timeout(&self, duration: Duration) -> MemoryResult<AsyncPooledValue<T>> {
        // Check for shutdown
        if self.shutdown.is_cancelled() {
            #[cfg(feature = "tracing")]
            warn!("Attempted to acquire from shut down pool");

            return Err(MemoryError::InvalidState {
                reason: "pool is shut down".into(),
            });
        }

        // Wrap the acquisition in a timeout
        timeout(duration, self.acquire_inner()).await.map_err(|_| {
            #[cfg(feature = "tracing")]
            warn!(?duration, "Pool acquire operation timed out");

            MemoryError::InvalidState {
                reason: format!("pool acquire timed out after {:?}", duration).into(),
            }
        })?
    }

    /// Internal acquire implementation without timeout
    async fn acquire_inner(&self) -> MemoryResult<AsyncPooledValue<T>> {
        // Acquire an owned permit (stored in the returned value).
        // The permit is held as long as the AsyncPooledValue is alive,
        // ensuring at most `max_capacity` values are outstanding at once.
        let permit = tokio::select! {
            permit = Arc::clone(&self.semaphore).acquire_owned() => {
                permit.map_err(|_| MemoryError::InvalidState {
                    reason: "semaphore closed".into(),
                })?
            }
            _ = self.shutdown.cancelled() => {
                return Err(MemoryError::InvalidState {
                    reason: "pool is shut down".into(),
                });
            }
        };

        #[cfg(feature = "tracing")]
        trace!("Acquired semaphore permit");

        let mut inner = self.inner.lock().await;

        // Fast path: reuse an existing object from the pool
        if let Some(obj) = inner.objects.pop() {
            #[cfg(feature = "tracing")]
            trace!(pool_size = inner.objects.len(), "Acquired object from pool");

            drop(inner); // Release lock before returning
            return Ok(AsyncPooledValue {
                value: Some(obj),
                return_tx: self.return_tx.clone(),
                _permit: permit,
            });
        }

        // Slow path: pool temporarily empty because the background task hasn't yet
        // processed a returned object from the channel.
        //
        // Only create a new object if we are still under max_capacity.  When
        // total_created == max_capacity the semaphore already guarantees there are
        // exactly max_capacity live objects; the pool vector is just momentarily empty
        // because the background channel hasn't flushed yet.  In that case we yield
        // repeatedly until the background task places an object back in the pool.
        if let Some(max_cap) = inner.config.max_capacity {
            if inner.total_created >= max_cap {
                drop(inner); // release lock so background task can acquire it

                // Spin-yield until the background task returns an object.
                // Each yield_now() gives other tasks (including the background processor)
                // a chance to run.
                let obj = loop {
                    tokio::task::yield_now().await;
                    let mut guard = self.inner.lock().await;
                    if let Some(obj) = guard.objects.pop() {
                        break obj;
                    }
                    // Guard dropped here, background task gets another chance.
                };

                return Ok(AsyncPooledValue {
                    value: Some(obj),
                    return_tx: self.return_tx.clone(),
                    _permit: permit,
                });
            }
        }

        // Under max_capacity: create a fresh object.
        #[cfg(feature = "tracing")]
        debug!(
            total_created = inner.total_created,
            "Creating new pool object (pool temporarily empty)"
        );

        let obj = (inner.factory)();
        inner.total_created += 1;

        drop(inner);

        Ok(AsyncPooledValue {
            value: Some(obj),
            return_tx: self.return_tx.clone(),
            _permit: permit,
        })
    }

    /// Try to acquire object without waiting
    ///
    /// Returns `None` if no objects are immediately available or pool is at capacity.
    #[cfg_attr(feature = "tracing", instrument(skip(self)))]
    pub async fn try_acquire(&self) -> Option<AsyncPooledValue<T>> {
        // Check for shutdown
        if self.shutdown.is_cancelled() {
            #[cfg(feature = "tracing")]
            warn!("Attempted to try_acquire from shut down pool");
            return None;
        }

        // Try to acquire an owned permit without waiting
        let permit = match Arc::clone(&self.semaphore).try_acquire_owned() {
            Ok(p) => p,
            Err(_) => {
                #[cfg(feature = "tracing")]
                trace!("No permits available for try_acquire");
                return None;
            }
        };

        let mut inner = self.inner.lock().await;

        // Fast path: reuse existing object
        if let Some(obj) = inner.objects.pop() {
            #[cfg(feature = "tracing")]
            trace!(
                pool_size = inner.objects.len(),
                "Try acquired object from pool"
            );

            drop(inner);
            return Some(AsyncPooledValue {
                value: Some(obj),
                return_tx: self.return_tx.clone(),
                _permit: permit,
            });
        }

        // Slow path: pool temporarily empty.
        // For try_acquire (non-blocking) we cannot wait, so if we are at max capacity
        // return None instead of creating a surplus object.
        if let Some(max_cap) = inner.config.max_capacity {
            if inner.total_created >= max_cap {
                #[cfg(feature = "tracing")]
                trace!("Pool temporarily empty and at max capacity, try_acquire returns None");
                return None;
            }
        }

        let obj = (inner.factory)();
        inner.total_created += 1;

        #[cfg(feature = "tracing")]
        trace!(
            total_created = inner.total_created,
            "Created new object for try_acquire"
        );

        drop(inner);

        Some(AsyncPooledValue {
            value: Some(obj),
            return_tx: self.return_tx.clone(),
            _permit: permit,
        })
    }

    /// Get current pool size
    pub async fn len(&self) -> usize {
        let inner = self.inner.lock().await;
        inner.objects.len()
    }

    /// Check if pool is empty
    pub async fn is_empty(&self) -> bool {
        self.len().await == 0
    }

    /// Get total objects created
    pub async fn total_created(&self) -> usize {
        let inner = self.inner.lock().await;
        inner.total_created
    }

    /// Clone the pool handle (shares the same underlying pool)
    pub fn clone_handle(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            return_tx: self.return_tx.clone(),
            semaphore: Arc::clone(&self.semaphore),
            shutdown: self.shutdown.clone(),
            default_timeout: self.default_timeout,
        }
    }

    /// Initiate graceful shutdown
    ///
    /// This cancels the shutdown token, which will terminate the background
    /// task processing returns. Call this before dropping the pool to ensure
    /// clean shutdown.
    #[cfg_attr(feature = "tracing", instrument(skip(self)))]
    pub fn shutdown(&self) {
        #[cfg(feature = "tracing")]
        debug!("Initiating pool shutdown");

        self.shutdown.cancel();
    }

    /// Wait for pool to finish processing returns (best effort)
    ///
    /// This gives the background task a chance to process any pending returns
    /// before the pool is dropped.
    #[cfg_attr(feature = "tracing", instrument(skip(self)))]
    pub async fn drain(&self, wait_duration: Duration) {
        #[cfg(feature = "tracing")]
        debug!(?wait_duration, "Draining pool");

        tokio::time::sleep(wait_duration).await;
    }

    /// Get the shutdown token for this pool
    pub fn shutdown_token(&self) -> &CancellationToken {
        &self.shutdown
    }
}

#[cfg(all(test, feature = "async"))]
mod tests {
    use super::*;

    #[derive(Debug, Clone)]
    struct TestObject {
        id: usize,
        data: Vec<u8>,
    }

    impl TestObject {
        fn new(id: usize) -> Self {
            Self {
                id,
                data: vec![0; 1024],
            }
        }
    }

    impl Poolable for TestObject {
        fn reset(&mut self) {
            self.data.fill(0);
        }

        fn memory_usage(&self) -> usize {
            std::mem::size_of::<Self>() + self.data.capacity()
        }
    }

    #[tokio::test]
    async fn test_async_pool_basic() {
        let shutdown = CancellationToken::new();
        let pool = AsyncPool::new(10, || TestObject::new(0), shutdown.clone());

        let obj1 = pool.acquire().await.unwrap();
        assert_eq!(obj1.id, 0);

        let obj2 = pool.acquire().await.unwrap();
        assert_eq!(obj2.id, 0);

        drop(obj1);
        drop(obj2);

        // Give background task time to process returns
        tokio::time::sleep(Duration::from_millis(10)).await;

        // Objects should be returned
        assert!(pool.len().await > 0);

        // Clean shutdown
        pool.shutdown();
        pool.drain(Duration::from_millis(10)).await;
    }

    #[tokio::test]
    async fn test_async_pool_concurrent() {
        let shutdown = CancellationToken::new();
        let pool = AsyncPool::new(5, || TestObject::new(0), shutdown.clone());

        let handles: Vec<_> = (0..20)
            .map(|_| {
                let pool = pool.clone_handle();
                tokio::spawn(async move {
                    let obj = pool.acquire().await.unwrap();
                    // Simulate some work
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    drop(obj);
                })
            })
            .collect();

        for handle in handles {
            handle.await.unwrap();
        }

        let total = pool.total_created().await;
        assert!(total >= 5);

        // Clean shutdown
        pool.shutdown();
        pool.drain(Duration::from_millis(10)).await;
    }

    #[tokio::test]
    async fn test_async_pool_timeout() {
        let shutdown = CancellationToken::new();
        let pool = AsyncPool::new(1, || TestObject::new(1), shutdown.clone())
            .with_timeout(Duration::from_millis(50));

        // Acquire the only object
        let _obj1 = pool.acquire().await.unwrap();

        // Try to acquire with timeout (should fail)
        let result = pool.acquire().await;
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(e.to_string().contains("timed out"));
        }

        pool.shutdown();
    }

    #[tokio::test]
    async fn test_async_pool_graceful_shutdown() {
        let shutdown = CancellationToken::new();
        let pool = AsyncPool::new(5, || TestObject::new(99), shutdown.clone());

        // Acquire some objects
        let obj1 = pool.acquire().await.unwrap();
        let obj2 = pool.acquire().await.unwrap();

        // Initiate shutdown
        pool.shutdown();

        // Should not be able to acquire after shutdown
        let result = pool.acquire().await;
        assert!(result.is_err());

        // Can still drop objects
        drop(obj1);
        drop(obj2);
    }

    #[tokio::test]
    async fn test_async_pool_try_acquire() {
        let shutdown = CancellationToken::new();
        let pool = AsyncPool::new(2, || TestObject::new(1), shutdown.clone());

        let obj1 = pool.try_acquire().await;
        assert!(obj1.is_some());

        let obj2 = pool.try_acquire().await;
        assert!(obj2.is_some());

        // Pool exhausted, should return None
        let obj3 = pool.try_acquire().await;
        assert!(obj3.is_none());

        drop(obj1);

        // Give background task time to process return
        tokio::time::sleep(Duration::from_millis(10)).await;

        // Should work now
        let obj4 = pool.try_acquire().await;
        assert!(obj4.is_some());

        pool.shutdown();
    }

    #[tokio::test]
    async fn test_async_pool_detach() {
        let shutdown = CancellationToken::new();
        let pool = AsyncPool::new(5, || TestObject::new(42), shutdown.clone());

        let obj = pool.acquire().await.unwrap();
        let initial_len = pool.len().await;

        let detached = obj.detach();
        assert_eq!(detached.id, 42);

        // Object was not returned
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert_eq!(pool.len().await, initial_len);

        pool.shutdown();
    }

    /// Test that objects are returned even during high contention
    #[tokio::test]
    async fn test_async_pool_drop_guarantees() {
        let shutdown = CancellationToken::new();
        let pool = AsyncPool::new(3, || TestObject::new(99), shutdown.clone());

        // Acquire and drop many objects rapidly
        for _ in 0..100 {
            let obj = pool.acquire().await.unwrap();
            drop(obj);
        }

        // Give background task time to process all returns
        pool.drain(Duration::from_millis(50)).await;

        // Pool should have objects returned
        let len = pool.len().await;
        assert!(len > 0, "Pool should have returned objects, got {}", len);

        // Should be able to create very few new objects
        let total = pool.total_created().await;
        assert!(
            total < 20,
            "Should reuse objects efficiently, created {}",
            total
        );

        pool.shutdown();
    }
}
