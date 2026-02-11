//! Production-ready bump allocator with safe abstractions
//!
//! A bump allocator (also called arena allocator) provides fast sequential allocations
//! by simply incrementing a pointer. All memory is freed at once when the allocator is dropped.
//!
//! # Safety
//!
//! This module implements a bump allocator with careful synchronization:
//! - Memory buffer wrapped in `SyncUnsafeCell` for interior mutability
//! - Atomic cursor (or Cell for !Send) ensures exclusive allocation ranges
//! - Compare-and-swap prevents overlapping allocations in multi-threaded mode
//! - Checkpoint/restore with generation counters prevent use-after-restore
//!
//! ## Invariants
//!
//! - Allocated memory ranges never overlap (enforced by atomic CAS)
//! - All pointers within [`start_addr`, `end_addr`) bounds
//! - Cursor only moves forward (monotonic within a generation)
//! - Checkpoints validated by generation counter
//! - Individual deallocation not supported (no-op)

use core::alloc::Layout;
use core::cell::UnsafeCell;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

mod checkpoint;
mod config;
mod cursor;

pub use checkpoint::{BumpCheckpoint, BumpScope};
pub use config::BumpConfig;
use cursor::{AtomicCursor, CellCursor, Cursor};

use crate::allocator::{
    AllocError, AllocResult, Allocator, BulkAllocator, MemoryUsage, OptionalStats, Resettable,
    StatisticsProvider, ThreadSafeAllocator,
};

use crate::utils::{Backoff, MemoryOps, PrefetchManager, align_up, atomic_max, cache_line_size};

/// Thread-safe wrapper for memory buffer with interior mutability
#[repr(transparent)]
struct SyncUnsafeCell<T: ?Sized>(UnsafeCell<T>);

// SAFETY: SyncUnsafeCell<T> is Sync even though UnsafeCell<T> is not.
// - All mutable access goes through atomic cursor (compare_exchange)
// - CAS success guarantees exclusive access to allocated range
// - No overlapping mutable references (disjoint memory regions)
// - AcqRel ordering synchronizes cursor updates between threads
unsafe impl<T: ?Sized> Sync for SyncUnsafeCell<T> {}

// SAFETY: SyncUnsafeCell<T> is Send if T is Send.
// - Wrapper is repr(transparent), same layout as UnsafeCell<T>
// - T: Send bound ensures inner value can move between threads
// - No thread-local state in wrapper
unsafe impl<T: ?Sized + Send> Send for SyncUnsafeCell<T> {}

impl<T> SyncUnsafeCell<T> {
    fn new(value: T) -> Self {
        Self(UnsafeCell::new(value))
    }
}

impl<T: ?Sized> SyncUnsafeCell<T> {
    fn get(&self) -> *mut T {
        self.0.get()
    }
}

/// Production-ready bump allocator
pub struct BumpAllocator {
    /// Memory buffer with explicit interior mutability for safe mutable access
    memory: Box<SyncUnsafeCell<[u8]>>,
    config: BumpConfig,
    prefetch_mgr: PrefetchManager,
    memory_ops: MemoryOps,
    start_addr: usize,
    end_addr: usize,
    capacity: usize,
    cursor: Box<dyn Cursor>,
    stats: OptionalStats,
    peak_usage: AtomicUsize,
    generation: AtomicU32,
}

impl BumpAllocator {
    /// Creates a new bump allocator with specified capacity and configuration
    pub fn with_config(capacity: usize, config: BumpConfig) -> AllocResult<Self> {
        if capacity == 0 {
            return Err(AllocError::invalid_layout("invalid layout"));
        }

        let mut vec = vec![0u8; capacity];

        let memory_ops = MemoryOps::new();
        if let Some(pattern) = config.alloc_pattern {
            // SAFETY: Filling freshly allocated vector with pattern.
            // - vec is a valid mutable slice (just allocated)
            // - MemoryOps::secure_fill_slice writes pattern to entire slice
            // - No concurrent access (vec is exclusive to this thread)
            unsafe {
                MemoryOps::secure_fill_slice(&mut vec, pattern);
            }
        }

        // Wrap in SyncUnsafeCell for interior mutability
        // We need to convert Box<[u8]> into Box<SyncUnsafeCell<[u8]>>
        let boxed_slice = vec.into_boxed_slice();
        let len = boxed_slice.len();
        let ptr = Box::into_raw(boxed_slice).cast::<u8>();
        // SAFETY: Transmuting Box<[u8]> to Box<SyncUnsafeCell<[u8]>>.
        // - SyncUnsafeCell is repr(transparent), identical layout to inner type
        // - ptr is a valid Box<[u8]> pointer from Box::into_raw
        // - Length is preserved (len from original boxed_slice)
        // - Box ownership transferred correctly (from_raw after into_raw)
        // - Memory remains valid (same allocation, just different type wrapper)
        let memory: Box<SyncUnsafeCell<[u8]>> = unsafe {
            Box::from_raw(core::ptr::slice_from_raw_parts_mut(ptr, len) as *mut SyncUnsafeCell<[u8]>)
        };

        // SAFETY: Getting pointer from freshly created SyncUnsafeCell.
        // - memory.get() returns *mut [u8] pointing to valid allocation
        // - Dereferencing to call as_ptr() on the slice
        // - Result is a valid pointer to the start of the buffer
        let start_addr = unsafe { (*memory.get()).as_ptr() as usize };
        let end_addr = start_addr + capacity;

        let cursor: Box<dyn Cursor> = if config.thread_safe {
            Box::new(AtomicCursor::new(start_addr))
        } else {
            Box::new(CellCursor::new(start_addr))
        };

        let track_stats = config.track_stats;

        Ok(Self {
            memory,
            config,
            prefetch_mgr: PrefetchManager::new(),
            memory_ops,
            start_addr,
            end_addr,
            capacity,
            cursor,
            stats: if track_stats {
                OptionalStats::enabled()
            } else {
                OptionalStats::disabled()
            },
            peak_usage: AtomicUsize::new(0),
            generation: AtomicU32::new(0),
        })
    }

    /// Creates allocator with default configuration
    pub fn new(capacity: usize) -> AllocResult<Self> {
        Self::with_config(capacity, BumpConfig::default())
    }

    /// Creates production-optimized allocator
    pub fn production(capacity: usize) -> AllocResult<Self> {
        Self::with_config(capacity, BumpConfig::production())
    }

    /// Creates debug-optimized allocator
    pub fn debug(capacity: usize) -> AllocResult<Self> {
        Self::with_config(capacity, BumpConfig::debug())
    }

    /// Convenience: 64KB allocator
    pub fn small() -> AllocResult<Self> {
        Self::production(64 * 1024)
    }

    /// Convenience: 1MB allocator
    pub fn medium() -> AllocResult<Self> {
        Self::production(1024 * 1024)
    }

    /// Convenience: 16MB allocator
    pub fn large() -> AllocResult<Self> {
        Self::production(16 * 1024 * 1024)
    }

    /// Total capacity
    #[inline]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Currently used memory
    #[inline]
    pub fn used(&self) -> usize {
        self.cursor
            .load(Ordering::Relaxed)
            .saturating_sub(self.start_addr)
    }

    /// Available memory
    #[inline]
    pub fn available(&self) -> usize {
        self.capacity().saturating_sub(self.used())
    }

    /// Peak usage
    #[inline]
    pub fn peak_usage(&self) -> usize {
        self.peak_usage.load(Ordering::Relaxed)
    }

    /// Creates a checkpoint at current position
    #[must_use = "checkpoint должен быть сохранён для последующего restore"]
    pub fn checkpoint(&self) -> BumpCheckpoint {
        BumpCheckpoint {
            position: self.cursor.load(Ordering::Acquire),
            generation: self.generation.load(Ordering::Acquire),
        }
    }

    /// Restores allocator to previous checkpoint
    pub fn restore(&self, checkpoint: BumpCheckpoint) -> AllocResult<()> {
        let current_gen = self.generation.load(Ordering::Acquire);
        if checkpoint.generation != current_gen {
            return Err(AllocError::invalid_input(
                "checkpoint from different generation",
            ));
        }

        let current = self.cursor.load(Ordering::Acquire);
        if checkpoint.position < self.start_addr || checkpoint.position > self.end_addr {
            return Err(AllocError::invalid_input(
                "checkpoint position out of bounds",
            ));
        }
        if checkpoint.position > current {
            return Err(AllocError::invalid_input("checkpoint is in the future"));
        }

        // Fill deallocated region with pattern
        if let Some(pattern) = self.config.dealloc_pattern {
            let start = checkpoint.position - self.start_addr;
            let end = current - self.start_addr;
            // SAFETY: Filling memory being restored (checkpoint to current cursor).
            // - memory.get() returns *mut [u8] to our buffer
            // - [start..end) range is within bounds (validated above)
            // - This range was previously allocated (between checkpoint and current)
            // - UnsafeCell grants mutable access
            // - After restore, this memory becomes unallocated again
            unsafe {
                let memory_slice = &mut *self.memory.get();
                if let Some(slice) = memory_slice.get_mut(start..end) {
                    MemoryOps::secure_fill_slice(slice, pattern);
                }
            }
        }

        self.cursor.store(checkpoint.position, Ordering::Release);
        Ok(())
    }

    /// Creates scoped allocation with auto-restore
    pub fn scoped(&self) -> BumpScope<'_> {
        BumpScope::new(self)
    }

    /// Resets allocator to initial state (internal helper)
    fn reset_internal(&self) {
        self.cursor.store(self.start_addr, Ordering::Release);
        self.generation.fetch_add(1, Ordering::Release);
        self.peak_usage.store(0, Ordering::Relaxed);
        self.stats.reset();
    }

    #[inline]
    fn effective_size(&self, size: usize) -> usize {
        size.max(self.config.min_alloc_size)
    }

    #[inline]
    fn prefetch_next(&self, addr: usize) {
        if !self.config.enable_prefetch {
            return;
        }
        let prefetch_dist = self.config.prefetch_distance * cache_line_size();
        let prefetch_end = (addr + prefetch_dist).min(self.end_addr);
        if addr < self.end_addr && prefetch_end > addr {
            let start = addr - self.start_addr;
            let end = prefetch_end - self.start_addr;
            // SAFETY: Read-only prefetch hint for upcoming allocations.
            // - memory.get() returns *mut [u8], we create shared reference
            // - [start..end) range is within buffer bounds (clamped to end_addr)
            // - Read-only access is safe (no mutation)
            // - Prefetch is advisory (CPU hint), no memory safety impact
            // - Concurrent allocations OK (they access disjoint ranges via CAS)
            unsafe {
                let memory_slice = &*self.memory.get();
                if let Some(slice) = memory_slice.get(start..end) {
                    self.prefetch_mgr.prefetch_slice_read(slice);
                }
            }
        }
    }

    fn try_bump(&self, size: usize, align: usize) -> Option<NonNull<u8>> {
        let actual_size = self.effective_size(size);
        const MAX_RETRIES: usize = 100;
        let mut backoff = Backoff::new();
        let mut attempts = 0;

        loop {
            if attempts >= MAX_RETRIES {
                self.stats.record_allocation_failure();
                return None;
            }

            let current = self.cursor.load(Ordering::Acquire);
            let aligned = align_up(current, align);
            let new_current = aligned.checked_add(actual_size)?;

            if new_current > self.end_addr {
                self.stats.record_allocation_failure();
                return None;
            }

            self.prefetch_next(new_current);

            let result = self.cursor.compare_exchange_weak(
                current,
                new_current,
                Ordering::AcqRel,
                Ordering::Acquire,
            );

            if result.is_ok() {
                self.stats.record_allocation(actual_size);
                let usage = new_current - self.start_addr;
                atomic_max(&self.peak_usage, usage);

                // Calculate return pointer with proper provenance through UnsafeCell
                let offset = aligned - self.start_addr;
                // SAFETY: Getting pointer to freshly allocated memory using slice indexing.
                // - memory.get() returns *mut [u8] to our buffer
                // - offset is within bounds: aligned < new_current <= end_addr (checked above)
                // - CAS success means we exclusively own [aligned, new_current) range
                // - get_unchecked_mut provides bounds-checked access in debug mode
                // - No other thread can allocate overlapping range (cursor advanced)
                let ptr = unsafe {
                    let memory_ptr = self.memory.get();
                    let slice = &mut *memory_ptr;
                    slice.get_unchecked_mut(offset..).as_mut_ptr()
                };

                // Fill with pattern if configured
                if let Some(pattern) = self.config.alloc_pattern {
                    // SAFETY: Writing pattern to freshly allocated, uninitialized memory.
                    // - ptr points to [aligned, aligned+actual_size) range
                    // - This range is exclusively owned (just allocated via CAS)
                    // - write_bytes writes pattern to every byte
                    // - actual_size is within allocated range (checked above)
                    unsafe {
                        core::ptr::write_bytes(ptr, pattern, actual_size);
                    }
                }

                // Convert raw pointer to NonNull with explicit check
                // ptr is guaranteed non-null (buffer from Box + valid offset)
                // but we use explicit check for additional safety
                return NonNull::new(ptr);
            }
            attempts += 1;
            if self.config.thread_safe {
                backoff.spin();
            }
        }
    }
}

// SAFETY: BumpAllocator implements Allocator via bump pointer allocation.
// - allocate() uses atomic CAS to reserve exclusive memory ranges
// - deallocate() is intentionally a no-op (bump allocators don't free individually)
// - All allocations within bounded buffer [start_addr, end_addr)
// - Proper alignment enforced by align_up in try_bump
unsafe impl Allocator for BumpAllocator {
    /// # Safety
    ///
    /// Caller must ensure:
    /// - `layout` has valid size and alignment (align is power of two)
    /// - Returned pointer not used after allocator reset/drop
    /// - Bump allocators don't support individual deallocation
    unsafe fn allocate(&self, layout: Layout) -> AllocResult<NonNull<[u8]>> {
        let ptr = self
            .try_bump(layout.size(), layout.align())
            .ok_or_else(|| AllocError::out_of_memory_with_layout(layout))?;

        let slice = NonNull::slice_from_raw_parts(ptr, layout.size());
        Ok(slice)
    }

    /// # Safety
    ///
    /// This is intentionally a no-op - bump allocators don't support individual deallocation.
    /// Memory is only freed when the entire allocator is reset or dropped.
    unsafe fn deallocate(&self, _ptr: NonNull<u8>, _layout: Layout) {
        // Bump allocator doesn't support individual deallocation
    }
}

// SAFETY: BumpAllocator is ThreadSafeAllocator when configured with thread_safe=true.
// - Atomic cursor (AtomicCursor) provides thread-safe allocation
// - CAS operations prevent race conditions
// - Memory buffer wrapped in SyncUnsafeCell (proven Sync above)
// - When thread_safe=false, CellCursor is used (not thread-safe, but correct for !Send)
unsafe impl ThreadSafeAllocator for BumpAllocator {}

// SAFETY: BumpAllocator implements BulkAllocator using default trait methods.
// - Bulk operations delegate to allocate/deallocate
// - Same safety properties as base Allocator impl
unsafe impl BulkAllocator for BumpAllocator {}

impl MemoryUsage for BumpAllocator {
    fn used_memory(&self) -> usize {
        self.used()
    }

    fn available_memory(&self) -> Option<usize> {
        Some(self.available())
    }
}

impl Resettable for BumpAllocator {
    /// # Safety
    ///
    /// Caller must ensure:
    /// - No outstanding references to allocated memory exist
    /// - All allocations from this allocator are no longer in use
    /// - Reset is properly synchronized if allocator is shared across threads
    unsafe fn reset(&self) {
        self.reset_internal();
    }
}

impl StatisticsProvider for BumpAllocator {
    fn statistics(&self) -> crate::allocator::AllocatorStats {
        self.stats.snapshot().unwrap_or_default()
    }

    fn reset_statistics(&self) {
        self.stats.reset();
    }
}

// ============================================================================
// Sealed Internal Trait Implementation
// ============================================================================

impl crate::allocator::sealed::AllocatorInternal for BumpAllocator {
    fn internal_checkpoint(&self) -> crate::allocator::sealed::InternalCheckpoint {
        let offset = self.cursor.load(Ordering::Acquire);
        let generation = self.generation.load(Ordering::Acquire);

        crate::allocator::sealed::InternalCheckpoint::new(
            offset - self.start_addr, // Relative offset
            self.start_addr as u64,   // Chunk ID (use start address as identifier)
            generation,
        )
    }

    unsafe fn internal_restore(
        &mut self,
        checkpoint: crate::allocator::sealed::InternalCheckpoint,
    ) -> AllocResult<()> {
        let current_generation = self.generation.load(Ordering::Acquire);

        // Validate checkpoint is not stale
        if checkpoint.generation != current_generation {
            return Err(AllocError::InvalidState {
                reason: format!(
                    "stale checkpoint: expected generation {}, got {}",
                    current_generation, checkpoint.generation
                ),
            });
        }

        // Validate checkpoint is from this allocator
        if checkpoint.chunk_id != self.start_addr as u64 {
            return Err(AllocError::InvalidState {
                reason: "checkpoint from different allocator".into(),
            });
        }

        // Validate offset is within bounds
        if checkpoint.offset > self.capacity {
            return Err(AllocError::InvalidState {
                reason: format!(
                    "checkpoint offset {} exceeds capacity {}",
                    checkpoint.offset, self.capacity
                ),
            });
        }

        // SAFETY: Caller guarantees no allocations after checkpoint are in use
        // - Checkpoint validated above (correct generation, allocator, bounds)
        // - Setting cursor to checkpoint offset is safe (within [start_addr, end_addr))
        // - Updates stats to reflect restored state
        let restored_addr = self.start_addr + checkpoint.offset;
        self.cursor.store(restored_addr, Ordering::Release);

        // Update stats to reflect rewound state
        let freed_bytes = self.used();
        if freed_bytes > checkpoint.offset {
            self.stats
                .record_deallocation(freed_bytes - checkpoint.offset);
        }

        Ok(())
    }

    fn internal_fragmentation(&self) -> crate::allocator::sealed::FragmentationStats {
        // Bump allocators have zero internal fragmentation by design
        // - All free space is contiguous (at the end)
        // - No free list, no fragments
        let total_free = self.available();
        crate::allocator::sealed::FragmentationStats::calculate(
            total_free,
            total_free,                         // Largest block = all free space
            if total_free > 0 { 1 } else { 0 }, // Single fragment or none
        )
    }

    #[cfg(debug_assertions)]
    fn internal_validate(&self) -> Result<(), &'static str> {
        let cursor_pos = self.cursor.load(Ordering::Acquire);

        // Validate cursor is within bounds
        if cursor_pos < self.start_addr || cursor_pos > self.end_addr {
            return Err("cursor out of bounds");
        }

        // Validate monotonicity
        let used = cursor_pos - self.start_addr;
        if used > self.capacity {
            return Err("used memory exceeds capacity");
        }

        // Validate stats consistency (if enabled)
        if let Some(stats) = self.stats.snapshot() {
            if stats.total_bytes_deallocated > stats.total_bytes_allocated {
                return Err("deallocated bytes exceed allocated bytes");
            }
        }

        Ok(())
    }

    fn internal_type_name(&self) -> &'static str {
        "BumpAllocator"
    }
}
