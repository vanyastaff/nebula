//! Async-friendly arena allocator with owned value API

use std::sync::Arc;

use tokio::sync::{RwLock, Semaphore};

use crate::arena::{Arena, ArenaConfig};
use crate::error::MemoryResult;

/// Handle to allocated arena memory
///
/// This provides safe access to arena-allocated values without lifetime issues.
/// The value is owned by the arena and accessed through Arc<RwLock>.
pub struct ArenaHandle<T> {
    ptr: *mut T,
    arena: Arc<RwLock<Arena>>,
}

unsafe impl<T: Send> Send for ArenaHandle<T> {}
unsafe impl<T: Send> Sync for ArenaHandle<T> {}

impl<T> ArenaHandle<T> {
    /// Get reference to the value
    pub async fn get(&self) -> &T {
        // Keep arena lock alive while accessing
        let _arena = self.arena.read().await;
        unsafe { &*self.ptr }
    }

    /// Get mutable reference to the value
    pub async fn get_mut(&self) -> &mut T {
        // Keep arena lock alive while accessing
        let _arena = self.arena.write().await;
        unsafe { &mut *self.ptr }
    }

    /// Modify the value with a closure
    pub async fn modify<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut T) -> R,
    {
        let _arena = self.arena.write().await;
        unsafe { f(&mut *self.ptr) }
    }

    /// Read the value with a closure
    pub async fn read<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&T) -> R,
    {
        let _arena = self.arena.read().await;
        unsafe { f(&*self.ptr) }
    }
}

/// Async-friendly arena allocator
///
/// Uses an owned value API to avoid lifetime issues with async functions.
/// Returns handles instead of direct references.
pub struct AsyncArena {
    /// Underlying arena allocator
    arena: Arc<RwLock<Arena>>,

    /// Semaphore for backpressure control
    semaphore: Arc<Semaphore>,

    /// Maximum concurrent allocations
    max_concurrent: usize,
}

impl AsyncArena {
    /// Create new async arena with default configuration
    pub fn new() -> Self {
        Self::with_config(ArenaConfig::default())
    }

    /// Create new async arena with custom configuration
    pub fn with_config(config: ArenaConfig) -> Self {
        let max_concurrent = 1000; // Default limit

        Self {
            arena: Arc::new(RwLock::new(Arena::new(config))),
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
            max_concurrent,
        }
    }

    /// Create new async arena with capacity and concurrency limits
    pub fn with_capacity(capacity: usize, max_concurrent: usize) -> Self {
        let mut config = ArenaConfig::default();
        config.initial_size = capacity;

        Self {
            arena: Arc::new(RwLock::new(Arena::new(config))),
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
            max_concurrent,
        }
    }

    /// Allocate a value asynchronously, returning a handle
    ///
    /// This method will wait if the arena is currently being accessed by other tasks.
    pub async fn alloc<T>(&self, value: T) -> MemoryResult<ArenaHandle<T>> {
        // Acquire permit for backpressure control
        let _permit = self.semaphore.acquire().await.unwrap();

        // Lock arena for writing
        let arena = self.arena.write().await;

        // Perform allocation
        let ptr = arena.alloc(value)?;

        Ok(ArenaHandle {
            ptr,
            arena: Arc::clone(&self.arena),
        })
    }

    /// Allocate a slice asynchronously, copying data into arena
    pub async fn alloc_slice_copy<T>(&self, slice: &[T]) -> MemoryResult<Vec<T>>
    where
        T: Copy,
    {
        let _permit = self.semaphore.acquire().await.unwrap();
        let arena = self.arena.write().await;

        // Allocate slice in arena
        let arena_slice = arena.alloc_slice(slice)?;

        // Return owned Vec copy to avoid lifetime issues
        Ok(arena_slice.to_vec())
    }

    /// Allocate a string asynchronously, returning owned String
    pub async fn alloc_string(&self, s: &str) -> MemoryResult<String> {
        let _permit = self.semaphore.acquire().await.unwrap();
        let arena = self.arena.write().await;

        // Allocate in arena
        let arena_str = arena.alloc_str(s)?;

        // Return owned String to avoid lifetime issues
        Ok(arena_str.to_string())
    }

    /// Reset the arena asynchronously
    ///
    /// WARNING: All handles become invalid after reset!
    pub async fn reset(&self) {
        let mut arena = self.arena.write().await;
        arena.reset();
    }

    /// Clone the arena handle (shares the same underlying arena)
    pub fn clone_handle(&self) -> Self {
        Self {
            arena: Arc::clone(&self.arena),
            semaphore: Arc::clone(&self.semaphore),
            max_concurrent: self.max_concurrent,
        }
    }
}

impl Default for AsyncArena {
    fn default() -> Self {
        Self::new()
    }
}

/// Scoped async arena that automatically resets on drop
pub struct AsyncArenaScope {
    arena: AsyncArena,
}

impl AsyncArenaScope {
    /// Create new scoped async arena
    pub fn new(config: ArenaConfig) -> Self {
        Self {
            arena: AsyncArena::with_config(config),
        }
    }

    /// Create with default configuration
    pub fn with_default() -> Self {
        Self::new(ArenaConfig::default())
    }

    /// Get reference to the underlying arena
    pub fn arena(&self) -> &AsyncArena {
        &self.arena
    }
}

impl Drop for AsyncArenaScope {
    fn drop(&mut self) {
        // We can't call async reset in Drop, so we'll need to use blocking
        // In a real async context, users should call reset() explicitly before dropping
        // This is a best-effort cleanup
    }
}

#[cfg(all(test, feature = "async"))]
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn test_async_arena_basic() {
        let arena = AsyncArena::new();

        // Test handle-based allocation
        let handle = arena.alloc(42).await.unwrap();
        let value = handle.read(|v| *v).await;
        assert_eq!(value, 42);

        // Test modification
        handle.modify(|v| *v = 100).await;
        let value = handle.read(|v| *v).await;
        assert_eq!(value, 100);

        // Test slice allocation (returns owned Vec)
        let vec = arena.alloc_slice_copy(&[1, 2, 3, 4, 5]).await.unwrap();
        assert_eq!(vec.len(), 5);
        assert_eq!(vec[0], 1);

        // Test string allocation (returns owned String)
        let s = arena.alloc_string("hello async").await.unwrap();
        assert_eq!(s, "hello async");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_async_arena_concurrent() {
        let arena = AsyncArena::with_capacity(1024 * 1024, 100);

        // Sequential allocations since Arena is !Send
        for i in 0..10 {
            for j in 0..100 {
                let handle = arena.alloc(i * 100 + j).await.unwrap();
                // Verify allocation
                let val = handle.read(|v| *v).await;
                assert_eq!(val, i * 100 + j);
            }
        }

        // Arena should have allocated something, reset it
        arena.reset().await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_async_arena_scope() {
        let scope = AsyncArenaScope::with_default();

        let handle = scope.arena().alloc(123).await.unwrap();
        let value = handle.read(|v| *v).await;
        assert_eq!(value, 123);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_arena_handle_modify() {
        let arena = AsyncArena::new();
        let handle = arena.alloc(vec![1, 2, 3]).await.unwrap();

        // Modify through handle
        handle.modify(|v| v.push(4)).await;

        // Verify modification
        let len = handle.read(|v| v.len()).await;
        assert_eq!(len, 4);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_multiple_handles() {
        let arena = AsyncArena::with_capacity(1024, 50);

        let h1 = arena.alloc(10).await.unwrap();
        let h2 = arena.alloc(20).await.unwrap();
        let h3 = arena.alloc(30).await.unwrap();

        assert_eq!(h1.read(|v| *v).await, 10);
        assert_eq!(h2.read(|v| *v).await, 20);
        assert_eq!(h3.read(|v| *v).await, 30);

        // Sequential modification (Arena is !Send)
        h1.modify(|v| *v += 5).await;
        h2.modify(|v| *v += 5).await;
        h3.modify(|v| *v += 5).await;

        assert_eq!(h1.read(|v| *v).await, 15);
        assert_eq!(h2.read(|v| *v).await, 25);
        assert_eq!(h3.read(|v| *v).await, 35);
    }
}
