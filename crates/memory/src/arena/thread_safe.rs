//! High-performance thread-safe arena allocator
//!
//! # Safety
//!
//! This module implements a thread-safe arena using atomic operations and careful synchronization:
//! - Chunks use atomic bump pointers (compare-and-swap) for lock-free allocations
//! - A global `current_chunk` pointer enables fast-path allocations without locks
//! - Chunk allocation is synchronized with a mutex to prevent races
//! - All memory is properly aligned and lifetime-bound to the arena
//! - Memory is never deallocated until the arena is reset or dropped
//!
//! ## Memory Safety
//!
//! - **Allocation**: Atomic CAS ensures no two threads allocate overlapping memory
//! - **Deallocation**: Chunks are deallocated only when arena is dropped (no use-after-free)
//! - **Alignment**: All allocations respect type alignment requirements
//! - **Lifetime**: References are bound to arena lifetime ('a), preventing dangling pointers
//! - **Concurrency**: Send/Sync bounds ensure proper thread safety

use parking_lot::{Mutex, RwLock};
use std::alloc::{Layout, alloc, dealloc};
use std::mem;
use std::ptr::{self, NonNull};
use std::sync::Arc;
use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};
use std::time::Instant;

use super::{ArenaAllocate, ArenaConfig, ArenaStats};
use crate::error::MemoryError;
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
        let layout = Layout::from_size_align(size, 1)
            .map_err(|_| MemoryError::invalid_layout("layout creation failed"))?;

        // SAFETY: Allocating raw memory from global allocator.
        // - Layout has non-zero size (at least 64 bytes) and valid alignment (1)
        // - Returned pointer is checked for null via NonNull::new
        // - Memory will be deallocated in Drop impl with same layout
        let ptr = unsafe { alloc(layout) };
        let ptr = NonNull::new(ptr).ok_or_else(|| MemoryError::out_of_memory(size, 0))?;

        Ok(Self {
            ptr,
            capacity: size,
            used: AtomicUsize::new(0),
        })
    }

    /// Attempts to allocate from this chunk
    #[inline]
    fn try_alloc(&self, size: usize, align: usize) -> Option<*mut u8> {
        let base = self.ptr.as_ptr() as usize;
        let mut current = self.used.load(Ordering::Relaxed);

        loop {
            // Align the absolute address, then convert back to offset
            let aligned_addr = align_up(base + current, align);
            let aligned_pos = aligned_addr - base;
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
                // SAFETY: Pointer arithmetic within allocated chunk bounds.
                // - self.ptr is a valid pointer from alloc() in new()
                // - aligned_pos is within [0, capacity) due to check above
                // - CAS success means we exclusively own [aligned_pos, new_used) range
                // - AcqRel ordering synchronizes with other threads
                Ok(_) => return Some(unsafe { self.ptr.as_ptr().add(aligned_pos) }),
                Err(actual) => current = actual,
            }
        }
    }
}

impl Drop for ThreadSafeChunk {
    fn drop(&mut self) {
        // SAFETY: Deallocating memory allocated in new().
        // - self.ptr was allocated with alloc() in new() with same layout
        // - Layout::from_size_align_unchecked is safe because:
        //   * capacity was validated in new() (at least 64, within isize::MAX)
        //   * align=1 is always valid
        // - This is the only deallocation (Drop is called exactly once)
        unsafe {
            dealloc(
                self.ptr.as_ptr(),
                Layout::from_size_align_unchecked(self.capacity, 1),
            );
        }
    }
}

// SAFETY: ThreadSafeChunk is Send because:
// - ptr: NonNull<u8> is Send (raw pointer to untyped memory)
// - capacity: usize is Send (primitive type)
// - used: AtomicUsize is Send (atomic primitive)
// - No interior references or thread-local state
unsafe impl Send for ThreadSafeChunk {}

// SAFETY: ThreadSafeChunk is Sync because:
// - All mutations go through atomic operations (used.compare_exchange_weak)
// - Memory pointed to by ptr is allocated but uninitialized (no aliasing concerns)
// - Chunk allocation is bump-pointer style (disjoint regions per thread)
// - AcqRel memory ordering ensures proper synchronization between threads
// - No shared mutable state (ptr/capacity are immutable after creation)
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
    #[must_use]
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
    #[must_use]
    pub fn production(capacity: usize) -> Self {
        Self::new(ArenaConfig::production().with_initial_size(capacity))
    }

    /// Creates arena with debug config - optimized for debugging
    #[must_use]
    pub fn debug(capacity: usize) -> Self {
        Self::new(ArenaConfig::debug().with_initial_size(capacity))
    }

    /// Creates arena with performance config (alias for production)
    #[must_use]
    pub fn performance(capacity: usize) -> Self {
        Self::production(capacity)
    }

    /// Creates arena with conservative config - balanced
    #[must_use]
    pub fn conservative(capacity: usize) -> Self {
        Self::new(ArenaConfig::conservative().with_initial_size(capacity))
    }

    /// Creates a tiny thread-safe arena (4KB)
    #[must_use]
    pub fn tiny() -> Self {
        Self::new(ArenaConfig::small_objects().with_initial_size(4 * 1024))
    }

    /// Creates a small thread-safe arena (64KB)
    #[must_use]
    pub fn small() -> Self {
        Self::new(ArenaConfig::default().with_initial_size(64 * 1024))
    }

    /// Creates a medium thread-safe arena (1MB)
    #[must_use]
    pub fn medium() -> Self {
        Self::new(ArenaConfig::default().with_initial_size(1024 * 1024))
    }

    /// Creates a large thread-safe arena (16MB)
    #[must_use]
    pub fn large() -> Self {
        Self::new(ArenaConfig::large_objects().with_initial_size(16 * 1024 * 1024))
    }

    /// Allocates a new chunk when needed
    fn allocate_chunk(&self, min_size: usize) -> Result<(), MemoryError> {
        let _lock = self.chunk_mutex.lock();

        // Double-check if another thread already allocated
        let current = self.current_chunk.load(Ordering::Acquire);
        if !current.is_null() {
            // SAFETY: current_chunk is only set to non-null pointers created from Arc::as_ptr.
            // - The Arc is stored in self.chunks, keeping the chunk alive
            // - Acquire ordering synchronizes with Release store in allocate_chunk
            // - Multiple threads can safely read the chunk (Chunk is Sync)
            let chunk = unsafe { &*current };
            if chunk.try_alloc(min_size, 1).is_some() {
                return Ok(());
            }
        }

        // Calculate new chunk size
        let chunks = self.chunks.read();
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
            // SAFETY: Writing zeros to freshly allocated memory.
            // - chunk.ptr is a valid pointer from ThreadSafeChunk::new
            // - chunk_size is the exact allocation size (within bounds)
            // - Memory is exclusively owned (no other references exist yet)
            // - write_bytes is safe for any byte value (0)
            unsafe {
                ptr::write_bytes(chunk.ptr.as_ptr(), 0, chunk_size);
            }
        }

        let chunk = Arc::new(chunk);
        let chunk_ptr = Arc::as_ptr(&chunk).cast_mut();

        // Update current chunk pointer
        self.current_chunk.store(chunk_ptr, Ordering::Release);

        // Add to chunks list
        let mut chunks = self.chunks.write();
        chunks.push(chunk);

        // Update statistics
        if self.config.track_stats {
            self.stats.record_chunk_allocation(chunk_size);
        }

        Ok(())
    }

    /// Allocates aligned memory block (thread-safe)
    #[inline]
    #[must_use = "allocated memory must be used"]
    pub fn alloc_bytes_aligned(&self, size: usize, align: usize) -> Result<*mut u8, MemoryError> {
        if !align.is_power_of_two() {
            return Err(MemoryError::invalid_alignment(align));
        }

        let start_time = self.config.track_stats.then(Instant::now);

        // Fast path: try current chunk first
        let current = self.current_chunk.load(Ordering::Acquire);
        if !current.is_null() {
            // SAFETY: Dereferencing current_chunk pointer (fast path).
            // - Pointer is from Arc::as_ptr, kept alive by Arc in self.chunks
            // - Acquire ordering synchronizes with Release in allocate_chunk
            // - Chunk is Sync, so concurrent access is safe
            let chunk = unsafe { &*current };
            if let Some(ptr) = chunk.try_alloc(size, align) {
                if let Some(start) = start_time {
                    self.stats
                        .record_allocation(size, start.elapsed().as_nanos() as u64);
                }
                return Ok(ptr);
            }
        }

        // Slow path: allocate new chunk
        self.allocate_chunk(size + align)?;

        // Try again with new chunk
        let current = self.current_chunk.load(Ordering::Acquire);
        // SAFETY: Dereferencing current_chunk after successful chunk allocation.
        // - allocate_chunk() just set current_chunk to a valid Arc pointer
        // - Acquire ordering synchronizes with Release store
        // - Chunk cannot be deallocated (still in self.chunks)
        let chunk = unsafe { &*current };
        chunk
            .try_alloc(size, align)
            .ok_or(MemoryError::allocation_failed(0, 1))
            .inspect(|_ptr| {
                if let Some(start) = start_time {
                    self.stats
                        .record_allocation(size, start.elapsed().as_nanos() as u64);
                }
            })
    }

    /// Allocates and initializes a value (thread-safe)
    #[inline]
    #[must_use = "allocated memory must be used"]
    pub fn alloc<T>(&self, value: T) -> Result<&T, MemoryError> {
        let ptr =
            (self.alloc_bytes_aligned(mem::size_of::<T>(), mem::align_of::<T>())?).cast::<T>();

        // SAFETY: Writing initialized value to freshly allocated memory.
        // - ptr is properly aligned for T (alloc_bytes_aligned ensures this)
        // - ptr points to size_of::<T>() bytes of valid memory
        // - Memory is uninitialized, so write() is correct (doesn't drop old value)
        // - Returned reference is bound to arena lifetime (no use-after-free)
        // - Arena never deallocates until reset/drop (reference stays valid)
        unsafe {
            ptr.write(value);
            Ok(&*ptr)
        }
    }

    /// Allocates and copies a slice (thread-safe)
    #[inline]
    #[must_use = "allocated memory must be used"]
    pub fn alloc_slice<T: Copy>(&self, slice: &[T]) -> Result<&[T], MemoryError> {
        if slice.is_empty() {
            return Ok(&[]);
        }

        let ptr =
            (self.alloc_bytes_aligned(mem::size_of_val(slice), mem::align_of::<T>())?).cast::<T>();

        // SAFETY: Copying slice data to freshly allocated memory.
        // - ptr is properly aligned for T (alloc_bytes_aligned ensures this)
        // - ptr has space for slice.len() elements (size_of_val checks this)
        // - slice.as_ptr() and ptr don't overlap (ptr is freshly allocated)
        // - T is Copy, so bitwise copy is safe
        // - Returned slice reference is bound to arena lifetime
        // - Arena never deallocates until reset/drop (slice stays valid)
        unsafe {
            ptr::copy_nonoverlapping(slice.as_ptr(), ptr, slice.len());
            Ok(&*ptr::slice_from_raw_parts(ptr, slice.len()))
        }
    }

    /// Allocates a string (thread-safe)
    #[must_use = "allocated memory must be used"]
    pub fn alloc_str(&self, s: &str) -> Result<&str, MemoryError> {
        let bytes = self.alloc_slice(s.as_bytes())?;
        // SAFETY: Creating &str from copied bytes.
        // - Input s is &str, so bytes are valid UTF-8
        // - alloc_slice performs exact byte-for-byte copy
        // - No mutations occur (bytes slice is immutable)
        // - UTF-8 validity is preserved through the copy
        unsafe { Ok(std::str::from_utf8_unchecked(bytes)) }
    }

    /// Resets the arena (not thread-safe during concurrent allocations)
    pub fn reset(&mut self) {
        let start_time = self.config.track_stats.then(Instant::now);

        let mut chunks = self.chunks.write();
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
    /// # Safety
    ///
    /// Caller must ensure:
    /// - `size` is non-zero and within allocator limits
    /// - `align` is a power of two
    /// - Returned pointer is not used after arena reset/drop
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

// SAFETY: ThreadSafeArena is Send because:
// - chunks: RwLock<Vec<Arc<ThreadSafeChunk>>> is Send (all components are Send)
// - current_chunk: AtomicPtr<ThreadSafeChunk> is Send (atomic primitive)
// - config: ArenaConfig is Send (simple config struct)
// - stats: ArenaStats is Send (atomic counters)
// - chunk_mutex: Mutex<()> is Send
// - All owned data can be safely transferred to another thread
unsafe impl Send for ThreadSafeArena {}

// SAFETY: ThreadSafeArena is Sync because:
// - All allocation operations are thread-safe (atomic CAS in chunks)
// - current_chunk uses Acquire/Release ordering for synchronization
// - chunk_mutex protects chunk allocation from races
// - RwLock protects chunks Vec from concurrent modification
// - All public methods are either &self (immutable) or &mut self (exclusive)
// - Stats recording uses atomic operations
// - Allocated memory is never freed until reset/drop (no dangling pointers)
unsafe impl Sync for ThreadSafeArena {}

/// Thread-safe reference to an arena-allocated value
pub struct ThreadSafeArenaRef<'a, T: ?Sized + Sync> {
    ptr: NonNull<T>,
    _arena: &'a ThreadSafeArena,
}

impl<'a, T: ?Sized + Sync> ThreadSafeArenaRef<'a, T> {
    /// Creates a new thread-safe reference from raw pointer
    #[allow(dead_code)] // public API not yet wired up
    pub(crate) fn new(ptr: NonNull<T>, arena: &'a ThreadSafeArena) -> Self {
        Self { ptr, _arena: arena }
    }

    /// Gets immutable reference to the value
    #[must_use]
    pub fn get(&self) -> &T {
        // SAFETY: Converting NonNull<T> to &T.
        // - ptr was created from arena allocation (valid, aligned, initialized)
        // - Lifetime 'a is bound to the arena, ensuring ptr stays valid
        // - Arena never deallocates memory until reset/drop
        // - T: Sync bound ensures shared access is safe
        unsafe { self.ptr.as_ref() }
    }
}

impl<T: ?Sized + Sync> std::ops::Deref for ThreadSafeArenaRef<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.get()
    }
}

// SAFETY: ThreadSafeArenaRef<'a, T> is Send if T is Sync because:
// - ptr: NonNull<T> is Send (raw pointer to T)
// - _arena: &'a ThreadSafeArena is Send (immutable reference to Sync type)
// - T: Sync means &T can be sent to another thread safely
// - The reference is immutable, so no exclusive access issues
unsafe impl<T: ?Sized + Sync> Send for ThreadSafeArenaRef<'_, T> {}

// SAFETY: ThreadSafeArenaRef<'a, T> is Sync if T is Sync because:
// - ThreadSafeArenaRef only provides immutable access via Deref
// - T: Sync means multiple threads can share &T safely
// - Arena is Sync, ensuring underlying memory is properly synchronized
// - No interior mutability in ThreadSafeArenaRef itself
unsafe impl<T: ?Sized + Sync> Sync for ThreadSafeArenaRef<'_, T> {}

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
        let config = ArenaConfig::default()
            .with_initial_size(128)
            .with_growth_factor(2.0)
            .with_stats(true);
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
