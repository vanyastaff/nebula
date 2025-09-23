//! Cross-thread arena implementation that can be safely moved between threads

use std::cell::UnsafeCell;
use std::marker::PhantomData;
use std::sync::{Arc, Mutex};

use super::{Arena, ArenaAllocate, ArenaConfig, ArenaStats};
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
        CrossThreadArena { inner: Arc::new(Mutex::new(Arena::new(config))) }
    }

    /// Create with default configuration
    pub fn default() -> Self {
        Self::new(ArenaConfig::default())
    }

    /// Lock the arena for exclusive access
    pub fn lock(&self) -> CrossThreadArenaGuard<'_> {
        CrossThreadArenaGuard { guard: self.inner.lock().unwrap() }
    }

    /// Try to lock the arena without blocking
    pub fn try_lock(&self) -> Option<CrossThreadArenaGuard<'_>> {
        self.inner.try_lock().ok().map(|guard| CrossThreadArenaGuard { guard })
    }

    /// Reset the arena
    pub fn reset(&mut self) {
        self.inner.lock().unwrap().reset();
    }

    /// Get statistics
    pub fn stats(&self) -> ArenaStats {
        self.inner.lock().unwrap().stats().snapshot().into()
    }

    /// Create a reference that can be sent across threads
    pub fn create_ref<T>(&self, value: T) -> Result<CrossThreadArenaRef<T>, MemoryError> {
        let guard = self.inner.lock().unwrap();
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
        CrossThreadArena { inner: Arc::clone(&self.inner) }
    }
}

unsafe impl Send for CrossThreadArena {}
unsafe impl Sync for CrossThreadArena {}

/// Guard for exclusive access to the arena
pub struct CrossThreadArenaGuard<'a> {
    guard: std::sync::MutexGuard<'a, Arena>,
}

impl<'a> CrossThreadArenaGuard<'a> {
    /// Allocate bytes
    pub fn alloc_bytes(&self, size: usize, align: usize) -> Result<*mut u8, MemoryError> {
        self.guard.alloc_bytes_aligned(size, align)
    }

    /// Allocate a value
    pub fn alloc<T>(&self, value: T) -> Result<&mut T, MemoryError> {
        self.guard.alloc(value)
    }

    /// Allocate a slice
    pub fn alloc_slice<T>(&self, slice: &[T]) -> Result<&mut [T], MemoryError>
    where T: Copy {
        self.guard.alloc_slice(slice)
    }

    /// Allocate a string
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
    where F: FnOnce(&T) -> R {
        let _guard = self.arena.lock().unwrap();
        unsafe {
            let ptr = *self.ptr.get();
            f(&*ptr)
        }
    }

    /// Get a mutable reference to the value
    ///
    /// This locks the arena for the duration of the access.
    pub fn with_mut<F, R>(&self, f: F) -> R
    where F: FnOnce(&mut T) -> R {
        let _guard = self.arena.lock().unwrap();
        unsafe {
            let ptr = *self.ptr.get();
            f(&mut *ptr)
        }
    }
}

unsafe impl<T: Send> Send for CrossThreadArenaRef<T> {}
unsafe impl<T: Send> Sync for CrossThreadArenaRef<T> {}

impl<T: Clone> Clone for CrossThreadArenaRef<T> {
    fn clone(&self) -> Self {
        let value = self.with(|v| v.clone());
        let guard = self.arena.lock().unwrap();
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
        CrossThreadArenaBuilder { config: ArenaConfig::default() }
    }

    pub fn initial_size(mut self, size: usize) -> Self {
        self.config.initial_size = size;
        self
    }

    pub fn growth_factor(mut self, factor: f64) -> Self {
        self.config.growth_factor = factor;
        self
    }

    pub fn max_chunk_size(mut self, size: usize) -> Self {
        self.config.max_chunk_size = size;
        self
    }

    pub fn track_stats(mut self, enabled: bool) -> Self {
        self.config.track_stats = enabled;
        self
    }

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
