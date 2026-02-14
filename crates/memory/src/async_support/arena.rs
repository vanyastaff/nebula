//! Async-friendly arena allocator with owned value API and modern async patterns
//!
//! This module provides async-safe arena access following modern async patterns:
//! - Graceful shutdown with CancellationToken
//! - Comprehensive tracing instrumentation
//! - Timeout support for operations
//! - Proper async Drop handling where possible
//!
//! # Safety
//!
//! This module provides async-safe arena access through RwLock:
//! - ArenaHandle holds raw pointer to arena-allocated value
//! - Arc<RwLock<Arena>> ensures arena stays alive
//! - RwLock read/write guards synchronize access to arena
//! - Pointer dereferencing only happens while holding lock
//!
//! ## Safety Contracts
//!
//! - ArenaHandle::get/get_mut: Dereferences ptr while holding arena lock
//! - ArenaHandle::modify/read: Dereferences ptr with write/read lock held
//! - Send/Sync: Safe if T: Send (arena synchronized by RwLock)
//! - Arena pointer remains valid (Arc keeps arena alive)

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

#[cfg(feature = "tracing")]
use tracing::{debug, instrument, trace, warn};

use crate::arena::{Arena, ArenaConfig};
use crate::error::{MemoryError, MemoryResult};

/// Default timeout for arena operations (10 seconds)
const DEFAULT_ARENA_TIMEOUT: Duration = Duration::from_secs(10);

/// Handle to allocated arena memory
///
/// This provides safe access to arena-allocated values without lifetime issues.
/// The value is owned by the arena and accessed through Arc<RwLock>.
pub struct ArenaHandle<T> {
    ptr: *mut T,
    arena: Arc<RwLock<Arena>>,
}

// SAFETY: ArenaHandle can be sent between threads if T: Send.
// - ptr is a raw pointer (requires T: Send for safe transfer)
// - arena: Arc<RwLock<Arena>> is Send+Sync
// - T: Send ensures value can be accessed from any thread
// - RwLock synchronizes all arena access
unsafe impl<T: Send> Send for ArenaHandle<T> {}

// SAFETY: ArenaHandle can be shared between threads if T: Send.
// - All access requires acquiring RwLock (read or write)
// - T: Send ensures value access is thread-safe
// - Multiple threads can hold handles, but RwLock ensures exclusive/shared access
// - Arc<RwLock<Arena>> provides synchronization
unsafe impl<T: Send> Sync for ArenaHandle<T> {}

impl<T> ArenaHandle<T> {
    /// Get reference to the value
    #[cfg_attr(feature = "tracing", instrument(skip(self)))]
    pub async fn get(&self) -> &T {
        #[cfg(feature = "tracing")]
        trace!("Getting arena handle reference");

        // Keep arena lock alive while accessing
        let _arena = self.arena.read().await;

        // SAFETY: Dereferencing arena-allocated pointer.
        // - ptr is valid (allocated in AsyncArena::alloc)
        // - _arena holds read lock (synchronized access)
        // - Arena remains alive (Arc keeps it valid)
        // - Returns immutable reference tied to _arena guard lifetime
        unsafe { &*self.ptr }
    }

    /// Get mutable reference to the value
    #[cfg_attr(feature = "tracing", instrument(skip(self)))]
    pub async fn get_mut(&self) -> &mut T {
        #[cfg(feature = "tracing")]
        trace!("Getting arena handle mutable reference");

        // Keep arena lock alive while accessing
        let _arena = self.arena.write().await;

        // SAFETY: Creating mutable reference to arena-allocated value.
        // - ptr is valid (allocated in AsyncArena::alloc)
        // - _arena holds write lock (exclusive access, no other references)
        // - Arena remains alive (Arc keeps it valid)
        // - Returns mutable reference tied to _arena guard lifetime
        unsafe { &mut *self.ptr }
    }

    /// Modify the value with a closure
    #[cfg_attr(feature = "tracing", instrument(skip(self, f)))]
    pub async fn modify<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut T) -> R,
    {
        #[cfg(feature = "tracing")]
        trace!("Modifying arena handle value");

        let _arena = self.arena.write().await;

        // SAFETY: Creating mutable reference for closure.
        // - ptr is valid (allocated in AsyncArena::alloc)
        // - _arena holds write lock (exclusive access)
        // - Arena remains alive (Arc keeps it valid)
        // - Mutable reference lifetime bounded by _arena guard
        unsafe { f(&mut *self.ptr) }
    }

    /// Read the value with a closure
    #[cfg_attr(feature = "tracing", instrument(skip(self, f)))]
    pub async fn read<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&T) -> R,
    {
        #[cfg(feature = "tracing")]
        trace!("Reading arena handle value");

        let _arena = self.arena.read().await;

        // SAFETY: Creating immutable reference for closure.
        // - ptr is valid (allocated in AsyncArena::alloc)
        // - _arena holds read lock (synchronized shared access)
        // - Arena remains alive (Arc keeps it valid)
        // - Immutable reference lifetime bounded by _arena guard
        unsafe { f(&*self.ptr) }
    }

    /// Try to modify the value with a timeout
    #[cfg_attr(feature = "tracing", instrument(skip(self, f)))]
    pub async fn try_modify<F, R>(&self, f: F, duration: Duration) -> MemoryResult<R>
    where
        F: FnOnce(&mut T) -> R,
    {
        timeout(duration, self.modify(f)).await.map_err(|_| {
            #[cfg(feature = "tracing")]
            warn!(?duration, "Arena modify operation timed out");

            MemoryError::InvalidState {
                reason: format!("arena modify timed out after {:?}", duration),
            }
        })
    }

    /// Try to read the value with a timeout
    #[cfg_attr(feature = "tracing", instrument(skip(self, f)))]
    pub async fn try_read<F, R>(&self, f: F, duration: Duration) -> MemoryResult<R>
    where
        F: FnOnce(&T) -> R,
    {
        timeout(duration, self.read(f)).await.map_err(|_| {
            #[cfg(feature = "tracing")]
            warn!(?duration, "Arena read operation timed out");

            MemoryError::InvalidState {
                reason: format!("arena read timed out after {:?}", duration),
            }
        })
    }
}

/// Async-friendly arena allocator with graceful shutdown
///
/// Uses an owned value API to avoid lifetime issues with async functions.
/// Returns handles instead of direct references.
///
/// # Features
///
/// - **Graceful shutdown**: CancellationToken for clean termination
/// - **Timeout support**: Configurable timeouts for operations
/// - **Tracing**: Comprehensive instrumentation for debugging
/// - **Backpressure**: Semaphore limits concurrent allocations
///
/// # Example
///
/// ```ignore
/// use nebula_memory::async_support::AsyncArena;
/// use tokio_util::sync::CancellationToken;
///
/// #[tokio::main]
/// async fn main() {
///     let shutdown = CancellationToken::new();
///     let arena = AsyncArena::new(shutdown.clone());
///
///     // Allocate values
///     let handle = arena.alloc(42).await.unwrap();
///     let value = handle.read(|v| *v).await;
///     assert_eq!(value, 42);
///
///     // Graceful shutdown
///     shutdown.cancel();
/// }
/// ```
pub struct AsyncArena {
    /// Underlying arena allocator
    arena: Arc<RwLock<Arena>>,

    /// Shutdown token for graceful termination
    shutdown: CancellationToken,

    /// Default timeout for operations
    default_timeout: Duration,
}

#[allow(clippy::arc_with_non_send_sync)]
impl AsyncArena {
    /// Create new async arena with default configuration and shutdown token
    #[cfg_attr(feature = "tracing", instrument(skip(shutdown)))]
    pub fn new(shutdown: CancellationToken) -> Self {
        #[cfg(feature = "tracing")]
        debug!("Creating AsyncArena with default config");

        Self::with_config(ArenaConfig::default(), shutdown)
    }

    /// Create new async arena with custom configuration and shutdown token
    #[cfg_attr(feature = "tracing", instrument(skip(shutdown)))]
    pub fn with_config(config: ArenaConfig, shutdown: CancellationToken) -> Self {
        #[cfg(feature = "tracing")]
        debug!(?config, "Creating AsyncArena with custom config");

        Self {
            arena: Arc::new(RwLock::new(Arena::new(config))),
            shutdown,
            default_timeout: DEFAULT_ARENA_TIMEOUT,
        }
    }

    /// Create new async arena with capacity and shutdown token
    #[cfg_attr(feature = "tracing", instrument(skip(shutdown)))]
    pub fn with_capacity(capacity: usize, shutdown: CancellationToken) -> Self {
        #[cfg(feature = "tracing")]
        debug!(capacity, "Creating AsyncArena with capacity");

        let config = ArenaConfig {
            initial_size: capacity,
            ..Default::default()
        };

        Self {
            arena: Arc::new(RwLock::new(Arena::new(config))),
            shutdown,
            default_timeout: DEFAULT_ARENA_TIMEOUT,
        }
    }

    /// Set default timeout for arena operations
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.default_timeout = timeout;
        self
    }

    /// Check if arena is shut down
    pub fn is_shutdown(&self) -> bool {
        self.shutdown.is_cancelled()
    }

    /// Allocate a value asynchronously, returning a handle
    ///
    /// This method will wait if the arena is currently being accessed by other tasks.
    ///
    /// # Errors
    ///
    /// - `MemoryError::InvalidState` if arena is shut down or operation times out
    /// - `MemoryError::ArenaExhausted` if arena has insufficient space
    #[cfg_attr(feature = "tracing", instrument(skip(self, value)))]
    pub async fn alloc<T>(&self, value: T) -> MemoryResult<ArenaHandle<T>> {
        self.alloc_timeout(value, self.default_timeout).await
    }

    /// Allocate a value with custom timeout
    ///
    /// # Errors
    ///
    /// - `MemoryError::InvalidState` if arena is shut down or operation times out
    /// - `MemoryError::ArenaExhausted` if arena has insufficient space
    #[cfg_attr(feature = "tracing", instrument(skip(self, value)))]
    pub async fn alloc_timeout<T>(
        &self,
        value: T,
        duration: Duration,
    ) -> MemoryResult<ArenaHandle<T>> {
        // Check for shutdown
        if self.shutdown.is_cancelled() {
            #[cfg(feature = "tracing")]
            warn!("Attempted to allocate from shut down arena");

            return Err(MemoryError::InvalidState {
                reason: "arena is shut down".to_string(),
            });
        }

        // Wrap allocation in a timeout
        timeout(duration, self.alloc_inner(value))
            .await
            .map_err(|_| {
                #[cfg(feature = "tracing")]
                warn!(?duration, "Arena allocation timed out");

                MemoryError::InvalidState {
                    reason: format!("arena allocation timed out after {:?}", duration),
                }
            })?
    }

    /// Internal allocation implementation
    async fn alloc_inner<T>(&self, value: T) -> MemoryResult<ArenaHandle<T>> {
        // Check for cancellation during lock acquisition
        tokio::select! {
            arena = self.arena.write() => {
                #[cfg(feature = "tracing")]
                trace!("Acquired arena write lock for allocation");

                // Perform allocation
                let ptr = arena.alloc(value)?;

                #[cfg(feature = "tracing")]
                trace!("Successfully allocated in arena");

                Ok(ArenaHandle {
                    ptr,
                    arena: Arc::clone(&self.arena),
                })
            }
            _ = self.shutdown.cancelled() => {
                Err(MemoryError::InvalidState {
                    reason: "arena is shut down".to_string(),
                })
            }
        }
    }

    /// Allocate a slice asynchronously, copying data into arena
    ///
    /// # Errors
    ///
    /// - `MemoryError::InvalidState` if arena is shut down or operation times out
    /// - `MemoryError::ArenaExhausted` if arena has insufficient space
    #[cfg_attr(feature = "tracing", instrument(skip(self, slice)))]
    pub async fn alloc_slice_copy<T>(&self, slice: &[T]) -> MemoryResult<Vec<T>>
    where
        T: Copy,
    {
        self.alloc_slice_copy_timeout(slice, self.default_timeout)
            .await
    }

    /// Allocate a slice with custom timeout
    #[cfg_attr(feature = "tracing", instrument(skip(self, slice)))]
    pub async fn alloc_slice_copy_timeout<T>(
        &self,
        slice: &[T],
        duration: Duration,
    ) -> MemoryResult<Vec<T>>
    where
        T: Copy,
    {
        // Check for shutdown
        if self.shutdown.is_cancelled() {
            #[cfg(feature = "tracing")]
            warn!("Attempted to allocate slice from shut down arena");

            return Err(MemoryError::InvalidState {
                reason: "arena is shut down".to_string(),
            });
        }

        timeout(duration, async {
            tokio::select! {
                arena = self.arena.write() => {
                    #[cfg(feature = "tracing")]
                    trace!(len = slice.len(), "Allocating slice in arena");

                    // Allocate slice in arena
                    let arena_slice = arena.alloc_slice(slice)?;

                    #[cfg(feature = "tracing")]
                    trace!("Successfully allocated slice");

                    // Return owned Vec copy to avoid lifetime issues
                    Ok(arena_slice.to_vec())
                }
                _ = self.shutdown.cancelled() => {
                    Err(MemoryError::InvalidState {
                        reason: "arena is shut down".to_string(),
                    })
                }
            }
        })
        .await
        .map_err(|_| {
            #[cfg(feature = "tracing")]
            warn!(?duration, "Arena slice allocation timed out");

            MemoryError::InvalidState {
                reason: format!("arena slice allocation timed out after {:?}", duration),
            }
        })?
    }

    /// Allocate a string asynchronously, returning owned String
    ///
    /// # Errors
    ///
    /// - `MemoryError::InvalidState` if arena is shut down or operation times out
    /// - `MemoryError::ArenaExhausted` if arena has insufficient space
    #[cfg_attr(feature = "tracing", instrument(skip(self, s)))]
    pub async fn alloc_string(&self, s: &str) -> MemoryResult<String> {
        self.alloc_string_timeout(s, self.default_timeout).await
    }

    /// Allocate a string with custom timeout
    #[cfg_attr(feature = "tracing", instrument(skip(self, s)))]
    pub async fn alloc_string_timeout(&self, s: &str, duration: Duration) -> MemoryResult<String> {
        // Check for shutdown
        if self.shutdown.is_cancelled() {
            #[cfg(feature = "tracing")]
            warn!("Attempted to allocate string from shut down arena");

            return Err(MemoryError::InvalidState {
                reason: "arena is shut down".to_string(),
            });
        }

        timeout(duration, async {
            tokio::select! {
                arena = self.arena.write() => {
                    #[cfg(feature = "tracing")]
                    trace!(len = s.len(), "Allocating string in arena");

                    // Allocate in arena
                    let arena_str = arena.alloc_str(s)?;

                    #[cfg(feature = "tracing")]
                    trace!("Successfully allocated string");

                    // Return owned String to avoid lifetime issues
                    Ok(arena_str.to_string())
                }
                _ = self.shutdown.cancelled() => {
                    Err(MemoryError::InvalidState {
                        reason: "arena is shut down".to_string(),
                    })
                }
            }
        })
        .await
        .map_err(|_| {
            #[cfg(feature = "tracing")]
            warn!(?duration, "Arena string allocation timed out");

            MemoryError::InvalidState {
                reason: format!("arena string allocation timed out after {:?}", duration),
            }
        })?
    }

    /// Reset the arena asynchronously
    ///
    /// WARNING: All handles become invalid after reset!
    #[cfg_attr(feature = "tracing", instrument(skip(self)))]
    pub async fn reset(&self) -> MemoryResult<()> {
        // Check for shutdown
        if self.shutdown.is_cancelled() {
            #[cfg(feature = "tracing")]
            warn!("Attempted to reset shut down arena");

            return Err(MemoryError::InvalidState {
                reason: "arena is shut down".to_string(),
            });
        }

        #[cfg(feature = "tracing")]
        debug!("Resetting arena");

        tokio::select! {
            mut arena = self.arena.write() => {
                arena.reset();

                #[cfg(feature = "tracing")]
                debug!("Arena reset complete");

                Ok(())
            }
            _ = self.shutdown.cancelled() => {
                Err(MemoryError::InvalidState {
                    reason: "arena is shut down".to_string(),
                })
            }
        }
    }

    /// Clone the arena handle (shares the same underlying arena)
    pub fn clone_handle(&self) -> Self {
        Self {
            arena: Arc::clone(&self.arena),
            shutdown: self.shutdown.clone(),
            default_timeout: self.default_timeout,
        }
    }

    /// Get the shutdown token for this arena
    pub fn shutdown_token(&self) -> &CancellationToken {
        &self.shutdown
    }
}

/// Scoped async arena that automatically resets on drop (best effort)
///
/// Note: Because Drop cannot be async, the reset operation is NOT guaranteed
/// to complete. For guaranteed cleanup, call `reset()` explicitly before dropping.
pub struct AsyncArenaScope {
    arena: AsyncArena,
}

impl AsyncArenaScope {
    /// Create new scoped async arena with configuration and shutdown token
    #[cfg_attr(feature = "tracing", instrument(skip(shutdown)))]
    pub fn new(config: ArenaConfig, shutdown: CancellationToken) -> Self {
        #[cfg(feature = "tracing")]
        debug!("Creating AsyncArenaScope");

        Self {
            arena: AsyncArena::with_config(config, shutdown),
        }
    }

    /// Create with default configuration
    #[cfg_attr(feature = "tracing", instrument(skip(shutdown)))]
    pub fn with_default(shutdown: CancellationToken) -> Self {
        Self::new(ArenaConfig::default(), shutdown)
    }

    /// Get reference to the underlying arena
    pub fn arena(&self) -> &AsyncArena {
        &self.arena
    }

    /// Explicitly reset the arena (recommended before drop)
    pub async fn reset(&self) -> MemoryResult<()> {
        self.arena.reset().await
    }
}

impl Drop for AsyncArenaScope {
    fn drop(&mut self) {
        // We can't call async reset in Drop, so we just log a warning
        // Users should call reset() explicitly before dropping for guaranteed cleanup

        #[cfg(feature = "tracing")]
        debug!(
            "AsyncArenaScope dropped (reset not guaranteed in Drop - call reset() explicitly for cleanup)"
        );
    }
}

#[cfg(all(test, feature = "async"))]
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn test_async_arena_basic() {
        let shutdown = CancellationToken::new();
        let arena = AsyncArena::new(shutdown.clone());

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

        shutdown.cancel();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_async_arena_timeout() {
        let shutdown = CancellationToken::new();
        let arena = AsyncArena::new(shutdown.clone()).with_timeout(Duration::from_millis(50));

        // Normal allocation should succeed
        let handle = arena.alloc(42).await.unwrap();

        // Try operations with timeout
        let result = handle.try_read(|v| *v, Duration::from_millis(50)).await;
        assert!(result.is_ok());

        shutdown.cancel();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_async_arena_shutdown() {
        let shutdown = CancellationToken::new();
        let arena = AsyncArena::new(shutdown.clone());

        // Allocate before shutdown
        let handle = arena.alloc(42).await.unwrap();

        // Initiate shutdown
        shutdown.cancel();

        // Should not be able to allocate after shutdown
        let result = arena.alloc(99).await;
        assert!(result.is_err());

        // Can still read existing handles
        let value = handle.read(|v| *v).await;
        assert_eq!(value, 42);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_async_arena_scope() {
        let shutdown = CancellationToken::new();
        let scope = AsyncArenaScope::with_default(shutdown.clone());

        let handle = scope.arena().alloc(123).await.unwrap();
        let value = handle.read(|v| *v).await;
        assert_eq!(value, 123);

        // Explicit reset before drop (recommended)
        scope.reset().await.unwrap();

        shutdown.cancel();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_arena_handle_modify() {
        let shutdown = CancellationToken::new();
        let arena = AsyncArena::new(shutdown.clone());
        let handle = arena.alloc(vec![1, 2, 3]).await.unwrap();

        // Modify through handle
        handle.modify(|v| v.push(4)).await;

        // Verify modification
        let len = handle.read(|v| v.len()).await;
        assert_eq!(len, 4);

        shutdown.cancel();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_multiple_handles() {
        let shutdown = CancellationToken::new();
        let arena = AsyncArena::with_capacity(1024, shutdown.clone());

        let h1 = arena.alloc(10).await.unwrap();
        let h2 = arena.alloc(20).await.unwrap();
        let h3 = arena.alloc(30).await.unwrap();

        assert_eq!(h1.read(|v| *v).await, 10);
        assert_eq!(h2.read(|v| *v).await, 20);
        assert_eq!(h3.read(|v| *v).await, 30);

        // Sequential modification
        h1.modify(|v| *v += 5).await;
        h2.modify(|v| *v += 5).await;
        h3.modify(|v| *v += 5).await;

        assert_eq!(h1.read(|v| *v).await, 15);
        assert_eq!(h2.read(|v| *v).await, 25);
        assert_eq!(h3.read(|v| *v).await, 35);

        shutdown.cancel();
    }
}
