//! Hierarchical object pool implementation
//!
//! # Safety
//!
//! This module implements hierarchical pooling with parent-child borrowing:
//! - `HierarchicalPooledValue` holds raw pointer to pool
//! - `ManuallyDrop` for controlled object lifecycle
//! - Drop returns object to correct pool (local or parent)
//! - `Arc<Mutex>` ensures pool stays alive while values exist
//!
//! ## Safety Contracts
//!
//! - `HierarchicalPooledValue::detach`: `ManuallyDrop::take` + `mem::forget` prevents drop
//! - `HierarchicalPooledValue::drop`: `ManuallyDrop::take` + pool deref + `return_object`
//! - Send implementation: Safe if T: Send (pool pointer not shared)
//! - Pool pointer remains valid (Arc keeps pool alive)

use core::mem::ManuallyDrop;
use core::ops::{Deref, DerefMut};
use parking_lot::Mutex;
use std::sync::{Arc, Weak};

#[cfg(feature = "stats")]
use super::PoolStats;
use super::{ObjectPool, PoolConfig, Poolable};
use crate::error::{MemoryError, MemoryResult};

/// Hierarchical pool supporting parent-child relationships
///
/// Child pools can borrow from parent pools when empty, creating
/// a hierarchy of resource sharing.
///
/// # Example
/// ```
/// use nebula_memory::pool::{HierarchicalPool, HierarchicalPoolExt};
///
/// // Global pool
/// let global_pool = HierarchicalPool::new(1000, || Vec::<u8>::with_capacity(4096));
///
/// // Thread-local pool that borrows from global
/// let local_pool = global_pool.create_child(100);
///
/// // Request-scoped pool that borrows from thread-local
/// let request_pool = local_pool.create_child(10);
/// ```
pub struct HierarchicalPool<T: Poolable> {
    local: ObjectPool<T>,
    parent: Option<Arc<Mutex<HierarchicalPool<T>>>>,
    children: Vec<Weak<Mutex<HierarchicalPool<T>>>>,
    max_borrow: usize,
    borrowed_count: usize,
}

impl<T: Poolable> HierarchicalPool<T> {
    /// Create new root pool
    pub fn new<F>(capacity: usize, factory: F) -> Arc<Mutex<Self>>
    where
        F: Fn() -> T + 'static,
    {
        Arc::new(Mutex::new(Self {
            local: ObjectPool::new(capacity, factory),
            parent: None,
            children: Vec::new(),
            max_borrow: capacity / 2, // Can borrow up to 50% from parent
            borrowed_count: 0,
        }))
    }

    /// Create pool with custom configuration
    pub fn with_config<F>(config: PoolConfig, factory: F) -> Arc<Mutex<Self>>
    where
        F: Fn() -> T + 'static,
    {
        let max_borrow = config.initial_capacity / 2;
        Arc::new(Mutex::new(Self {
            local: ObjectPool::with_config(config, factory),
            parent: None,
            children: Vec::new(),
            max_borrow,
            borrowed_count: 0,
        }))
    }
}

impl<T: Poolable> HierarchicalPool<T> {
    /// Create child pool
    pub fn create_child_static(
        parent: &Arc<Mutex<HierarchicalPool<T>>>,
        capacity: usize,
    ) -> Arc<Mutex<HierarchicalPool<T>>> {
        let parent_clone = parent.clone();

        // Child pools must not pre-warm because they borrow from parent instead
        // of creating objects via factory
        let child_config = super::PoolConfig {
            initial_capacity: capacity,
            pre_warm: false,
            ..Default::default()
        };
        let child = Arc::new(Mutex::new(Self {
            local: ObjectPool::with_config(child_config, || {
                unreachable!(
                    "Child pool factory should not be called directly; objects are borrowed from parent pool"
                )
            }),
            parent: Some(parent_clone),
            children: Vec::new(),
            max_borrow: capacity / 2,
            borrowed_count: 0,
        }));

        // Register child with parent
        parent.lock().children.push(Arc::downgrade(&child));

        child
    }

    /// Get object from pool hierarchy
    ///
    /// Requires `Arc<Mutex<Self>>` so the returned `HierarchicalPooledValue`
    /// can safely return the object on drop from any thread.
    pub fn get(this: &Arc<Mutex<Self>>) -> MemoryResult<HierarchicalPooledValue<T>> {
        let mut guard = this.lock();

        // Try local pool first
        let local_value = guard
            .local
            .get()
            .ok()
            .map(super::object_pool::PooledValue::detach);
        if let Some(detached) = local_value {
            return Ok(HierarchicalPooledValue {
                value: ManuallyDrop::new(detached),
                pool: Arc::clone(this),
                borrowed: false,
            });
        }

        // Try to borrow from parent
        let borrowed_result = if let Some(parent) = &guard.parent {
            if guard.borrowed_count < guard.max_borrow {
                let parent_guard = parent.lock();
                parent_guard
                    .local
                    .get()
                    .ok()
                    .map(super::object_pool::PooledValue::detach)
            } else {
                None
            }
        } else {
            None
        };

        if let Some(value) = borrowed_result {
            guard.borrowed_count += 1;
            return Ok(HierarchicalPooledValue {
                value: ManuallyDrop::new(value),
                pool: Arc::clone(this),
                borrowed: true,
            });
        }

        Err(MemoryError::pool_exhausted("pool", 0))
    }

    /// Return object to pool
    fn return_object(&mut self, obj: T, was_borrowed: bool) {
        if was_borrowed {
            // Return to parent
            if let Some(parent) = &self.parent {
                let parent_guard = parent.lock();
                parent_guard.local.return_object(obj);
                self.borrowed_count = self.borrowed_count.saturating_sub(1);
                return;
            }
        }

        // Return to local pool
        self.local.return_object(obj);
    }

    /// Get statistics for entire hierarchy
    #[cfg(feature = "stats")]
    pub fn hierarchy_stats(&self) -> HierarchyStats {
        // Получаем ссылку на stats для преобразования
        let stats = self.local.stats();
        let local_stats = PoolStatsSnapshot::from(stats);

        let mut total_stats = HierarchyStats {
            levels: vec![local_stats],
            total_objects: 0,
            total_borrowed: self.borrowed_count,
        };

        // Aggregate child stats
        for child_weak in &self.children {
            if let Some(child) = child_weak.upgrade() {
                let child_guard = child.lock();
                let child_stats = child_guard.hierarchy_stats();
                total_stats.levels.extend(child_stats.levels);
                total_stats.total_borrowed += child_stats.total_borrowed;
            }
        }

        total_stats
    }

    /// Clean up dead children
    pub fn cleanup_children(&mut self) {
        self.children.retain(|weak| weak.strong_count() > 0);
    }
}

/// Statistics for pool hierarchy
#[derive(Debug, Clone)]
#[allow(dead_code)] // public API for stats feature consumers
pub struct HierarchyStats {
    pub levels: Vec<PoolStatsSnapshot>,
    pub total_objects: usize,
    pub total_borrowed: usize,
}

/// Pool statistics snapshot
#[derive(Debug, Clone)]
#[allow(dead_code)] // public API for stats feature consumers
pub struct PoolStatsSnapshot {
    pub available: usize,
    pub total_created: usize,
    pub hit_rate: f64,
}

#[cfg(feature = "stats")]
impl From<&PoolStats> for PoolStatsSnapshot {
    fn from(_stats: &PoolStats) -> Self {
        Self {
            available: 0,     // заменить на stats.available() после добавления этого метода
            total_created: 0, // заменить на stats.total_created() после добавления этого метода
            hit_rate: 0.0,    // заменить на stats.hit_rate() после добавления этого метода
        }
    }
}

#[cfg(feature = "stats")]
impl From<PoolStats> for PoolStatsSnapshot {
    fn from(stats: PoolStats) -> Self {
        Self::from(&stats)
    }
}

/// RAII wrapper for hierarchical pooled values
///
/// Uses `Arc<Mutex<HierarchicalPool<T>>>` instead of a raw pointer to ensure
/// thread-safe return on drop. The Mutex is acquired in `Drop::drop` to
/// safely return the object to the pool.
pub struct HierarchicalPooledValue<T: Poolable> {
    value: ManuallyDrop<T>,
    pool: Arc<Mutex<HierarchicalPool<T>>>,
    borrowed: bool,
}

impl<T: Poolable> HierarchicalPooledValue<T> {
    /// Detach value from pool (won't be returned on drop)
    pub fn detach(mut self) -> T {
        // SAFETY: Extracting value from ManuallyDrop.
        // - value is initialized (created in HierarchicalPool::get)
        // - mem::forget prevents Drop::drop from running
        // - No double-free (Drop won't return object to pool)
        let value = unsafe { ManuallyDrop::take(&mut self.value) };
        core::mem::forget(self);
        value
    }

    /// Check if value was borrowed from parent
    pub fn is_borrowed(&self) -> bool {
        self.borrowed
    }
}

impl<T: Poolable> Deref for HierarchicalPooledValue<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<T: Poolable> DerefMut for HierarchicalPooledValue<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.value
    }
}

impl<T: Poolable> Drop for HierarchicalPooledValue<T> {
    fn drop(&mut self) {
        // SAFETY: Extracting value from ManuallyDrop — value is initialized,
        // this is the only take (Drop runs once).
        let obj = unsafe { ManuallyDrop::take(&mut self.value) };
        // Acquire the mutex to safely return the object
        self.pool.lock().return_object(obj, self.borrowed);
    }
}

/// Extension trait for `Arc<Mutex<HierarchicalPool<T>>>`
///
/// This trait provides ergonomic methods that hide the Arc<Mutex<>> complexity
/// from users, making the API cleaner and preventing common lifetime issues.
#[allow(dead_code)] // extension trait for ergonomic pool access
pub trait HierarchicalPoolExt<T: Poolable> {
    /// Create a child pool
    fn create_child(&self, capacity: usize) -> Arc<Mutex<HierarchicalPool<T>>>;

    /// Get object from pool
    fn get(&self) -> MemoryResult<HierarchicalPooledValue<T>>;

    /// Get statistics for entire hierarchy
    #[cfg(feature = "stats")]
    fn hierarchy_stats(&self) -> HierarchyStats;
}

impl<T: Poolable + 'static> HierarchicalPoolExt<T> for Arc<Mutex<HierarchicalPool<T>>> {
    fn create_child(&self, capacity: usize) -> Arc<Mutex<HierarchicalPool<T>>> {
        HierarchicalPool::create_child_static(self, capacity)
    }

    fn get(&self) -> MemoryResult<HierarchicalPooledValue<T>> {
        HierarchicalPool::get(self)
    }

    #[cfg(feature = "stats")]
    fn hierarchy_stats(&self) -> HierarchyStats {
        self.lock().hierarchy_stats()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    struct TestObject {
        value: i32,
    }

    impl Poolable for TestObject {
        fn reset(&mut self) {
            self.value = 0;
        }
    }

    impl Default for TestObject {
        fn default() -> Self {
            Self { value: 42 }
        }
    }

    #[test]
    fn test_hierarchical_borrowing() {
        // Create parent pool
        let parent = HierarchicalPool::new(10, TestObject::default);

        // Get from parent via associated function
        {
            let obj = HierarchicalPool::get(&parent).unwrap();
            assert_eq!((*obj).value, 0); // Reset
            assert!(!obj.is_borrowed());
        }

        // Stats should show activity
        #[cfg(feature = "stats")]
        {
            use super::HierarchicalPoolExt;
            let stats = parent.hierarchy_stats();
            assert_eq!(stats.total_borrowed, 0);
        }
    }
}
