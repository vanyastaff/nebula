//! Cross-thread arena implementation that can be safely moved between threads
//!
//! # Safety
//!
//! This module provides thread-safe arena access through explicit locking:
//! - CrossThreadArena uses Arc<Mutex<Arena>> for exclusive access across threads
//! - CrossThreadArenaRef provides synchronized access to arena-allocated values
//! - UnsafeCell<*mut T> requires external synchronization via Mutex guard
//! - Send/Sync implementations require T: Send for safe cross-thread transfer
//!
//! ## Safety Contracts
//!
//! - Mutex ensures exclusive access (only one thread accesses arena at a time)
//! - CrossThreadArenaRef::with/with_mut lock arena before pointer dereferencing
//! - Arena lifetime tied to Arc (values valid while any reference exists)
//! - Send/Sync only implemented when T: Send (ensures safe value transfer)

use parking_lot::Mutex;
use std::cell::UnsafeCell;
use std::marker::PhantomData;
use std::sync::Arc;

use super::{Arena, ArenaConfig, ArenaStats};
use crate::error::MemoryError;

/// An arena that can be safely moved between threads
///
/// Unlike ThreadSafeArena which allows concurrent access, CrossThreadArena
/// ensures exclusive access but can be passed between threads.
pub struct CrossThreadArena {
    inner: Arc<Mutex<Arena>>,
}

impl CrossThreadArena {
    /// Create a new cross-thread arena
    pub fn new(config: ArenaConfig) -> Self {
        CrossThreadArena {
            inner: Arc::new(Mutex::new(Arena::new(config))),
        }
    }

    /// Create with default configuration
    pub fn default() -> Self {
        Self::new(ArenaConfig::default())
    }

    /// Lock the arena for exclusive access
    pub fn lock(&self) -> CrossThreadArenaGuard<'_> {
        CrossThreadArenaGuard {
            guard: self.inner.lock(),
        }
    }

    /// Try to lock the arena without blocking
    pub fn try_lock(&self) -> Option<CrossThreadArenaGuard<'_>> {
        self.inner
            .try_lock()
            .map(|guard| CrossThreadArenaGuard { guard })
    }

    /// Reset the arena
    pub fn reset(&mut self) {
        self.inner.lock().reset();
    }

    /// Get statistics
    pub fn stats(&self) -> ArenaStats {
        self.inner.lock().stats().snapshot().into()
    }

    /// Create a reference that can be sent across threads
    #[must_use = "allocated memory must be used"]
    pub fn create_ref<T>(&self, value: T) -> Result<CrossThreadArenaRef<T>, MemoryError> {
        let guard = self.inner.lock();
        let ptr = guard.alloc(value)?;

        Ok(CrossThreadArenaRef {
            ptr: UnsafeCell::new(ptr as *mut T),
            arena: Arc::clone(&self.inner),
            _phantom: PhantomData,
        })
    }
}

impl Clone for CrossThreadArena {
    fn clone(&self) -> Self {
        CrossThreadArena {
            inner: Arc::clone(&self.inner),
        }
    }
}

// SAFETY: CrossThreadArena can be safely sent between threads.
// - Arc<Mutex<Arena>> is both Send and Sync
// - Mutex ensures exclusive access (no data races)
// - Arena is accessed only while holding lock
unsafe impl Send for CrossThreadArena {}

// SAFETY: CrossThreadArena can be safely shared between threads.
// - Arc allows shared ownership across threads
// - Mutex synchronizes all arena access
// - All operations lock the mutex before accessing Arena
unsafe impl Sync for CrossThreadArena {}

/// Guard for exclusive access to the arena
pub struct CrossThreadArenaGuard<'a> {
    guard: parking_lot::MutexGuard<'a, Arena>,
}

impl<'a> CrossThreadArenaGuard<'a> {
    /// Allocate bytes
    #[must_use = "allocated memory must be used"]
    pub fn alloc_bytes(&self, size: usize, align: usize) -> Result<*mut u8, MemoryError> {
        self.guard.alloc_bytes_aligned(size, align)
    }

    /// Allocate a value
    #[must_use = "allocated memory must be used"]
    pub fn alloc<T>(&self, value: T) -> Result<&mut T, MemoryError> {
        self.guard.alloc(value)
    }

    /// Allocate a slice
    #[must_use = "allocated memory must be used"]
    pub fn alloc_slice<T>(&self, slice: &[T]) -> Result<&mut [T], MemoryError>
    where
        T: Copy,
    {
        self.guard.alloc_slice(slice)
    }

    /// Allocate a string
    #[must_use = "allocated memory must be used"]
    pub fn alloc_str(&self, s: &str) -> Result<&str, MemoryError> {
        self.guard.alloc_str(s)
    }
}

/// A reference that can be sent across threads
///
/// This reference ensures the arena stays alive and provides
/// synchronized access to the referenced value.
pub struct CrossThreadArenaRef<T> {
    ptr: UnsafeCell<*mut T>,
    arena: Arc<Mutex<Arena>>,
    _phantom: PhantomData<T>,
}

impl<T> CrossThreadArenaRef<T> {
    /// Get a reference to the value
    ///
    /// This locks the arena for the duration of the access.
    pub fn with<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&T) -> R,
    {
        let _guard = self.arena.lock();
        // SAFETY: Dereferencing arena-allocated pointer.
        // - ptr was obtained from arena.alloc (valid allocation)
        // - _guard holds mutex lock (exclusive access, no concurrent modification)
        // - Arena remains alive (Arc keeps it valid)
        // - ptr is NonNull (allocated successfully in create_ref)
        unsafe {
            let ptr = *self.ptr.get();
            f(&*ptr)
        }
    }

    /// Get a mutable reference to the value
    ///
    /// This locks the arena for the duration of the access.
    pub fn with_mut<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut T) -> R,
    {
        let _guard = self.arena.lock();
        // SAFETY: Creating mutable reference to arena-allocated value.
        // - ptr was obtained from arena.alloc (valid allocation)
        // - _guard holds mutex lock (exclusive access, no other references exist)
        // - Arena remains alive (Arc keeps it valid)
        // - ptr is NonNull (allocated successfully in create_ref)
        unsafe {
            let ptr = *self.ptr.get();
            f(&mut *ptr)
        }
    }
}

// SAFETY: CrossThreadArenaRef can be sent between threads if T: Send.
// - ptr is protected by UnsafeCell (no direct access without lock)
// - Arena is protected by Mutex (exclusive access)
// - T: Send requirement ensures value can be safely sent
// - Arc<Mutex<Arena>> is both Send and Sync
unsafe impl<T: Send> Send for CrossThreadArenaRef<T> {}

// SAFETY: CrossThreadArenaRef can be shared between threads if T: Send.
// - All access requires locking arena mutex (synchronized)
// - UnsafeCell prevents data races (requires lock for access)
// - T: Send requirement ensures value can be safely accessed from any thread
// - Multiple threads can hold CrossThreadArenaRef, but only one can access at a time
unsafe impl<T: Send> Sync for CrossThreadArenaRef<T> {}

impl<T: Clone> Clone for CrossThreadArenaRef<T> {
    fn clone(&self) -> Self {
        let value = self.with(|v| v.clone());
        let guard = self.arena.lock();
        let ptr = guard.alloc(value).unwrap();

        CrossThreadArenaRef {
            ptr: UnsafeCell::new(ptr as *mut T),
            arena: Arc::clone(&self.arena),
            _phantom: PhantomData,
        }
    }
}

/// Builder pattern for cross-thread arena
pub struct CrossThreadArenaBuilder {
    config: ArenaConfig,
}

impl CrossThreadArenaBuilder {
    pub fn new() -> Self {
        CrossThreadArenaBuilder {
            config: ArenaConfig::default(),
        }
    }

    #[must_use = "builder methods must be chained or built"]
    pub fn initial_size(mut self, size: usize) -> Self {
        self.config.initial_size = size;
        self
    }

    #[must_use = "builder methods must be chained or built"]
    pub fn growth_factor(mut self, factor: f64) -> Self {
        self.config.growth_factor = factor;
        self
    }

    #[must_use = "builder methods must be chained or built"]
    pub fn max_chunk_size(mut self, size: usize) -> Self {
        self.config.max_chunk_size = size;
        self
    }

    #[must_use = "builder methods must be chained or built"]
    pub fn track_stats(mut self, enabled: bool) -> Self {
        self.config.track_stats = enabled;
        self
    }

    #[must_use = "builder methods must be chained or built"]
    pub fn zero_memory(mut self, enabled: bool) -> Self {
        self.config.zero_memory = enabled;
        self
    }

    pub fn build(self) -> CrossThreadArena {
        CrossThreadArena::new(self.config)
    }
}

impl Default for CrossThreadArenaBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::thread;

    use super::*;

    #[test]
    fn test_basic_usage() {
        let arena = CrossThreadArena::default();

        {
            let guard = arena.lock();
            let x = guard.alloc(42u32).unwrap();
            assert_eq!(*x, 42);
        }

        {
            let guard = arena.lock();
            let y = guard.alloc(100u64).unwrap();
            assert_eq!(*y, 100);
        }
    }

    #[test]
    fn test_cross_thread_ref() {
        let arena = CrossThreadArena::default();
        let reference = arena.create_ref(vec![1, 2, 3, 4, 5]).unwrap();

        // Access from main thread
        reference.with(|v| assert_eq!(v, &vec![1, 2, 3, 4, 5]));

        // Send to another thread
        let handle = thread::spawn(move || {
            reference.with(|v| {
                assert_eq!(v.len(), 5);
                v[0] + v[4]
            })
        });

        let result = handle.join().unwrap();
        assert_eq!(result, 6); // 1 + 5
    }

    #[test]
    fn test_multiple_threads() {
        let arena = CrossThreadArena::default();

        let handles: Vec<_> = (0..10)
            .map(|i| {
                let arena_clone = arena.clone();
                thread::spawn(move || {
                    let guard = arena_clone.lock();
                    let value = guard.alloc(i * 100).unwrap();
                    *value
                })
            })
            .collect();

        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();

        for (i, &result) in results.iter().enumerate() {
            assert_eq!(result, i * 100);
        }
    }

    #[test]
    fn test_builder() {
        let arena = CrossThreadArenaBuilder::new()
            .initial_size(8192)
            .growth_factor(1.5)
            .track_stats(true)
            .zero_memory(true)
            .build();

        let guard = arena.lock();
        let data = guard.alloc_bytes(100, 8).unwrap();

        // Check that memory is zeroed
        // SAFETY: Reading from allocated memory.
        // - data is valid pointer from alloc_bytes
        // - Loop indices are within allocated size (0..100)
        // - Memory should be zeroed per config
        unsafe {
            for i in 0..100 {
                assert_eq!(*data.add(i), 0);
            }
        }
    }

    #[test]
    fn test_try_lock() {
        let arena = CrossThreadArena::default();

        // First lock succeeds
        let _guard1 = arena.lock();

        // Second try_lock fails (would block)
        assert!(arena.try_lock().is_none());
    }

    #[test]
    fn test_ref_mutation() {
        let arena = CrossThreadArena::default();
        let reference = arena.create_ref(vec![1, 2, 3]).unwrap();

        reference.with_mut(|v| {
            v.push(4);
            v.push(5);
        });

        reference.with(|v| {
            assert_eq!(v, &vec![1, 2, 3, 4, 5]);
        });
    }
}
