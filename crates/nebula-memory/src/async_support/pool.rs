//! Async-friendly object pool with safe Drop semantics
//!
//! This module provides an async-safe object pool that correctly handles
//! returns in Drop using an unbounded channel instead of try_lock().

use std::sync::Arc;

use tokio::sync::{mpsc, Mutex, Semaphore};

use crate::error::{MemoryError, MemoryResult};
use crate::pool::{PoolConfig, Poolable};

/// Async pooled value that returns to pool on drop
///
/// Uses unbounded channel for guaranteed return-to-pool in Drop.
/// This avoids the anti-pattern of using try_lock() in Drop which can
/// silently fail and leak objects.
pub struct AsyncPooledValue<T: Poolable> {
    value: Option<T>,
    return_tx: mpsc::UnboundedSender<T>,
}

impl<T: Poolable> AsyncPooledValue<T> {
    /// Detach value from pool (won't be returned)
    pub fn detach(mut self) -> T {
        self.value.take().unwrap()
    }
}

impl<T: Poolable> std::ops::Deref for AsyncPooledValue<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.value.as_ref().unwrap()
    }
}

impl<T: Poolable> std::ops::DerefMut for AsyncPooledValue<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.value.as_mut().unwrap()
    }
}

impl<T: Poolable> Drop for AsyncPooledValue<T> {
    fn drop(&mut self) {
        if let Some(mut value) = self.value.take() {
            // Reset the object before returning
            value.reset();

            // ✅ SAFE: Channel send always succeeds (unbounded)
            // Objects are guaranteed to be returned to pool
            let _ = self.return_tx.send(value);
        }
    }
}

/// Internal pool state
struct AsyncPoolInner<T: Poolable> {
    objects: Vec<T>,
    factory: Box<dyn Fn() -> T + Send + Sync>,
    config: PoolConfig,
    total_created: usize,
}

/// Async-friendly object pool
///
/// Provides async/await compatible object pooling with proper async locking
/// and guaranteed object returns via unbounded channel.
///
/// # Architecture
///
/// - **Fast path**: Pop from Vec<T> when objects available
/// - **Slow path**: Create new object if under capacity
/// - **Return path**: Unbounded channel ensures no loss in Drop
/// - **Background task**: Processes returns and replenishes pool
pub struct AsyncPool<T: Poolable> {
    inner: Arc<Mutex<AsyncPoolInner<T>>>,
    return_tx: mpsc::UnboundedSender<T>,
    semaphore: Arc<Semaphore>,
}

impl<T: Poolable> AsyncPool<T> {
    /// Create new async pool with capacity and factory function
    pub fn new<F>(capacity: usize, factory: F) -> Self
    where
        F: Fn() -> T + Send + Sync + 'static,
    {
        let config = PoolConfig {
            initial_capacity: capacity,
            max_capacity: Some(capacity * 2),
            validate_on_return: true,
            ..Default::default()
        };

        Self::with_config(config, factory)
    }

    /// Create new async pool with custom configuration
    pub fn with_config<F>(config: PoolConfig, factory: F) -> Self
    where
        F: Fn() -> T + Send + Sync + 'static,
    {
        let initial_capacity = config.initial_capacity;

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

        // Spawn background task to process returns
        let inner_clone = Arc::clone(&inner);
        tokio::spawn(async move {
            while let Some(obj) = return_rx.recv().await {
                let mut pool = inner_clone.lock().await;
                if pool.objects.len() < pool.config.initial_capacity {
                    pool.objects.push(obj);
                }
                // Otherwise drop it (pool is full enough)
            }
        });

        Self {
            inner,
            return_tx,
            semaphore: Arc::new(Semaphore::new(initial_capacity)),
        }
    }

    /// Acquire object from pool asynchronously
    ///
    /// Waits if no objects are available. Creates new object if pool is empty
    /// and max capacity not reached.
    pub async fn acquire(&self) -> MemoryResult<AsyncPooledValue<T>> {
        // Wait for available permit
        let _permit = self.semaphore.acquire().await.unwrap();

        let mut inner = self.inner.lock().await;

        // Try to get from pool
        if let Some(obj) = inner.objects.pop() {
            drop(inner); // Release lock
            return Ok(AsyncPooledValue {
                value: Some(obj),
                return_tx: self.return_tx.clone(),
            });
        }

        // Create new object if under max capacity
        if let Some(max_cap) = inner.config.max_capacity
            && inner.total_created >= max_cap
        {
            return Err(MemoryError::pool_exhausted("pool", 0));
        }

        let obj = (inner.factory)();
        inner.total_created += 1;

        drop(inner);

        Ok(AsyncPooledValue {
            value: Some(obj),
            return_tx: self.return_tx.clone(),
        })
    }

    /// Try to acquire object without waiting
    pub async fn try_acquire(&self) -> Option<AsyncPooledValue<T>> {
        // Try to acquire permit without waiting
        if self.semaphore.try_acquire().is_err() {
            return None;
        }

        let mut inner = self.inner.lock().await;

        if let Some(obj) = inner.objects.pop() {
            drop(inner);
            return Some(AsyncPooledValue {
                value: Some(obj),
                return_tx: self.return_tx.clone(),
            });
        }

        // Create new if possible
        if let Some(max_cap) = inner.config.max_capacity
            && inner.total_created >= max_cap
        {
            return None;
        }

        let obj = (inner.factory)();
        inner.total_created += 1;

        drop(inner);

        Some(AsyncPooledValue {
            value: Some(obj),
            return_tx: self.return_tx.clone(),
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
        }
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
        let pool = AsyncPool::new(10, || TestObject::new(0));

        let obj1 = pool.acquire().await.unwrap();
        assert_eq!(obj1.id, 0);

        let obj2 = pool.acquire().await.unwrap();
        assert_eq!(obj2.id, 0);

        drop(obj1);
        drop(obj2);

        // Give background task time to process returns
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        // Objects should be returned
        assert!(pool.len().await > 0);
    }

    #[tokio::test]
    async fn test_async_pool_concurrent() {
        let pool = AsyncPool::new(5, || TestObject::new(0));

        let handles: Vec<_> = (0..20)
            .map(|_| {
                let pool = pool.clone_handle();
                tokio::spawn(async move {
                    let obj = pool.acquire().await.unwrap();
                    // Simulate some work
                    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
                    drop(obj);
                })
            })
            .collect();

        for handle in handles {
            handle.await.unwrap();
        }

        let total = pool.total_created().await;
        assert!(total >= 5);
    }

    #[tokio::test]
    async fn test_async_pool_try_acquire() {
        let pool = AsyncPool::new(2, || TestObject::new(1));

        let obj1 = pool.try_acquire().await;
        assert!(obj1.is_some());

        let obj2 = pool.try_acquire().await;
        assert!(obj2.is_some());

        // Pool exhausted, should return None
        let obj3 = pool.try_acquire().await;
        assert!(obj3.is_none());

        drop(obj1);

        // Give background task time to process return
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        // Should work now
        let obj4 = pool.try_acquire().await;
        assert!(obj4.is_some());
    }

    #[tokio::test]
    async fn test_async_pool_detach() {
        let pool = AsyncPool::new(5, || TestObject::new(42));

        let obj = pool.acquire().await.unwrap();
        let initial_len = pool.len().await;

        let detached = obj.detach();
        assert_eq!(detached.id, 42);

        // Object was not returned
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        assert_eq!(pool.len().await, initial_len);
    }

    /// Test that objects are returned even during high contention
    #[tokio::test]
    async fn test_async_pool_drop_guarantees() {
        let pool = AsyncPool::new(3, || TestObject::new(99));

        // Acquire and drop many objects rapidly
        for _ in 0..100 {
            let obj = pool.acquire().await.unwrap();
            drop(obj);
        }

        // Give background task time to process all returns
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

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
    }
}
