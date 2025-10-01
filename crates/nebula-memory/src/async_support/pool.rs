//! Async-friendly object pool

#[cfg(feature = "async")]
use std::sync::Arc;

#[cfg(feature = "async")]
use tokio::sync::{Mutex, Semaphore};

#[cfg(feature = "async")]
use crate::pool::{Poolable, PoolConfig};
#[cfg(feature = "async")]
use crate::core::error::{MemoryError, MemoryResult};

/// Async pooled value that returns to pool on drop
#[cfg(feature = "async")]
pub struct AsyncPooledValue<T: Poolable> {
    value: Option<T>,
    pool: Arc<Mutex<AsyncPoolInner<T>>>,
}

#[cfg(feature = "async")]
impl<T: Poolable> AsyncPooledValue<T> {
    /// Detach value from pool (won't be returned)
    pub fn detach(mut self) -> T {
        self.value.take().unwrap()
    }
}

#[cfg(feature = "async")]
impl<T: Poolable> std::ops::Deref for AsyncPooledValue<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.value.as_ref().unwrap()
    }
}

#[cfg(feature = "async")]
impl<T: Poolable> std::ops::DerefMut for AsyncPooledValue<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.value.as_mut().unwrap()
    }
}

#[cfg(feature = "async")]
impl<T: Poolable> Drop for AsyncPooledValue<T> {
    fn drop(&mut self) {
        if let Some(mut value) = self.value.take() {
            // Reset the object before returning
            value.reset();

            // Return to pool using blocking lock (best effort in Drop)
            if let Ok(mut pool) = self.pool.try_lock() {
                pool.return_object(value);
            }
        }
    }
}

/// Internal pool state
#[cfg(feature = "async")]
struct AsyncPoolInner<T: Poolable> {
    objects: Vec<T>,
    factory: Box<dyn Fn() -> T + Send + Sync>,
    config: PoolConfig,
    total_created: usize,
}

/// Async-friendly object pool
///
/// Provides async/await compatible object pooling with proper async locking
/// and waiting for available objects.
#[cfg(feature = "async")]
pub struct AsyncPool<T: Poolable> {
    inner: Arc<Mutex<AsyncPoolInner<T>>>,
    semaphore: Arc<Semaphore>,
}

#[cfg(feature = "async")]
impl<T: Poolable> AsyncPool<T> {
    /// Create new async pool with capacity and factory function
    pub fn new<F>(capacity: usize, factory: F) -> Self
    where
        F: Fn() -> T + Send + Sync + 'static,
    {
        let mut config = PoolConfig::default();
        config.initial_capacity = capacity;
        config.max_capacity = Some(capacity * 2);
        config.validate_on_return = true;

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

        let inner = AsyncPoolInner {
            objects,
            factory: Box::new(factory),
            config,
            total_created: initial_capacity,
        };

        Self {
            inner: Arc::new(Mutex::new(inner)),
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
                pool: Arc::clone(&self.inner),
            });
        }

        // Create new object if under max capacity
        if let Some(max_cap) = inner.config.max_capacity {
            if inner.total_created >= max_cap {
                return Err(MemoryError::pool_exhausted(
                    "AsyncPool max capacity reached"
                ));
            }
        }

        let obj = (inner.factory)();
        inner.total_created += 1;

        drop(inner);

        Ok(AsyncPooledValue {
            value: Some(obj),
            pool: Arc::clone(&self.inner),
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
                pool: Arc::clone(&self.inner),
            });
        }

        // Create new if possible
        if let Some(max_cap) = inner.config.max_capacity {
            if inner.total_created >= max_cap {
                return None;
            }
        }

        let obj = (inner.factory)();
        inner.total_created += 1;

        drop(inner);

        Some(AsyncPooledValue {
            value: Some(obj),
            pool: Arc::clone(&self.inner),
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
            semaphore: Arc::clone(&self.semaphore),
        }
    }
}

#[cfg(feature = "async")]
impl<T: Poolable> AsyncPoolInner<T> {
    fn return_object(&mut self, obj: T) {
        if self.objects.len() < self.config.initial_capacity {
            self.objects.push(obj);
        }
        // Otherwise drop it (pool is full enough)
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
        assert_eq!(pool.len().await, initial_len);
    }
}
