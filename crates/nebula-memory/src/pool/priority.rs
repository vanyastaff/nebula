//! Priority-based object pool

#[cfg(not(feature = "std"))]
use alloc::boxed::Box;
#[cfg(not(feature = "std"))]
use alloc::collections::BinaryHeap;
use core::cmp::Ordering as CmpOrdering;
use core::mem::ManuallyDrop;
use core::ops::{Deref, DerefMut};
#[cfg(feature = "std")]
use std::collections::BinaryHeap;

#[cfg(feature = "stats")]
use super::PoolStats;
use super::{NoOpCallbacks, PoolCallbacks, PoolConfig, Poolable};
use crate::error::{MemoryError, MemoryResult};

/// Object pool that maintains objects based on priority
///
/// Higher priority objects are kept longer when the pool needs to shrink.
/// This is useful for caching expensive-to-create objects.
///
/// # Example
/// ```
/// use nebula_memory::pool::{Poolable, PriorityPool};
///
/// struct Connection {
///     cost: u32,
/// }
///
/// impl Poolable for Connection {
///     fn reset(&mut self) {}
///
///     fn priority(&self) -> u8 {
///         // More expensive connections have higher priority
///         (self.cost / 100).min(255) as u8
///     }
/// }
/// ```
pub struct PriorityPool<T: Poolable> {
    objects: BinaryHeap<PriorityWrapper<T>>,
    factory: Box<dyn Fn() -> T>,
    config: PoolConfig,
    callbacks: Box<dyn PoolCallbacks<T>>,
    #[cfg(feature = "stats")]
    stats: PoolStats,
}

struct PriorityWrapper<T> {
    value: T,
    priority: u8,
}

impl<T> PartialEq for PriorityWrapper<T> {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority
    }
}

impl<T> Eq for PriorityWrapper<T> {}

impl<T> PartialOrd for PriorityWrapper<T> {
    fn partial_cmp(&self, other: &Self) -> Option<CmpOrdering> {
        Some(self.cmp(other))
    }
}

impl<T> Ord for PriorityWrapper<T> {
    fn cmp(&self, other: &Self) -> CmpOrdering {
        self.priority.cmp(&other.priority)
    }
}

impl<T: Poolable> PriorityPool<T> {
    /// Create new priority pool
    pub fn new<F>(capacity: usize, factory: F) -> Self
    where
        F: Fn() -> T + 'static,
    {
        Self::with_config(
            PoolConfig {
                initial_capacity: capacity,
                ..Default::default()
            },
            factory,
        )
    }

    /// Create pool with custom configuration
    pub fn with_config<F>(config: PoolConfig, factory: F) -> Self
    where
        F: Fn() -> T + 'static,
    {
        let mut objects = BinaryHeap::with_capacity(config.initial_capacity);

        #[cfg(feature = "stats")]
        let stats = PoolStats::default();

        // Pre-warm pool if configured
        if config.pre_warm {
            for _ in 0..config.initial_capacity {
                let obj = factory();
                let priority = obj.priority();
                #[cfg(feature = "stats")]
                stats.record_creation();
                objects.push(PriorityWrapper {
                    value: obj,
                    priority,
                });
            }
        }

        Self {
            objects,
            factory: Box::new(factory),
            config,
            callbacks: Box::new(NoOpCallbacks),
            #[cfg(feature = "stats")]
            stats,
        }
    }

    /// Set callbacks for pool events
    #[must_use = "builder methods must be chained or built"]
    pub fn with_callbacks<C: PoolCallbacks<T> + 'static>(mut self, callbacks: C) -> Self {
        self.callbacks = Box::new(callbacks);
        self
    }

    /// Get object from pool
    pub fn get(&mut self) -> MemoryResult<PriorityPooledValue<T>> {
        #[cfg(feature = "stats")]
        self.stats.record_get();

        let obj = if let Some(wrapper) = self.objects.pop() {
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
                    return Err(MemoryError::pool_exhausted("pool", 0));
                }
            }

            let obj = (self.factory)();
            self.callbacks.on_create(&obj);

            #[cfg(feature = "stats")]
            self.stats.record_creation();

            obj
        };

        Ok(PriorityPooledValue {
            value: ManuallyDrop::new(obj),
            pool: std::ptr::from_mut(self),
        })
    }

    /// Get object with minimum priority
    pub fn get_min_priority(&mut self, min_priority: u8) -> MemoryResult<PriorityPooledValue<T>> {
        #[cfg(feature = "stats")]
        self.stats.record_get();

        // Find object with sufficient priority
        let mut temp = Vec::new();
        let mut found = None;

        while let Some(wrapper) = self.objects.pop() {
            if wrapper.priority >= min_priority && found.is_none() {
                found = Some(wrapper);
                break;
            }
            temp.push(wrapper);
        }

        // Return objects we didn't use
        for wrapper in temp {
            self.objects.push(wrapper);
        }

        if let Some(wrapper) = found {
            #[cfg(feature = "stats")]
            self.stats.record_hit();

            let mut obj = wrapper.value;
            obj.reset();
            self.callbacks.on_checkout(&obj);

            Ok(PriorityPooledValue {
                value: ManuallyDrop::new(obj),
                pool: std::ptr::from_mut(self),
            })
        } else {
            // Create new object
            #[cfg(feature = "stats")]
            self.stats.record_miss();

            if let Some(max) = self.config.max_capacity {
                #[cfg(feature = "stats")]
                let created = self.stats.total_created();
                #[cfg(not(feature = "stats"))]
                let created = 0;

                if created >= max {
                    return Err(MemoryError::pool_exhausted("pool", 0));
                }
            }

            let obj = (self.factory)();
            self.callbacks.on_create(&obj);

            #[cfg(feature = "stats")]
            self.stats.record_creation();

            Ok(PriorityPooledValue {
                value: ManuallyDrop::new(obj),
                pool: std::ptr::from_mut(self),
            })
        }
    }

    /// Return object to pool
    pub(crate) fn return_object(&mut self, mut obj: T) {
        #[cfg(feature = "stats")]
        self.stats.record_return();

        self.callbacks.on_checkin(&obj);

        // Validate object
        if self.config.validate_on_return && (!obj.validate() || !obj.is_reusable()) {
            self.callbacks.on_destroy(&obj);
            #[cfg(feature = "stats")]
            self.stats.record_destruction();
            return;
        }

        let priority = obj.priority();

        // Check pool size - evict lowest priority if needed
        if let Some(max) = self.config.max_capacity
            && self.objects.len() >= max
        {
            // Check if new object has higher priority than lowest
            if let Some(lowest) = self.objects.peek()
                && priority <= lowest.priority
            {
                // Don't keep the new object
                self.callbacks.on_destroy(&obj);
                #[cfg(feature = "stats")]
                self.stats.record_destruction();
                return;
            }

            // Evict lowest priority object
            if let Some(wrapper) = self.objects.pop() {
                self.callbacks.on_destroy(&wrapper.value);
                #[cfg(feature = "stats")]
                self.stats.record_destruction();
            }
        }

        obj.reset();
        self.objects.push(PriorityWrapper {
            value: obj,
            priority,
        });
    }

    /// Shrink pool by removing lowest priority objects
    pub fn shrink_to(&mut self, size: usize) {
        while self.objects.len() > size {
            if let Some(wrapper) = self.objects.pop() {
                self.callbacks.on_destroy(&wrapper.value);
                #[cfg(feature = "stats")]
                self.stats.record_destruction();
            }
        }
    }

    /// Clear all objects
    pub fn clear(&mut self) {
        while let Some(wrapper) = self.objects.pop() {
            self.callbacks.on_destroy(&wrapper.value);
        }

        #[cfg(feature = "stats")]
        self.stats.record_clear();
    }

    /// Get number of available objects
    #[must_use]
    pub fn available(&self) -> usize {
        self.objects.len()
    }

    /// Get highest priority in pool
    #[must_use]
    pub fn highest_priority(&self) -> Option<u8> {
        self.objects.peek().map(|w| w.priority)
    }

    /// Get pool statistics
    #[cfg(feature = "stats")]
    pub fn stats(&self) -> &PoolStats {
        &self.stats
    }
}

/// RAII wrapper for priority pooled values
pub struct PriorityPooledValue<T: Poolable> {
    value: ManuallyDrop<T>,
    pool: *mut PriorityPool<T>,
}

impl<T: Poolable> PriorityPooledValue<T> {
    /// Detach value from pool
    pub fn detach(mut self) -> T {
        let value = unsafe { ManuallyDrop::take(&mut self.value) };
        core::mem::forget(self);
        value
    }
}

impl<T: Poolable> Deref for PriorityPooledValue<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<T: Poolable> DerefMut for PriorityPooledValue<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.value
    }
}

impl<T: Poolable> Drop for PriorityPooledValue<T> {
    fn drop(&mut self) {
        unsafe {
            let obj = ManuallyDrop::take(&mut self.value);
            (*self.pool).return_object(obj);
        }
    }
}

// Safety: PriorityPooledValue can be sent if T can
unsafe impl<T: Poolable + Send> Send for PriorityPooledValue<T> {}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct TestObject {
        value: i32,
        priority: u8,
    }

    impl Poolable for TestObject {
        fn reset(&mut self) {
            self.value = 0;
        }

        fn priority(&self) -> u8 {
            self.priority
        }
    }

    #[test]
    fn test_priority_order() {
        let mut pool = PriorityPool::new(10, || TestObject {
            value: 0,
            priority: 50,
        });

        // Return objects with different priorities
        let obj1 = TestObject {
            value: 1,
            priority: 100,
        };
        let obj2 = TestObject {
            value: 2,
            priority: 50,
        };
        let obj3 = TestObject {
            value: 3,
            priority: 150,
        };

        pool.return_object(obj1);
        pool.return_object(obj2);
        pool.return_object(obj3);

        // Should get highest priority first
        let obj = pool.get().unwrap();
        assert_eq!(obj.priority(), 150);
    }

    #[test]
    fn test_priority_eviction() {
        let config = PoolConfig::bounded(2);
        let mut pool = PriorityPool::with_config(config, || TestObject {
            value: 0,
            priority: 50,
        });

        // Fill pool
        pool.return_object(TestObject {
            value: 1,
            priority: 100,
        });
        pool.return_object(TestObject {
            value: 2,
            priority: 50,
        });

        // Try to add higher priority - should evict lowest
        pool.return_object(TestObject {
            value: 3,
            priority: 150,
        });
        assert_eq!(pool.available(), 2);

        // Highest priorities should remain
        assert_eq!(pool.highest_priority(), Some(150));
    }
}
