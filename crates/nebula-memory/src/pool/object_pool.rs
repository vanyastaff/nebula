//! Core object pool implementation

#[cfg(not(feature = "std"))]
use alloc::boxed::Box;
#[cfg(not(feature = "std"))]
use alloc::vec::Vec;
use core::mem::ManuallyDrop;
use core::ops::{Deref, DerefMut};
use core::ptr::NonNull;

use super::{NoOpCallbacks, PoolCallbacks, PoolConfig, Poolable};
#[cfg(feature = "stats")]
use super::PoolStats;
use crate::core::error::{MemoryError, MemoryResult};

/// Single-threaded object pool for efficient memory reuse
///
/// # Example
/// ```
/// use nebula_memory::pool::{ObjectPool, Poolable};
///
/// // Pool of reusable strings
/// let mut pool = ObjectPool::new(100, || String::with_capacity(1024));
///
/// // Get string from pool
/// let mut s = pool.get().unwrap();
/// s.push_str("Hello, World!");
///
/// // String is automatically returned when dropped
/// drop(s);
/// ```
pub struct ObjectPool<T: Poolable> {
    objects: Vec<T>,
    factory: Box<dyn Fn() -> T>,
    config: PoolConfig,
    callbacks: Box<dyn PoolCallbacks<T>>,
    #[cfg(feature = "stats")]
    stats: PoolStats,
}

impl<T: Poolable> ObjectPool<T> {
    /// Create new pool with factory function
    pub fn new<F>(capacity: usize, factory: F) -> Self
    where F: Fn() -> T + 'static {
        Self::with_config(PoolConfig { initial_capacity: capacity, ..Default::default() }, factory)
    }

    /// Create pool with custom configuration
    pub fn with_config<F>(config: PoolConfig, factory: F) -> Self
    where F: Fn() -> T + 'static {
        let mut objects = Vec::with_capacity(config.initial_capacity);

        #[cfg(feature = "stats")]
        let stats = PoolStats::default();

        // Pre-warm pool if configured
        if config.pre_warm {
            for _ in 0..config.initial_capacity {
                let obj = factory();
                #[cfg(feature = "stats")]
                {
                    stats.record_creation();
                    stats.update_memory(
                        objects.iter().map(|o: &T| o.memory_usage()).sum::<usize>()
                            + obj.memory_usage(),
                    );
                }
                objects.push(obj);
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
    pub fn with_callbacks<C: PoolCallbacks<T> + 'static>(mut self, callbacks: C) -> Self {
        self.callbacks = Box::new(callbacks);
        self
    }

    /// Get object from pool
    pub fn get(&mut self) -> MemoryResult<PooledValue<T>> {
        #[cfg(feature = "stats")]
        self.stats.record_get();

        let obj = if let Some(mut obj) = self.objects.pop() {
            // Got object from pool
            #[cfg(feature = "stats")]
            self.stats.record_hit();

            obj.reset();
            self.callbacks.on_checkout(&obj);
            obj
        } else {
            // Pool is empty, create new object
            #[cfg(feature = "stats")]
            self.stats.record_miss();

            // Check capacity limits
            if let Some(max) = self.config.max_capacity {
                #[cfg(feature = "stats")]
                let created = self.stats.total_created();
                #[cfg(not(feature = "stats"))]
                let created = 0;

                if created >= max {
                    return Err(MemoryError::pool_exhausted());
                }
            }

            // Create new object
            let obj = (self.factory)();
            self.callbacks.on_create(&obj);

            #[cfg(feature = "stats")]
            {
                self.stats.record_creation();
                self.update_memory_stats();
            }

            obj
        };

        Ok(PooledValue {
            value: ManuallyDrop::new(obj),
            pool: unsafe { NonNull::new_unchecked(self as *mut _) },
        })
    }

    /// Try to get object without creating new one
    pub fn try_get(&mut self) -> Option<PooledValue<T>> {
        #[cfg(feature = "stats")]
        self.stats.record_get();

        self.objects.pop().map(|mut obj| {
            #[cfg(feature = "stats")]
            self.stats.record_hit();

            obj.reset();
            self.callbacks.on_checkout(&obj);

            PooledValue {
                value: ManuallyDrop::new(obj),
                pool: unsafe { NonNull::new_unchecked(self as *mut _) },
            }
        })
    }

    /// Return object to pool
    pub(crate) fn return_object(&mut self, mut obj: T) {
        #[cfg(feature = "stats")]
        self.stats.record_return();

        self.callbacks.on_checkin(&obj);

        // Validate object if configured
        if self.config.validate_on_return {
            if !obj.validate() || !obj.is_reusable() {
                self.callbacks.on_destroy(&obj);
                #[cfg(feature = "stats")]
                {
                    self.stats.record_destruction();
                    self.update_memory_stats();
                }
                return;
            }
        }

        // Check if we should grow the pool
        if let Some(max) = self.config.max_capacity {
            if self.objects.len() >= max {
                self.callbacks.on_destroy(&obj);
                #[cfg(feature = "stats")]
                {
                    self.stats.record_destruction();
                    self.update_memory_stats();
                }
                return;
            }
        }

        obj.reset();
        self.objects.push(obj);

        #[cfg(feature = "stats")]
        self.update_memory_stats();

        // Try to optimize memory if under pressure
        #[cfg(feature = "adaptive")]
        {
            if self.should_optimize_memory() {
                self.optimize_memory();
            }
        }
    }

    /// Pre-allocate objects
    pub fn reserve(&mut self, additional: usize) -> MemoryResult<()> {
        // Check capacity
        if let Some(max) = self.config.max_capacity {
            let current = self.objects.len();
            let new_total = current.saturating_add(additional);
            if new_total > max {
                return Err(MemoryError::budget_exceeded());
            }
        }

        self.objects.reserve(additional);

        for _ in 0..additional {
            let obj = (self.factory)();
            self.callbacks.on_create(&obj);
            self.objects.push(obj);

            #[cfg(feature = "stats")]
            self.stats.record_creation();
        }

        #[cfg(feature = "stats")]
        self.update_memory_stats();

        Ok(())
    }

    /// Shrink pool to specified size
    pub fn shrink_to(&mut self, size: usize) {
        while self.objects.len() > size {
            if let Some(obj) = self.objects.pop() {
                self.callbacks.on_destroy(&obj);
                #[cfg(feature = "stats")]
                self.stats.record_destruction();
            }
        }

        self.objects.shrink_to_fit();

        #[cfg(feature = "stats")]
        self.update_memory_stats();
    }

    /// Clear all pooled objects
    pub fn clear(&mut self) {
        for obj in self.objects.drain(..) {
            self.callbacks.on_destroy(&obj);
        }

        #[cfg(feature = "stats")]
        {
            self.stats.record_clear();
            self.update_memory_stats();
        }
    }

    /// Get number of available objects
    pub fn available(&self) -> usize {
        self.objects.len()
    }

    /// Get pool capacity
    pub fn capacity(&self) -> usize {
        self.objects.capacity()
    }

    /// Get pool statistics
    #[cfg(feature = "stats")]
    pub fn stats(&self) -> &PoolStats {
        &self.stats
    }

    /// Get pool statistics (empty stats when feature is disabled)
    #[cfg(not(feature = "stats"))]
    pub fn stats(&self) -> PoolStats {
        PoolStats::default()
    }

    /// Update memory statistics
    #[cfg(feature = "stats")]
    fn update_memory_stats(&self) {
        let memory = self.objects.iter().map(|obj| obj.memory_usage()).sum::<usize>()
            + self.objects.capacity() * core::mem::size_of::<T>();

        self.stats.update_memory(memory);
    }

    /// Optimize memory usage by compressing objects
    ///
    /// This method attempts to reduce memory usage of all pooled objects
    /// by calling their `compress()` method when memory pressure is high.
    ///
    /// Returns the amount of memory saved in bytes.
    #[cfg(feature = "adaptive")]
    pub fn optimize_memory(&mut self) -> usize {
        let mut total_saved = 0;

        for obj in &mut self.objects {
            let before_size = obj.memory_usage();
            let success = obj.compress();
            let after_size = obj.memory_usage();

            #[cfg(feature = "stats")]
            self.stats.record_compression_attempt(before_size, after_size, success);

            if before_size > after_size {
                total_saved += before_size - after_size;
            }
        }

        #[cfg(feature = "stats")]
        if self.objects.len() > 0 {
            self.stats.update_memory(self.objects.iter().map(|o| o.memory_usage()).sum());
        }

        total_saved
    }

    /// Check if pool should optimize memory based on pressure threshold
    ///
    /// Returns true if optimization should be performed.
    #[cfg(feature = "adaptive")]
    pub fn should_optimize_memory(&self) -> bool {
        // Check against pressure threshold from config
        if let Some(max) = self.config.max_capacity {
            let usage_percent = (self.objects.len() * 100) / max;
            usage_percent >= self.config.pressure_threshold as usize
        } else {
            // For unbounded pools, use a heuristic based on current size
            self.objects.len() > 1000 && self.objects.capacity() > self.objects.len() * 2
        }
    }

    /// Try to optimize memory if needed
    ///
    /// Returns amount of memory saved, or 0 if optimization wasn't needed.
    #[cfg(feature = "adaptive")]
    pub fn try_optimize_memory(&mut self) -> usize {
        if self.should_optimize_memory() {
            self.optimize_memory()
        } else {
            0
        }
    }
}

/// RAII wrapper for pooled values
pub struct PooledValue<T: Poolable> {
    value: ManuallyDrop<T>,
    pool: NonNull<ObjectPool<T>>,
}

impl<T: Poolable> PooledValue<T> {
    /// Detach value from pool (won't be returned)
    pub fn detach(mut self) -> T {
        let value = unsafe { ManuallyDrop::take(&mut self.value) };
        core::mem::forget(self);
        value
    }

    /// Get reference to the pool
    pub fn pool(&self) -> &ObjectPool<T> {
        unsafe { self.pool.as_ref() }
    }
}

impl<T: Poolable> Deref for PooledValue<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &*self.value
    }
}

impl<T: Poolable> DerefMut for PooledValue<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut *self.value
    }
}

impl<T: Poolable> Drop for PooledValue<T> {
    fn drop(&mut self) {
        unsafe {
            let obj = ManuallyDrop::take(&mut self.value);
            self.pool.as_mut().return_object(obj);
        }
    }
}

impl<T: Poolable> AsRef<T> for PooledValue<T> {
    fn as_ref(&self) -> &T {
        &self.value
    }
}

impl<T: Poolable> AsMut<T> for PooledValue<T> {
    fn as_mut(&mut self) -> &mut T {
        &mut self.value
    }
}

// Safety: PooledValue can be sent between threads if T can
unsafe impl<T: Poolable + Send> Send for PooledValue<T> {}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq)]
    struct TestObject {
        value: i32,
        resets: usize,
    }

    impl Poolable for TestObject {
        fn reset(&mut self) {
            self.value = 0;
            self.resets += 1;
        }
    }

    #[test]
    fn test_basic_pool_operations() {
        let mut pool = ObjectPool::new(10, || TestObject { value: 42, resets: 0 });

        // Get object
        let mut obj = pool.get().unwrap();
        assert_eq!(obj.value, 0); // Should be reset
        obj.value = 100;

        // Return happens on drop
        drop(obj);

        // Get again - should reuse
        let obj2 = pool.get().unwrap();
        assert_eq!(obj2.resets, 2); // Reset on first get and second get
    }

    #[test]
    fn test_pool_exhaustion() {
        let config = PoolConfig::bounded(2);
        let mut pool = ObjectPool::with_config(config, || TestObject { value: 0, resets: 0 });

        let _obj1 = pool.get().unwrap();
        let _obj2 = pool.get().unwrap();

        // Pool should be exhausted
        assert!(pool.get().is_err());
    }

    #[test]
    fn test_detach() {
        let mut pool = ObjectPool::new(10, || TestObject { value: 42, resets: 0 });

        let obj = pool.get().unwrap();
        let detached = obj.detach();

        assert_eq!(detached.value, 0);
        assert_eq!(pool.available(), 9); // Object not returned
    }

    #[cfg(feature = "adaptive")]
    #[test]
    fn test_memory_optimization() {
        // Create a test type that supports compression
        struct CompressibleObject {
            data: Vec<u8>,
            compressed: bool,
        }

        impl Poolable for CompressibleObject {
            fn reset(&mut self) {
                self.data.clear();
                self.compressed = false;
            }

            fn memory_usage(&self) -> usize {
                self.data.capacity() + core::mem::size_of::<Self>()
            }

            #[cfg(feature = "adaptive")]
            fn compress(&mut self) -> bool {
                if !self.compressed && self.data.capacity() > 100 {
                    // Simulate compression by reducing capacity
                    let mut new_data = Vec::with_capacity(100);
                    new_data.extend_from_slice(&self.data);
                    self.data = new_data;
                    self.compressed = true;
                    true
                } else {
                    false
                }
            }
        }

        // Create pool with adaptive config
        let mut config = PoolConfig::bounded(10);
        #[cfg(feature = "adaptive")]
        {
            config.pressure_threshold = 50; // 50% usage triggers optimization
        }

        let mut pool = ObjectPool::with_config(config, || {
            let mut obj = CompressibleObject { data: Vec::with_capacity(1000), compressed: false };
            obj.data.extend_from_slice(&[1, 2, 3, 4, 5]);
            obj
        });

        // Add objects to pool until we hit the threshold
        for _ in 0..6 {
            let obj = pool.get().unwrap();
            drop(obj);
        }

        // Check if we should optimize
        assert!(pool.should_optimize_memory());

        // Optimize memory
        let saved = pool.optimize_memory();
        assert!(saved > 0);

        // Objects should now be compressed
        let obj = pool.get().unwrap();
        assert!(obj.compressed);
        assert!(obj.data.capacity() <= 100);
    }

    #[cfg(feature = "adaptive")]
    #[test]
    fn test_automatic_memory_optimization() {
        // Create a test type that supports compression
        struct CompressibleObject {
            data: Vec<u8>,
            compressed: bool,
        }

        impl Poolable for CompressibleObject {
            fn reset(&mut self) {
                self.data.clear();
                self.compressed = false;
            }

            fn memory_usage(&self) -> usize {
                self.data.capacity() + core::mem::size_of::<Self>()
            }

            #[cfg(feature = "adaptive")]
            fn compress(&mut self) -> bool {
                if !self.compressed && self.data.capacity() > 100 {
                    // Simulate compression by reducing capacity
                    let mut new_data = Vec::with_capacity(100);
                    new_data.extend_from_slice(&self.data);
                    self.data = new_data;
                    self.compressed = true;
                    true
                } else {
                    false
                }
            }
        }

        // Create pool with adaptive config
        let mut config = PoolConfig::bounded(10);
        #[cfg(feature = "adaptive")]
        {
            config.pressure_threshold = 50; // 50% usage triggers optimization
        }

        let mut pool = ObjectPool::with_config(config, || {
            let mut obj = CompressibleObject { data: Vec::with_capacity(1000), compressed: false };
            obj.data.extend_from_slice(&[1, 2, 3, 4, 5]);
            obj
        });

        // Get some objects
        let obj1 = pool.get().unwrap();
        let obj2 = pool.get().unwrap();
        let obj3 = pool.get().unwrap();

        // Return objects one by one to trigger automatic optimization
        drop(obj1);
        drop(obj2);

        // When we return the third object, it should trigger optimization
        // because we'll reach 6/10 = 60% capacity which is above our 50% threshold
        drop(obj3);

        // Objects in pool should now be compressed due to automatic optimization
        let obj = pool.get().unwrap();
        assert!(obj.compressed);
        assert!(obj.data.capacity() <= 100);

        #[cfg(feature = "stats")]
        {
            let stats = pool.stats();
            // Check that compression stats were recorded
            assert!(stats.compression_attempts.load(core::sync::atomic::Ordering::Relaxed) > 0);
            assert!(stats.successful_compressions.load(core::sync::atomic::Ordering::Relaxed) > 0);
            assert!(stats.memory_saved.load(core::sync::atomic::Ordering::Relaxed) > 0);
        }
    }
}
