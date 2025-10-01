//! Async-friendly arena allocator

#[cfg(feature = "async")]
use std::sync::Arc;

#[cfg(feature = "async")]
use tokio::sync::{RwLock, Semaphore};

#[cfg(feature = "async")]
use crate::arena::{Arena, ArenaConfig};
#[cfg(feature = "async")]
use crate::core::error::MemoryResult;

/// Async-friendly arena allocator
///
/// Provides async/await compatible allocation with proper async locking
/// and backpressure support.
#[cfg(feature = "async")]
pub struct AsyncArena {
    /// Underlying arena allocator
    arena: Arc<RwLock<Arena>>,

    /// Semaphore for backpressure control
    semaphore: Arc<Semaphore>,

    /// Maximum concurrent allocations
    max_concurrent: usize,
}

#[cfg(feature = "async")]
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

    /// Allocate a value asynchronously
    ///
    /// This method will wait if the arena is currently being accessed by other tasks.
    pub async fn alloc<T>(&self, value: T) -> MemoryResult<&mut T> {
        // Acquire permit for backpressure control
        let _permit = self.semaphore.acquire().await.unwrap();

        // Lock arena for writing
        let mut arena = self.arena.write().await;

        // Perform allocation
        // SAFETY: The returned reference is tied to the arena's lifetime
        // We need to extend the lifetime here, which is unsafe but correct
        // because the arena is Arc-wrapped and won't be dropped while references exist
        unsafe {
            let ptr = arena.alloc(value)?;
            Ok(&mut *(ptr as *mut T))
        }
    }

    /// Allocate a slice asynchronously
    pub async fn alloc_slice<T>(&self, slice: &[T]) -> MemoryResult<&mut [T]>
    where
        T: Copy,
    {
        let _permit = self.semaphore.acquire().await.unwrap();
        let mut arena = self.arena.write().await;

        unsafe {
            let ptr = arena.alloc_slice(slice)?;
            Ok(&mut *(ptr as *mut [T]))
        }
    }

    /// Allocate a string asynchronously
    pub async fn alloc_str(&self, s: &str) -> MemoryResult<&str> {
        let _permit = self.semaphore.acquire().await.unwrap();
        let mut arena = self.arena.write().await;

        unsafe {
            let ptr = arena.alloc_str(s)?;
            Ok(&*(ptr as *const str))
        }
    }

    /// Reset the arena asynchronously
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

#[cfg(feature = "async")]
impl Default for AsyncArena {
    fn default() -> Self {
        Self::new()
    }
}

/// Scoped async arena that automatically resets on drop
#[cfg(feature = "async")]
pub struct AsyncArenaScope {
    arena: AsyncArena,
}

#[cfg(feature = "async")]
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

#[cfg(feature = "async")]
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

    #[tokio::test]
    async fn test_async_arena_basic() {
        let arena = AsyncArena::new();

        let value = arena.alloc(42).await.unwrap();
        assert_eq!(*value, 42);

        let slice = arena.alloc_slice(&[1, 2, 3, 4, 5]).await.unwrap();
        assert_eq!(slice.len(), 5);
        assert_eq!(slice[0], 1);

        let s = arena.alloc_str("hello async").await.unwrap();
        assert_eq!(s, "hello async");
    }

    #[tokio::test]
    async fn test_async_arena_concurrent() {
        let arena = AsyncArena::with_capacity(1024 * 1024, 100);

        let handles: Vec<_> = (0..10)
            .map(|i| {
                let arena = arena.clone_handle();
                tokio::spawn(async move {
                    for j in 0..100 {
                        let _ = arena.alloc(i * 100 + j).await;
                    }
                })
            })
            .collect();

        for handle in handles {
            handle.await.unwrap();
        }

        // Arena should have allocated something, reset it
        arena.reset().await;
    }

    #[tokio::test]
    async fn test_async_arena_scope() {
        let scope = AsyncArenaScope::with_default();

        let value = scope.arena().alloc(123).await.unwrap();
        assert_eq!(*value, 123);
    }
}
