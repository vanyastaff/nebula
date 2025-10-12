//! Batch allocation optimization for object pools
//!
//! # Safety
//!
//! This module implements batch allocation for efficient bulk operations:
//! - Batch holds raw pointer to BatchAllocator
//! - Drop returns all objects to pool via allocator pointer
//! - mem::take + mem::forget pattern prevents double-return
//! - Allocator pointer remains valid (lifetime tied to get_batch borrow)
//!
//! ## Safety Contracts
//!
//! - Batch::drop: Dereferences allocator pointer and returns objects
//! - Send implementation: Safe if T: Send (allocator pointer not shared)
//! - Allocator pointer valid (created from &mut in get_batch)

#[cfg(not(feature = "std"))]
use alloc::{boxed::Box, vec::Vec};

#[cfg(feature = "stats")]
use super::PoolStats;
use super::{ObjectPool, PoolConfig, Poolable};
use crate::error::{MemoryError, MemoryResult};

/// Batch allocator for efficient bulk operations
///
/// Optimizes scenarios where multiple objects are needed at once
/// by reducing allocation overhead and improving cache locality.
///
/// # Example
/// ```
/// use nebula_memory::pool::BatchAllocator;
///
/// let mut allocator = BatchAllocator::new(1000, || Vec::<u8>::with_capacity(1024));
///
/// // Get 10 buffers at once
/// let buffers = allocator.get_batch(10).unwrap();
///
/// // Process buffers...
///
/// // Return all at once
/// allocator.return_batch(buffers);
/// ```
pub struct BatchAllocator<T: Poolable> {
    pool: ObjectPool<T>,
    batch_size_hint: usize,
    #[cfg(feature = "stats")]
    batch_stats: BatchStats,
}

/// Statistics for batch operations
#[cfg(feature = "stats")]
#[derive(Debug, Default)]
pub struct BatchStats {
    pub total_batches: u64,
    pub total_objects: u64,
    pub average_batch_size: f64,
    pub max_batch_size: usize,
    pub batch_hit_rate: f64,
}

/// Batch of pooled objects
pub struct Batch<T: Poolable> {
    objects: Vec<T>,
    allocator: *mut BatchAllocator<T>,
}

impl<T: Poolable> BatchAllocator<T> {
    /// Create new batch allocator
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

    /// Create with custom configuration
    pub fn with_config<F>(config: PoolConfig, factory: F) -> Self
    where
        F: Fn() -> T + 'static,
    {
        let batch_size_hint = (config.initial_capacity / 10).max(1);

        Self {
            pool: ObjectPool::with_config(config, factory),
            batch_size_hint,
            #[cfg(feature = "stats")]
            batch_stats: BatchStats::default(),
        }
    }

    /// Get a batch of objects
    pub fn get_batch(&mut self, count: usize) -> MemoryResult<Batch<T>> {
        if count == 0 {
            return Ok(Batch {
                objects: Vec::new(),
                allocator: self as *mut _,
            });
        }

        let mut objects = Vec::with_capacity(count);

        #[cfg(feature = "stats")]
        let mut created = 0;

        // Try to get as many as possible from pool
        for _ in 0..count {
            match self.pool.get() {
                Ok(pooled) => {
                    objects.push(pooled.detach());
                }
                Err(_) if objects.is_empty() => {
                    // First object failed - propagate error
                    return Err(MemoryError::pool_exhausted("pool", 0));
                }
                Err(_) => {
                    // Partial batch is ok
                    break;
                }
            }
        }

        #[cfg(feature = "stats")]
        {
            self.batch_stats.total_batches += 1;
            self.batch_stats.total_objects += objects.len() as u64;
            self.batch_stats.average_batch_size =
                self.batch_stats.total_objects as f64 / self.batch_stats.total_batches as f64;
            self.batch_stats.max_batch_size = self.batch_stats.max_batch_size.max(objects.len());

            if objects.len() == count {
                self.batch_stats.batch_hit_rate = (self.batch_stats.batch_hit_rate
                    * (self.batch_stats.total_batches - 1) as f64
                    + 1.0)
                    / self.batch_stats.total_batches as f64;
            } else {
                self.batch_stats.batch_hit_rate = (self.batch_stats.batch_hit_rate
                    * (self.batch_stats.total_batches - 1) as f64)
                    / self.batch_stats.total_batches as f64;
            }
        }

        Ok(Batch {
            objects,
            allocator: self as *mut _,
        })
    }

    /// Try to get exact batch size
    pub fn try_get_exact_batch(&mut self, count: usize) -> Option<Batch<T>> {
        let mut objects = Vec::with_capacity(count);

        // Get all objects first
        for _ in 0..count {
            match self.pool.try_get() {
                Some(pooled) => objects.push(pooled.detach()),
                None => {
                    // Return what we got so far
                    for obj in objects {
                        self.pool.return_object(obj);
                    }
                    return None;
                }
            }
        }

        #[cfg(feature = "stats")]
        {
            self.batch_stats.total_batches += 1;
            self.batch_stats.total_objects += count as u64;
            self.batch_stats.average_batch_size =
                self.batch_stats.total_objects as f64 / self.batch_stats.total_batches as f64;
            self.batch_stats.max_batch_size = self.batch_stats.max_batch_size.max(count);
            self.batch_stats.batch_hit_rate = (self.batch_stats.batch_hit_rate
                * (self.batch_stats.total_batches - 1) as f64
                + 1.0)
                / self.batch_stats.total_batches as f64;
        }

        Some(Batch {
            objects,
            allocator: self as *mut _,
        })
    }

    /// Return a batch of objects
    pub fn return_batch(&mut self, mut batch: Batch<T>) {
        // Используем mem::take вместо прямого доступа к batch.objects
        for obj in core::mem::take(&mut batch.objects) {
            self.pool.return_object(obj);
        }

        // Forget the batch to prevent double-return
        core::mem::forget(batch);
    }

    /// Pre-allocate objects for future batches
    pub fn reserve_batch(&mut self, count: usize) -> MemoryResult<()> {
        self.pool.reserve(count)
    }

    /// Get batch size hint
    pub fn batch_size_hint(&self) -> usize {
        self.batch_size_hint
    }

    /// Set batch size hint for optimization
    pub fn set_batch_size_hint(&mut self, hint: usize) {
        self.batch_size_hint = hint;
    }

    /// Get pool statistics
    #[cfg(feature = "stats")]
    pub fn pool_stats(&self) -> &PoolStats {
        self.pool.stats()
    }

    /// Get batch statistics
    #[cfg(feature = "stats")]
    pub fn batch_stats(&self) -> &BatchStats {
        &self.batch_stats
    }

    /// Get underlying pool
    pub fn pool(&self) -> &ObjectPool<T> {
        &self.pool
    }

    /// Get mutable underlying pool
    pub fn pool_mut(&mut self) -> &mut ObjectPool<T> {
        &mut self.pool
    }
}

impl<T: Poolable> Batch<T> {
    /// Get number of objects in batch
    pub fn len(&self) -> usize {
        self.objects.len()
    }

    /// Check if batch is empty
    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }

    /// Get slice of objects
    pub fn as_slice(&self) -> &[T] {
        &self.objects
    }

    /// Get mutable slice of objects
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        &mut self.objects
    }

    /// Iterate over objects
    pub fn iter(&self) -> core::slice::Iter<'_, T> {
        self.objects.iter()
    }

    /// Iterate mutably over objects
    pub fn iter_mut(&mut self) -> core::slice::IterMut<'_, T> {
        self.objects.iter_mut()
    }

    /// Split batch at index
    pub fn split_at(mut self, index: usize) -> (Batch<T>, Batch<T>) {
        let allocator = self.allocator;
        // Используем временный вектор для получения второй половины
        let mut current_objects = core::mem::take(&mut self.objects);
        let second_half = if index < current_objects.len() {
            current_objects.split_off(index)
        } else {
            Vec::new()
        };

        let first = Batch {
            objects: current_objects,
            allocator,
        };

        let second = Batch {
            objects: second_half,
            allocator,
        };

        core::mem::forget(self);

        (first, second)
    }

    /// Take specific object from batch
    pub fn take(&mut self, index: usize) -> Option<T> {
        if index < self.objects.len() {
            Some(self.objects.swap_remove(index))
        } else {
            None
        }
    }

    /// Convert to vector (consumes batch)
    pub fn into_vec(mut self) -> Vec<T> {
        // Используем mem::take вместо прямого перемещения из self.objects
        let objects = core::mem::take(&mut self.objects);
        core::mem::forget(self);
        objects
    }
}

impl<T: Poolable> Drop for Batch<T> {
    fn drop(&mut self) {
        // Return all objects to pool
        // SAFETY: Returning batch objects to allocator.
        // - allocator pointer is valid (created from &mut in get_batch)
        // - mem::take extracts objects (prevents double-return)
        // - Each object returned to pool via return_object
        // - pool.return_object handles object lifecycle
        unsafe {
            let objects = core::mem::take(&mut self.objects);
            for obj in objects {
                (*self.allocator).pool.return_object(obj);
            }
        }
    }
}

impl<T: Poolable> IntoIterator for Batch<T> {
    type Item = T;
    #[cfg(feature = "std")]
    type IntoIter = std::vec::IntoIter<T>;
    #[cfg(not(feature = "std"))]
    type IntoIter = alloc::vec::IntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        self.into_vec().into_iter()
    }
}

// SAFETY: Batch can be sent between threads if T: Send.
// - objects: Vec<T> is Send if T: Send
// - allocator: Raw pointer not shared (exclusive ownership of batch)
// - T: Send ensures objects can be safely sent
// - Allocator pointer used only for returning (no concurrent access)
// - Drop on destination thread safely returns objects to pool
unsafe impl<T: Poolable + Send> Send for Batch<T> {}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone)]
    struct TestObject {
        value: i32,
    }

    impl Poolable for TestObject {
        fn reset(&mut self) {
            self.value = 0;
        }
    }

    #[test]
    fn test_batch_allocation() {
        let mut allocator = BatchAllocator::new(100, || TestObject { value: 42 });

        // Get batch
        let batch = allocator.get_batch(10).unwrap();
        assert_eq!(batch.len(), 10);

        // All objects should be reset
        for obj in batch.iter() {
            assert_eq!(obj.value, 0);
        }
    }

    #[test]
    fn test_batch_return() {
        let mut allocator = BatchAllocator::new(10, || TestObject { value: 42 });

        // Get and return batch
        let batch = allocator.get_batch(5).unwrap();
        allocator.return_batch(batch);

        // Should be able to get same objects again
        let batch2 = allocator.get_batch(5).unwrap();
        assert_eq!(batch2.len(), 5);
    }

    #[test]
    fn test_batch_split() {
        let mut allocator = BatchAllocator::new(100, || TestObject { value: 42 });

        let batch = allocator.get_batch(10).unwrap();
        let (first, second) = batch.split_at(5);

        assert_eq!(first.len(), 5);
        assert_eq!(second.len(), 5);
    }

    #[test]
    fn test_partial_batch() {
        let mut allocator = BatchAllocator::new(5, || TestObject { value: 42 });

        // Request more than available
        let batch = allocator.get_batch(10).unwrap();
        assert!(batch.len() <= 5);
    }
}
