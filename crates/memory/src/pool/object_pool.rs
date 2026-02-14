//! Core object pool implementation
//!
//! # Safety
//!
//! This module implements single-threaded object pooling with RAII:
//! - `ObjectPool` owns all pooled objects in Vec<T>
//! - `PooledValue` uses `NonNull`<`ObjectPool`<T>> pointer to pool
//! - `ManuallyDrop` prevents automatic drop of value (manual control)
//! - Drop implementation returns object to pool
//!
//! ## Safety Contracts
//!
//! - `PooledValue::pool`: `NonNull` pointer created from &mut self (valid while pool exists)
//! - `PooledValue::detach`: `ManuallyDrop::take` extracts value, `mem::forget` prevents drop
//! - `PooledValue::drop`: `ManuallyDrop::take` + `pool.as_mut()` returns object
//! - `PooledValue::pool()`: Dereferences `NonNull` (safe while pool exists)
//! - Send implementation: Safe if T: Send (pool pointer not shared)

use core::cell::RefCell;
use core::mem::ManuallyDrop;
use core::ops::{Deref, DerefMut};

#[cfg(feature = "stats")]
use super::PoolStats;
use super::{NoOpCallbacks, PoolCallbacks, PoolConfig, Poolable};
use crate::error::{MemoryError, MemoryResult};

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
    objects: RefCell<Vec<T>>,
    factory: Box<dyn Fn() -> T>,
    config: PoolConfig,
    callbacks: Box<dyn PoolCallbacks<T>>,
    #[cfg(feature = "stats")]
    stats: PoolStats,
}

impl<T: Poolable> ObjectPool<T> {
    /// Create new pool with factory function
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
            objects: RefCell::new(objects),
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
    pub fn get(&self) -> MemoryResult<PooledValue<'_, T>> {
        #[cfg(feature = "stats")]
        self.stats.record_get();

        let obj = if let Some(mut obj) = self.objects.borrow_mut().pop() {
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
                    return Err(MemoryError::pool_exhausted("pool", 0));
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
            pool: self,
        })
    }

    /// Try to get object without creating new one
    pub fn try_get(&self) -> Option<PooledValue<'_, T>> {
        #[cfg(feature = "stats")]
        self.stats.record_get();

        self.objects.borrow_mut().pop().map(|mut obj| {
            #[cfg(feature = "stats")]
            self.stats.record_hit();

            obj.reset();
            self.callbacks.on_checkout(&obj);

            PooledValue {
                value: ManuallyDrop::new(obj),
                pool: self,
            }
        })
    }

    /// Return object to pool
    pub(crate) fn return_object(&self, mut obj: T) {
        #[cfg(feature = "stats")]
        self.stats.record_return();

        self.callbacks.on_checkin(&obj);

        // Validate object if configured
        if self.config.validate_on_return && (!obj.validate() || !obj.is_reusable()) {
            self.callbacks.on_destroy(&obj);
            #[cfg(feature = "stats")]
            {
                self.stats.record_destruction();
                self.update_memory_stats();
            }
            return;
        }

        // Check if we should grow the pool
        if let Some(max) = self.config.max_capacity {
            let len = self.objects.borrow().len();
            if len >= max {
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
        self.objects.borrow_mut().push(obj);

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
            let current = self.objects.borrow().len();
            let new_total = current.saturating_add(additional);
            if new_total > max {
                return Err(MemoryError::budget_exceeded(new_total, max));
            }
        }

        self.objects.borrow_mut().reserve(additional);

        for _ in 0..additional {
            let obj = (self.factory)();
            self.callbacks.on_create(&obj);
            self.objects.borrow_mut().push(obj);

            #[cfg(feature = "stats")]
            self.stats.record_creation();
        }

        #[cfg(feature = "stats")]
        self.update_memory_stats();

        Ok(())
    }

    /// Shrink pool to specified size
    pub fn shrink_to(&self, size: usize) {
        let mut objects = self.objects.borrow_mut();
        while objects.len() > size {
            if let Some(obj) = objects.pop() {
                self.callbacks.on_destroy(&obj);
                #[cfg(feature = "stats")]
                self.stats.record_destruction();
            }
        }

        objects.shrink_to_fit();

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
    #[must_use]
    pub fn available(&self) -> usize {
        self.objects.borrow().len()
    }

    /// Get pool capacity
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.objects.borrow().capacity()
    }

    /// Get pool statistics
    /// Note: This method is only available with the "stats" feature enabled
    #[cfg(feature = "stats")]
    pub fn stats(&self) -> &PoolStats {
        &self.stats
    }

    /// Update memory statistics
    #[cfg(feature = "stats")]
    fn update_memory_stats(&self) {
        let objects = self.objects.borrow();
        let memory = objects
            .iter()
            .map(|obj| obj.memory_usage())
            .sum::<usize>()
            + objects.capacity() * core::mem::size_of::<T>();

        self.stats.update_memory(memory);
    }

    /// Optimize memory usage by compressing objects
    ///
    /// This method attempts to reduce memory usage of all pooled objects
    /// by calling their `compress()` method when memory pressure is high.
    ///
    /// Returns the amount of memory saved in bytes.
    #[cfg(feature = "adaptive")]
    pub fn optimize_memory(&self) -> usize {
        let mut total_saved = 0;
        let mut objects = self.objects.borrow_mut();

        for obj in objects.iter_mut() {
            let before_size = obj.memory_usage();
            let success = obj.compress();
            let after_size = obj.memory_usage();

            #[cfg(feature = "stats")]
            self.stats
                .record_compression_attempt(before_size, after_size, success);

            if before_size > after_size {
                total_saved += before_size - after_size;
            }
        }

        #[cfg(feature = "stats")]
        if !objects.is_empty() {
            self.stats
                .update_memory(objects.iter().map(|o| o.memory_usage()).sum());
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
            let len = self.objects.borrow().len();
            let usage_percent = (len * 100) / max;
            usage_percent >= self.config.pressure_threshold as usize
        } else {
            // For unbounded pools, use a heuristic based on current size
            let objects = self.objects.borrow();
            objects.len() > 1000 && objects.capacity() > objects.len() * 2
        }
    }

    /// Try to optimize memory if needed
    ///
    /// Returns amount of memory saved, or 0 if optimization wasn't needed.
    #[cfg(feature = "adaptive")]
    pub fn try_optimize_memory(&self) -> usize {
        if self.should_optimize_memory() {
            self.optimize_memory()
        } else {
            0
        }
    }
}

/// RAII wrapper for pooled values
pub struct PooledValue<'a, T: Poolable> {
    value: ManuallyDrop<T>,
    pool: &'a ObjectPool<T>,
}

impl<'a, T: Poolable> PooledValue<'a, T> {
    /// Detach value from pool (won't be returned)
    pub fn detach(mut self) -> T {
        // SAFETY: Extracting value from ManuallyDrop.
        // - value is initialized (created in ObjectPool::get)
        // - mem::forget prevents Drop::drop from running
        // - No double-free (Drop won't return object to pool)
        let value = unsafe { ManuallyDrop::take(&mut self.value) };
        core::mem::forget(self);
        value
    }

    /// Get reference to the pool
    pub fn pool(&self) -> &ObjectPool<T> {
        self.pool
    }
}

impl<'a, T: Poolable> Deref for PooledValue<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<'a, T: Poolable> DerefMut for PooledValue<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.value
    }
}

impl<'a, T: Poolable> Drop for PooledValue<'a, T> {
    fn drop(&mut self) {
        // SAFETY: Returning object to pool.
        // - ManuallyDrop::take extracts value (initialized in ObjectPool::get)
        // - pool is valid reference
        // - return_object takes ownership and adds to pool's Vec
        // - No double-drop (ManuallyDrop prevents automatic drop)
        unsafe {
            let obj = ManuallyDrop::take(&mut self.value);
            self.pool.return_object(obj);
        }
    }
}

impl<'a, T: Poolable> AsRef<T> for PooledValue<'a, T> {
    fn as_ref(&self) -> &T {
        &self.value
    }
}

impl<'a, T: Poolable> AsMut<T> for PooledValue<'a, T> {
    fn as_mut(&mut self) -> &mut T {
        &mut self.value
    }
}

// pooled value is Send if T is Send.
// pool ref is Send if ObjectPool is Sync (which it is not).
// But we are sending PooledValue, which implies sending reference to Pool.
// If ObjectPool is not Sync, we cannot send &ObjectPool to another thread.
// So PooledValue cannot be Send if Pool is not Sync.
// ObjectPool is !Sync because of RefCell.
// So PooledValue is !Send.
// THIS IS A REGRESSION if users sent PooledValue across threads.
// But ObjectPool is "Single-threaded object pool".
// Previously, PooledValue was Send if T: Send.
// But it held a raw pointer to Pool.
// If Pool was on thread A, and PooledValue sent to thread B, then dropped on B.
// Thread B calls pool.return_object().
// return_object writes to pool.
// Thread A might be writing to pool too.
// Data race!
// So the PREVIOUS implementation was UNSOUND if PooledValue was Send and Pool !Sync.
// Previous impl had:
// unsafe impl<T: Poolable + Send> Send for PooledValue<T> {}
// This allowed sending PooledValue to another thread.
// If ObjectPool was not Sync (it wasn't, Vec is not Sync if T not Sync, but even if T Sync, Vec is Sync. Wait.
// Vec<T> is Sync if T: Sync.
// ObjectPool fields: objects: Vec<T>.
// If T: Sync, ObjectPool is Sync?
// No, `Cell` / `RefCell` make things !Sync.
// Previous `ObjectPool` had `Vec<T>`. It was Sync if `T: Sync`.
// BUT `get` took `&mut self`.
// So you couldn't access pool from multiple threads simultaneously anyway?
// No, `Sync` means `&Pool` is `Send`.
// If `Pool` is `Sync`, multiple threads can have `&Pool`.
// But `get` takes `&mut self`. So only one thread can call `get` at a time.
// But what about `PooledValue`?
// If `PooledValue` is sent to Thread B.
// Thread B drops it. Calls `pool.return_object`.
// `return_object` takes `&mut self`.
// `PooledValue` held `NonNull<ObjectPool>`.
// `pool.as_mut().return_object(obj)`
// This creates `&mut pool` from pointer.
// If `pool` is currently borrowed mutably on Thread A?
// `PooledValue` was created from `&mut self` on Thread A.
// So `pool` was borrowed mutably.
// But that borrow "ended" (lifetime-wise) for the compiler in the old code.
// If Thread A still uses `pool`?
// It can't if we respect rules. But `NonNull` bypasses rules.
// Basic issue: `PooledValue` allows mutating `Pool` remotely.
// If `Pool` is not thread safe (e.g. `RefCell`), then `PooledValue` must NOT be Send.
// ObjectPool with RefCell is !Sync.
// So &ObjectPool is !Send.
// So PooledValue holding &ObjectPool is !Send.
// Correct. `PooledValue` should not be Send.
// Unless `ObjectPool` is `Sync` (thread safe pool).
// This is the single-threaded `ObjectPool`.
// So it is correct that `PooledValue` is not Send.
// I will remove the unsafe Send impl.

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
        let mut pool = ObjectPool::new(10, || TestObject {
            value: 42,
            resets: 0,
        });

        // Get object
        let mut obj = pool.get().unwrap();
        assert_eq!((*obj).value, 0); // Should be reset
        (*obj).value = 100;

        // Return happens on drop
        drop(obj);

        // Get again - should reuse
        let obj2 = pool.get().unwrap();
        assert_eq!(obj2.resets, 2); // Reset on first get and second get
    }

    #[test]
    fn test_pool_exhaustion() {
        let config = PoolConfig::bounded(2);
        let mut pool = ObjectPool::with_config(config, || TestObject {
            value: 0,
            resets: 0,
        });

        let _obj1 = pool.get().unwrap();
        let _obj2 = pool.get().unwrap();

        // Pool should be exhausted
        assert!(pool.get().is_err());
    }

    #[test]
    fn test_detach() {
        let mut pool = ObjectPool::new(10, || TestObject {
            value: 42,
            resets: 0,
        });

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
            let mut obj = CompressibleObject {
                data: Vec::with_capacity(1000),
                compressed: false,
            };
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
            let mut obj = CompressibleObject {
                data: Vec::with_capacity(1000),
                compressed: false,
            };
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
            assert!(
                stats
                    .compression_attempts
                    .load(core::sync::atomic::Ordering::Relaxed)
                    > 0
            );
            assert!(
                stats
                    .successful_compressions
                    .load(core::sync::atomic::Ordering::Relaxed)
                    > 0
            );
            assert!(
                stats
                    .memory_saved
                    .load(core::sync::atomic::Ordering::Relaxed)
                    > 0
            );
        }
    }
}
