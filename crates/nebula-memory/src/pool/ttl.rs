//! Time-to-live object pool

#[cfg(not(feature = "std"))]
use alloc::{boxed::Box, collections::VecDeque, vec::Vec};
use core::mem::ManuallyDrop;
use core::ops::{Deref, DerefMut};
#[cfg(feature = "std")]
use std::collections::VecDeque;
#[cfg(feature = "std")]
use std::time::{Duration, Instant};

#[cfg(feature = "stats")]
use super::PoolStats;
use super::{NoOpCallbacks, PoolCallbacks, PoolConfig, Poolable};
use crate::core::error::{MemoryError, MemoryResult};

/// Object pool with time-to-live for pooled objects
///
/// Objects are automatically removed from the pool after their TTL expires.
/// This is useful for objects that may become stale or invalid over time.
///
/// # Example
/// ```
/// use std::time::Duration;
///
/// use nebula_memory::pool::TtlPool;
///
/// // Pool with 30-second TTL
/// let mut pool = TtlPool::new(100, Duration::from_secs(30), || Vec::<u8>::with_capacity(1024));
/// ```
#[cfg(feature = "std")]
pub struct TtlPool<T: Poolable> {
    objects: VecDeque<TtlWrapper<T>>,
    factory: Box<dyn Fn() -> T>,
    ttl: Duration,
    config: PoolConfig,
    callbacks: Box<dyn PoolCallbacks<T>>,
    last_cleanup: Instant,
    cleanup_interval: Duration,
    #[cfg(feature = "stats")]
    stats: PoolStats,
}

#[cfg(feature = "std")]
struct TtlWrapper<T> {
    value: T,
    created_at: Instant,
}

#[cfg(feature = "std")]
impl<T: Poolable> TtlPool<T> {
    /// Create new TTL pool
    pub fn new<F>(capacity: usize, ttl: Duration, factory: F) -> Self
    where
        F: Fn() -> T + 'static,
    {
        Self::with_config(
            PoolConfig {
                initial_capacity: capacity,
                ttl: Some(ttl),
                ..Default::default()
            },
            factory,
        )
    }

    /// Create pool with custom configuration
    pub fn with_config<F>(mut config: PoolConfig, factory: F) -> Self
    where
        F: Fn() -> T + 'static,
    {
        let ttl = config.ttl.unwrap_or(Duration::from_secs(300)); // 5 min default
        config.ttl = Some(ttl);

        let mut objects = VecDeque::with_capacity(config.initial_capacity);

        #[cfg(feature = "stats")]
        let stats = PoolStats::default();

        let now = Instant::now();

        // Pre-warm pool if configured
        if config.pre_warm {
            for _ in 0..config.initial_capacity {
                let obj = factory();
                #[cfg(feature = "stats")]
                stats.record_creation();
                objects.push_back(TtlWrapper {
                    value: obj,
                    created_at: now,
                });
            }
        }

        Self {
            objects,
            factory: Box::new(factory),
            ttl,
            config,
            callbacks: Box::new(NoOpCallbacks),
            last_cleanup: now,
            cleanup_interval: ttl / 10, // Cleanup every 10% of TTL
            #[cfg(feature = "stats")]
            stats,
        }
    }

    /// Set callbacks for pool events
    pub fn with_callbacks<C: PoolCallbacks<T> + 'static>(mut self, callbacks: C) -> Self {
        self.callbacks = Box::new(callbacks);
        self
    }

    /// Set cleanup interval
    pub fn with_cleanup_interval(mut self, interval: Duration) -> Self {
        self.cleanup_interval = interval;
        self
    }

    /// Remove expired objects
    fn cleanup_expired(&mut self) {
        let now = Instant::now();

        // Only cleanup periodically
        if now.duration_since(self.last_cleanup) < self.cleanup_interval {
            return;
        }

        self.last_cleanup = now;

        // Remove expired objects from front (oldest first)
        while let Some(wrapper) = self.objects.front() {
            if now.duration_since(wrapper.created_at) > self.ttl {
                if let Some(wrapper) = self.objects.pop_front() {
                    self.callbacks.on_destroy(&wrapper.value);
                    #[cfg(feature = "stats")]
                    self.stats.record_destruction();
                }
            } else {
                break; // Remaining objects are newer
            }
        }
    }

    /// Get object from pool
    pub fn get(&mut self) -> MemoryResult<TtlPooledValue<T>> {
        #[cfg(feature = "stats")]
        self.stats.record_get();

        // Cleanup expired objects
        self.cleanup_expired();

        let obj = if let Some(wrapper) = self.objects.pop_back() {
            #[cfg(feature = "stats")]
            self.stats.record_hit();

            let mut obj = wrapper.value;
            obj.reset();
            self.callbacks.on_checkout(&obj);
            obj
        } else {
            #[cfg(feature = "stats")]
            self.stats.record_miss();

            // Check capacity
            if let Some(max) = self.config.max_capacity {
                #[cfg(feature = "stats")]
                let created = self.stats.total_created();
                #[cfg(not(feature = "stats"))]
                let created = 0;

                if created >= max {
                    return Err(MemoryError::pool_exhausted());
                }
            }

            let obj = (self.factory)();
            self.callbacks.on_create(&obj);

            #[cfg(feature = "stats")]
            self.stats.record_creation();

            obj
        };

        Ok(TtlPooledValue {
            value: ManuallyDrop::new(obj),
            pool: self as *mut _,
        })
    }

    /// Return object to pool
    pub(crate) fn return_object(&mut self, mut obj: T) {
        #[cfg(feature = "stats")]
        self.stats.record_return();

        self.callbacks.on_checkin(&obj);

        // Validate object
        if self.config.validate_on_return {
            if !obj.validate() || !obj.is_reusable() {
                self.callbacks.on_destroy(&obj);
                #[cfg(feature = "stats")]
                self.stats.record_destruction();
                return;
            }
        }

        // Check pool size
        if let Some(max) = self.config.max_capacity {
            if self.objects.len() >= max {
                self.callbacks.on_destroy(&obj);
                #[cfg(feature = "stats")]
                self.stats.record_destruction();
                return;
            }
        }

        obj.reset();
        self.objects.push_back(TtlWrapper {
            value: obj,
            created_at: Instant::now(),
        });
    }

    /// Force cleanup of expired objects
    pub fn force_cleanup(&mut self) {
        self.last_cleanup = Instant::now() - self.cleanup_interval;
        self.cleanup_expired();
    }

    /// Clear all objects
    pub fn clear(&mut self) {
        for wrapper in self.objects.drain(..) {
            self.callbacks.on_destroy(&wrapper.value);
        }

        #[cfg(feature = "stats")]
        self.stats.record_clear();
    }

    /// Get number of available objects
    pub fn available(&self) -> usize {
        self.objects.len()
    }

    /// Get age of oldest object
    pub fn oldest_age(&self) -> Option<Duration> {
        self.objects.front().map(|w| w.created_at.elapsed())
    }

    /// Get pool statistics
    #[cfg(feature = "stats")]
    pub fn stats(&self) -> &PoolStats {
        &self.stats
    }
}

/// RAII wrapper for TTL pooled values
#[cfg(feature = "std")]
pub struct TtlPooledValue<T: Poolable> {
    value: ManuallyDrop<T>,
    pool: *mut TtlPool<T>,
}

#[cfg(feature = "std")]
impl<T: Poolable> TtlPooledValue<T> {
    /// Detach value from pool
    pub fn detach(mut self) -> T {
        let value = unsafe { ManuallyDrop::take(&mut self.value) };
        core::mem::forget(self);
        value
    }
}

#[cfg(feature = "std")]
impl<T: Poolable> Deref for TtlPooledValue<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

#[cfg(feature = "std")]
impl<T: Poolable> DerefMut for TtlPooledValue<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.value
    }
}

#[cfg(feature = "std")]
impl<T: Poolable> Drop for TtlPooledValue<T> {
    fn drop(&mut self) {
        unsafe {
            let obj = ManuallyDrop::take(&mut self.value);
            (*self.pool).return_object(obj);
        }
    }
}

// Safety: TtlPooledValue can be sent if T can
#[cfg(feature = "std")]
unsafe impl<T: Poolable + Send> Send for TtlPooledValue<T> {}

// No-std stub
#[cfg(not(feature = "std"))]
pub struct TtlPool<T: Poolable> {
    _phantom: core::marker::PhantomData<T>,
}

#[cfg(not(feature = "std"))]
impl<T: Poolable> TtlPool<T> {
    pub fn new<F>(_capacity: usize, _ttl: Duration, _factory: F) -> Self {
        panic!("TTL pool requires std feature");
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use std::thread;

    use super::*;

    #[derive(Debug, PartialEq)]
    struct TestObject {
        value: i32,
    }

    impl Poolable for TestObject {
        fn reset(&mut self) {
            self.value = 0;
        }
    }

    #[test]
    fn test_ttl_expiration() {
        let mut pool = TtlPool::new(10, Duration::from_millis(100), || TestObject { value: 42 });

        // Return some objects
        pool.return_object(TestObject { value: 1 });
        pool.return_object(TestObject { value: 2 });

        assert_eq!(pool.available(), 2);

        // Wait for TTL to expire
        thread::sleep(Duration::from_millis(150));

        // Force cleanup
        pool.force_cleanup();

        // Objects should be expired
        assert_eq!(pool.available(), 0);
    }

    #[test]
    fn test_ttl_ordering() {
        let mut pool = TtlPool::new(10, Duration::from_secs(10), || TestObject { value: 0 });

        // Return objects at different times
        pool.return_object(TestObject { value: 1 });
        thread::sleep(Duration::from_millis(10));
        pool.return_object(TestObject { value: 2 });

        // Should get newest object first (LIFO within TTL)
        let obj = pool.get().unwrap();
        assert_eq!(obj.value, 0); // Reset value

        // Age should be minimal for newest
        assert!(pool.oldest_age().unwrap() >= Duration::from_millis(10));
    }
}
