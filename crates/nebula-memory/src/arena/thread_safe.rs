//! High-performance thread-safe arena allocator

use std::alloc::{alloc, dealloc, Layout};
use std::mem;
use std::ptr::{self, NonNull};
use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;

use super::{ArenaAllocate, ArenaConfig, ArenaStats};
use crate::core::error::MemoryError;
use crate::utils::align_up;

/// Thread-safe memory chunk with atomic bump pointer
struct ThreadSafeChunk {
    ptr: NonNull<u8>,
    capacity: usize,
    used: AtomicUsize,
}

impl ThreadSafeChunk {
    /// Creates a new chunk with specified size (minimum 64 bytes)
    fn new(size: usize) -> Result<Self, MemoryError> {
        let size = size.max(64); // Minimum chunk size to reduce overhead
        let layout = Layout::from_size_align(size, 1).map_err(|_| MemoryError::invalid_layout())?;

        // Safety: Layout is non-zero and properly aligned
        let ptr = unsafe { alloc(layout) };
        let ptr =
            NonNull::new(ptr).ok_or_else(|| MemoryError::out_of_memory(size, 0))?;

        Ok(Self { ptr, capacity: size, used: AtomicUsize::new(0) })
    }

    /// Attempts to allocate from this chunk
    #[inline]
    fn try_alloc(&self, size: usize, align: usize) -> Option<*mut u8> {
        let mut current = self.used.load(Ordering::Relaxed);

        loop {
            let aligned_pos = align_up(current, align);
            let new_used = aligned_pos + size;

            if new_used > self.capacity {
                return None;
            }

            match self.used.compare_exchange_weak(
                current,
                new_used,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => return Some(unsafe { self.ptr.as_ptr().add(aligned_pos) }),
                Err(actual) => current = actual,
            }
        }
    }
}

impl Drop for ThreadSafeChunk {
    fn drop(&mut self) {
        unsafe {
            dealloc(self.ptr.as_ptr(), Layout::from_size_align_unchecked(self.capacity, 1));
        }
    }
}

unsafe impl Send for ThreadSafeChunk {}
unsafe impl Sync for ThreadSafeChunk {}

/// Thread-safe arena allocator using atomic operations and lock-free fast paths
pub struct ThreadSafeArena {
    chunks: RwLock<Vec<Arc<ThreadSafeChunk>>>,
    current_chunk: AtomicPtr<ThreadSafeChunk>,
    config: ArenaConfig,
    stats: ArenaStats,
    chunk_mutex: Mutex<()>, // For chunk allocation synchronization
}

impl ThreadSafeArena {
    /// Creates a new thread-safe arena with given configuration
    pub fn new(config: ArenaConfig) -> Self {
        Self {
            chunks: RwLock::new(Vec::new()),
            current_chunk: AtomicPtr::new(ptr::null_mut()),
            config,
            stats: ArenaStats::new(),
            chunk_mutex: Mutex::new(()),
        }
    }

    /// Creates arena with production config - optimized for performance
    pub fn production(capacity: usize) -> Self {
        Self::new(ArenaConfig::production().with_initial_size(capacity))
    }

    /// Creates arena with debug config - optimized for debugging
    pub fn debug(capacity: usize) -> Self {
        Self::new(ArenaConfig::debug().with_initial_size(capacity))
    }

    /// Creates arena with performance config (alias for production)
    pub fn performance(capacity: usize) -> Self {
        Self::production(capacity)
    }

    /// Creates arena with conservative config - balanced
    pub fn conservative(capacity: usize) -> Self {
        Self::new(ArenaConfig::conservative().with_initial_size(capacity))
    }

    /// Creates a tiny thread-safe arena (4KB)
    pub fn tiny() -> Self {
        Self::new(ArenaConfig::small_objects().with_initial_size(4 * 1024))
    }

    /// Creates a small thread-safe arena (64KB)
    pub fn small() -> Self {
        Self::new(ArenaConfig::default().with_initial_size(64 * 1024))
    }

    /// Creates a medium thread-safe arena (1MB)
    pub fn medium() -> Self {
        Self::new(ArenaConfig::default().with_initial_size(1024 * 1024))
    }

    /// Creates a large thread-safe arena (16MB)
    pub fn large() -> Self {
        Self::new(ArenaConfig::large_objects().with_initial_size(16 * 1024 * 1024))
    }

    /// Allocates a new chunk when needed
    fn allocate_chunk(&self, min_size: usize) -> Result<(), MemoryError> {
        let _lock = self.chunk_mutex.lock().unwrap();

        // Double-check if another thread already allocated
        let current = self.current_chunk.load(Ordering::Acquire);
        if !current.is_null() {
            let chunk = unsafe { &*current };
            if chunk.try_alloc(min_size, 1).is_some() {
                return Ok(());
            }
        }

        // Calculate new chunk size
        let chunks = self.chunks.read().unwrap();
        let chunk_size = if chunks.is_empty() {
            self.config.initial_size.max(min_size)
        } else {
            let last_size = chunks.last().unwrap().capacity;
            let new_size = (last_size as f64 * self.config.growth_factor) as usize;
            new_size.max(min_size).min(self.config.max_chunk_size)
        };
        drop(chunks);

        // Create and initialize new chunk
        let chunk = ThreadSafeChunk::new(chunk_size)?;
        if self.config.zero_memory {
            unsafe {
                ptr::write_bytes(chunk.ptr.as_ptr(), 0, chunk_size);
            }
        }

        let chunk = Arc::new(chunk);
        let chunk_ptr = Arc::as_ptr(&chunk) as *mut ThreadSafeChunk;

        // Update current chunk pointer
        self.current_chunk.store(chunk_ptr, Ordering::Release);

        // Add to chunks list
        let mut chunks = self.chunks.write().unwrap();
        chunks.push(chunk);

        // Update statistics
        if self.config.track_stats {
            self.stats.record_chunk_allocation(chunk_size);
        }

        Ok(())
    }

    /// Allocates aligned memory block (thread-safe)
    pub fn alloc_bytes_aligned(&self, size: usize, align: usize) -> Result<*mut u8, MemoryError> {
        if !align.is_power_of_two() {
            return Err(MemoryError::invalid_alignment(align, 0));
        }

        let start_time = self.config.track_stats.then(Instant::now);

        // Fast path: try current chunk first
        let current = self.current_chunk.load(Ordering::Acquire);
        if !current.is_null() {
            let chunk = unsafe { &*current };
            if let Some(ptr) = chunk.try_alloc(size, align) {
                if let Some(start) = start_time {
                    self.stats.record_allocation(size, start.elapsed().as_nanos() as u64);
                }
                return Ok(ptr);
            }
        }

        // Slow path: allocate new chunk
        self.allocate_chunk(size + align)?;

        // Try again with new chunk
        let current = self.current_chunk.load(Ordering::Acquire);
        let chunk = unsafe { &*current };
        chunk.try_alloc(size, align).ok_or(MemoryError::allocation_failed()).map(|ptr| {
            if let Some(start) = start_time {
                self.stats.record_allocation(size, start.elapsed().as_nanos() as u64);
            }
            ptr
        })
    }

    /// Allocates and initializes a value (thread-safe)
    pub fn alloc<T>(&self, value: T) -> Result<&T, MemoryError> {
        let ptr = self.alloc_bytes_aligned(mem::size_of::<T>(), mem::align_of::<T>())? as *mut T;

        // Safety: We just allocated properly aligned space for T
        unsafe {
            ptr.write(value);
            Ok(&*ptr)
        }
    }

    /// Allocates and copies a slice (thread-safe)
    pub fn alloc_slice<T: Copy>(&self, slice: &[T]) -> Result<&[T], MemoryError> {
        if slice.is_empty() {
            return Ok(&[]);
        }

        let ptr =
            self.alloc_bytes_aligned(mem::size_of_val(slice), mem::align_of::<T>())? as *mut T;

        // Safety: We just allocated properly aligned space for the slice
        unsafe {
            ptr::copy_nonoverlapping(slice.as_ptr(), ptr, slice.len());
            Ok(&*ptr::slice_from_raw_parts(ptr, slice.len()))
        }
    }

    /// Allocates a string (thread-safe)
    pub fn alloc_str(&self, s: &str) -> Result<&str, MemoryError> {
        let bytes = self.alloc_slice(s.as_bytes())?;
        // Safety: Input is valid UTF-8 since it comes from &str
        unsafe { Ok(std::str::from_utf8_unchecked(bytes)) }
    }

    /// Resets the arena (not thread-safe during concurrent allocations)
    pub fn reset(&mut self) {
        let start_time = self.config.track_stats.then(Instant::now);

        let mut chunks = self.chunks.write().unwrap();
        chunks.clear();
        self.current_chunk.store(ptr::null_mut(), Ordering::Release);

        if let Some(start) = start_time {
            self.stats.record_reset(start.elapsed().as_nanos() as u64);
        }
    }

    /// Returns reference to statistics
    pub fn stats(&self) -> &ArenaStats {
        &self.stats
    }
}

impl ArenaAllocate for ThreadSafeArena {
    unsafe fn alloc_bytes(&self, size: usize, align: usize) -> Result<*mut u8, MemoryError> {
        self.alloc_bytes_aligned(size, align)
    }

    fn stats(&self) -> &ArenaStats {
        &self.stats
    }

    fn reset(&mut self) {
        self.reset();
    }
}

unsafe impl Send for ThreadSafeArena {}
unsafe impl Sync for ThreadSafeArena {}

/// Thread-safe reference to an arena-allocated value
pub struct ThreadSafeArenaRef<'a, T: ?Sized + Sync> {
    ptr: NonNull<T>,
    _arena: &'a ThreadSafeArena,
}

impl<'a, T: ?Sized + Sync> ThreadSafeArenaRef<'a, T> {
    /// Creates a new thread-safe reference from raw pointer
    pub(crate) fn new(ptr: NonNull<T>, arena: &'a ThreadSafeArena) -> Self {
        Self { ptr, _arena: arena }
    }

    /// Gets immutable reference to the value
    pub fn get(&self) -> &T {
        // Safety: The pointer is valid for the lifetime of the arena
        unsafe { self.ptr.as_ref() }
    }
}

impl<'a, T: ?Sized + Sync> std::ops::Deref for ThreadSafeArenaRef<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.get()
    }
}

// Implement Send and Sync for ThreadSafeArenaRef if T is Sync
unsafe impl<'a, T: ?Sized + Sync> Send for ThreadSafeArenaRef<'a, T> {}
unsafe impl<'a, T: ?Sized + Sync> Sync for ThreadSafeArenaRef<'a, T> {}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::thread;

    use super::*;
    use crate::utils::is_aligned;

    #[test]
    fn basic_allocation() {
        let arena = ThreadSafeArena::new(ArenaConfig::default());
        let value = arena.alloc(42u32).unwrap();
        assert_eq!(*value, 42);
    }

    #[test]
    fn concurrent_allocations() {
        let arena = Arc::new(ThreadSafeArena::new(ArenaConfig::default()));
        let mut handles = vec![];

        for i in 0..10 {
            let arena = Arc::clone(&arena);
            handles.push(thread::spawn(move || {
                let val = arena.alloc(i).unwrap();
                assert_eq!(*val, i);
            }));
        }

        for handle in handles {
            handle.join().unwrap();
        }
    }

    #[test]
    fn chunk_growth() {
        let config =
            ArenaConfig::default().with_initial_size(128).with_growth_factor(2.0).with_stats(true);
        let arena = ThreadSafeArena::new(config);

        // First allocation fits in initial chunk
        let _ = arena.alloc_bytes_aligned(64, 1).unwrap();

        // This should trigger chunk growth
        let _ = arena.alloc_bytes_aligned(256, 1).unwrap();

        assert!(arena.stats().chunks_allocated() > 1);
    }

    #[test]
    fn alignment_requirements() {
        let arena = ThreadSafeArena::new(ArenaConfig::default());

        let p1 = arena.alloc_bytes_aligned(1, 1).unwrap();
        assert!(is_aligned(p1 as usize, 1));

        let p64 = arena.alloc_bytes_aligned(1, 64).unwrap();
        assert!(is_aligned(p64 as usize, 64));
    }

    #[test]
    fn string_allocation() {
        let arena = ThreadSafeArena::new(ArenaConfig::default());
        let s = arena.alloc_str("test").unwrap();
        assert_eq!(s, "test");
    }
}
