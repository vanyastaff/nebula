//! Main pool allocator implementation
//!
//! # Safety
//!
//! This module implements a thread-safe pool allocator using lock-free free list:
//! - Fixed-size blocks organized in a singly-linked free list
//! - Atomic head pointer with CAS for thread-safe allocation/deallocation
//! - `SyncUnsafeCell` wrapper for interior mutability of memory buffer
//! - Free blocks store next pointer in first bytes (intrusive list)
//!
//! ## Invariants
//!
//! - All blocks are properly aligned to `block_align`
//! - Free list contains only valid, unallocated blocks
//! - Atomic CAS prevents double-allocation
//! - Block pointers validated on deallocation (bounds + alignment)
//! - `free_count` tracks free blocks for O(1) queries

use core::alloc::Layout;
use core::cell::UnsafeCell;
use core::ptr::{self, NonNull};
use core::sync::atomic::{AtomicPtr, AtomicU32, AtomicUsize, Ordering};

use super::{PoolConfig, PoolStats};
use crate::allocator::{
    AllocError, AllocResult, Allocator, AllocatorStats, MemoryUsage, Resettable, StatisticsProvider,
};
use crate::utils::{Backoff, align_up, atomic_max, is_power_of_two};

/// Thread-safe wrapper for memory buffer with interior mutability
#[repr(transparent)]
struct SyncUnsafeCell<T: ?Sized>(UnsafeCell<T>);

// SAFETY: SyncUnsafeCell<T> is Sync even though UnsafeCell<T> is not.
// - All access to memory buffer goes through atomic free list (CAS)
// - Allocated blocks are exclusively owned by allocator
// - Free blocks only accessed via free list pointer chase
// - AcqRel ordering synchronizes free list updates between threads
// - No overlapping access: allocated blocks disjoint from free list
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

/// Pool allocator for fixed-size blocks
///
/// This allocator manages a pool of equally-sized memory blocks using a
/// lock-free free list. All allocations must be of the same size and alignment
/// as specified during pool creation.
///
/// # Memory Layout
/// ```text
/// [Block0][Block1][Block2][Block3]...[BlockN]
///    ↓       ↓       ↓       ↓           ↓
/// [free] → [free] → [used] → [free] → [used] → null
/// ```
///
/// Free blocks are linked together in a singly-linked list.
pub struct PoolAllocator {
    /// Owned memory buffer containing all blocks with interior mutability
    memory: Box<SyncUnsafeCell<[u8]>>,

    /// Size of each individual block
    block_size: usize,

    /// Alignment requirement for blocks
    block_align: usize,

    /// Total number of blocks in the pool
    block_count: usize,

    /// Total capacity for convenience
    capacity: usize,

    /// Head of the free list (atomic for thread safety)
    free_head: AtomicPtr<FreeBlock>,

    /// Count of free blocks (atomic, for safe concurrent observation)
    free_count: AtomicUsize,

    /// Start address of the memory region
    start_addr: usize,

    /// End address of the memory region
    end_addr: usize,

    /// Configuration
    config: PoolConfig,

    /// Statistics (optional, only tracked if enabled)
    total_allocs: AtomicU32,
    total_deallocs: AtomicU32,
    peak_usage: AtomicUsize,
}

/// Node in the free list
///
/// When a block is free, the first bytes of the block are used to store
/// a pointer to the next free block, forming a linked list.
#[repr(C)]
struct FreeBlock {
    next: *mut FreeBlock,
}

impl PoolAllocator {
    /// Creates a new pool allocator with custom configuration
    ///
    /// # Parameters
    /// - `block_size`: Size of each block in bytes (must be >= `size_of::`<*mut u8>())
    /// - `block_align`: Alignment requirement for blocks (must be power of 2)
    /// - `block_count`: Number of blocks to allocate in the pool
    /// - `config`: Configuration for the allocator
    ///
    /// # Errors
    /// Returns an error if:
    /// - `block_size` is too small to hold a pointer
    /// - `block_align` is not a power of 2
    /// - Memory allocation fails
    pub fn with_config(
        block_size: usize,
        block_align: usize,
        block_count: usize,
        config: PoolConfig,
    ) -> AllocResult<Self> {
        // Validate parameters
        if block_size < core::mem::size_of::<*mut u8>() {
            return Err(AllocError::invalid_layout("block size too small"));
        }

        if !is_power_of_two(block_align) {
            return Err(AllocError::invalid_alignment(block_align));
        }

        if block_count == 0 {
            return Err(AllocError::invalid_layout("invalid layout"));
        }

        // Calculate total memory needed
        let aligned_block_size = align_up(block_size, block_align);
        let total_size = aligned_block_size
            .checked_mul(block_count)
            .ok_or_else(|| AllocError::size_overflow("block size calculation"))?;

        // Allocate memory buffer
        let mut vec = vec![0u8; total_size];

        // Fill with alloc pattern if debugging
        if let Some(pattern) = config.alloc_pattern {
            vec.fill(pattern);
        }

        // Wrap in SyncUnsafeCell for interior mutability
        let boxed_slice = vec.into_boxed_slice();
        let len = boxed_slice.len();
        let ptr = Box::into_raw(boxed_slice).cast::<u8>();
        // SAFETY: Transmuting Box<[u8]> to Box<SyncUnsafeCell<[u8]>>.
        // - SyncUnsafeCell is repr(transparent), identical layout to inner type
        // - ptr is a valid Box<[u8]> pointer from Box::into_raw
        // - Length preserved (len from original boxed_slice)
        // - Box ownership transferred correctly (from_raw after into_raw)
        // - Memory remains valid (same allocation, different type wrapper)
        let memory: Box<SyncUnsafeCell<[u8]>> = unsafe {
            Box::from_raw(core::ptr::slice_from_raw_parts_mut(ptr, len) as *mut SyncUnsafeCell<[u8]>)
        };

        // SAFETY: Getting pointer from freshly created SyncUnsafeCell.
        // - memory.get() returns *mut [u8] to valid allocation
        // - Dereferencing to call as_ptr() on the slice
        // - Result is a valid pointer to start of buffer
        let start_addr = unsafe { (*memory.get()).as_ptr() as usize };
        let end_addr = start_addr + total_size;

        let mut allocator = Self {
            memory,
            block_size: aligned_block_size,
            block_align,
            block_count,
            capacity: total_size,
            free_head: AtomicPtr::new(ptr::null_mut()),
            free_count: AtomicUsize::new(0),
            start_addr,
            end_addr,
            config,
            total_allocs: AtomicU32::new(0),
            total_deallocs: AtomicU32::new(0),
            peak_usage: AtomicUsize::new(0),
        };

        // Initialize the free list
        allocator.initialize_free_list();

        Ok(allocator)
    }

    /// Creates a new pool allocator with default configuration
    pub fn new(block_size: usize, block_align: usize, block_count: usize) -> AllocResult<Self> {
        Self::with_config(block_size, block_align, block_count, PoolConfig::default())
    }

    /// Creates a pool allocator for a specific type
    ///
    /// This is a convenience method that automatically determines the
    /// appropriate block size and alignment for the given type.
    pub fn for_type<T>(block_count: usize) -> AllocResult<Self> {
        let layout = Layout::new::<T>();
        // Ensure minimum size for free list pointer
        let min_size = core::mem::size_of::<*mut u8>();
        let actual_size = core::cmp::max(layout.size(), min_size);
        Self::new(actual_size, layout.align(), block_count)
    }

    /// Creates a pool allocator from a layout
    pub fn for_layout(layout: Layout, block_count: usize) -> AllocResult<Self> {
        Self::new(layout.size(), layout.align(), block_count)
    }

    /// Creates a pool allocator with production config - optimized for performance
    pub fn production(
        block_size: usize,
        block_align: usize,
        block_count: usize,
    ) -> AllocResult<Self> {
        Self::with_config(
            block_size,
            block_align,
            block_count,
            PoolConfig::production(),
        )
    }

    /// Creates a pool allocator with debug config - optimized for debugging
    pub fn debug(block_size: usize, block_align: usize, block_count: usize) -> AllocResult<Self> {
        Self::with_config(block_size, block_align, block_count, PoolConfig::debug())
    }

    /// Creates a pool allocator with performance config - minimal overhead
    pub fn performance(
        block_size: usize,
        block_align: usize,
        block_count: usize,
    ) -> AllocResult<Self> {
        Self::with_config(
            block_size,
            block_align,
            block_count,
            PoolConfig::performance(),
        )
    }

    /// Creates a production pool for a specific type
    pub fn production_for_type<T>(block_count: usize) -> AllocResult<Self> {
        let layout = Layout::new::<T>();
        let min_size = core::mem::size_of::<*mut u8>();
        let actual_size = core::cmp::max(layout.size(), min_size);
        Self::production(actual_size, layout.align(), block_count)
    }

    /// Creates a debug pool for a specific type
    pub fn debug_for_type<T>(block_count: usize) -> AllocResult<Self> {
        let layout = Layout::new::<T>();
        let min_size = core::mem::size_of::<*mut u8>();
        let actual_size = core::cmp::max(layout.size(), min_size);
        Self::debug(actual_size, layout.align(), block_count)
    }

    /// Creates a tiny pool (16 blocks) - for testing or minimal use
    pub fn tiny<T>() -> AllocResult<Self> {
        Self::for_type::<T>(16)
    }

    /// Creates a small pool (64 blocks) - for common use
    pub fn small<T>() -> AllocResult<Self> {
        Self::for_type::<T>(64)
    }

    /// Creates a medium pool (256 blocks) - for standard applications
    pub fn medium<T>() -> AllocResult<Self> {
        Self::for_type::<T>(256)
    }

    /// Creates a large pool (1024 blocks) - for heavy workloads
    pub fn large<T>() -> AllocResult<Self> {
        Self::for_type::<T>(1024)
    }

    /// Returns the size of each block
    pub fn block_size(&self) -> usize {
        self.block_size
    }

    /// Returns the alignment of each block
    pub fn block_align(&self) -> usize {
        self.block_align
    }

    /// Returns the total number of blocks in the pool
    pub fn block_count(&self) -> usize {
        self.block_count
    }

    /// Returns the total capacity in bytes
    pub fn capacity(&self) -> usize {
        self.block_size * self.block_count
    }

    /// Returns the number of allocated blocks
    pub fn allocated_blocks(&self) -> usize {
        self.block_count - self.free_blocks()
    }

    /// Returns the number of free blocks (atomic estimate, exact in absence of races)
    pub fn free_blocks(&self) -> usize {
        self.free_count.load(Ordering::Relaxed)
    }

    /// Checks if the pool is full (no free blocks)
    pub fn is_full(&self) -> bool {
        self.free_head.load(Ordering::Acquire).is_null()
    }

    /// Checks if the pool is empty (all blocks free)
    pub fn is_empty(&self) -> bool {
        self.allocated_blocks() == 0
    }

    /// Checks if a pointer belongs to this pool
    pub fn contains(&self, ptr: *const u8) -> bool {
        let addr = ptr as usize;
        addr >= self.start_addr && addr < self.end_addr
    }

    /// Initializes the free list by linking all blocks together
    fn initialize_free_list(&mut self) {
        let mut prev_block: *mut FreeBlock = ptr::null_mut();

        // Link all blocks together in reverse order
        for i in (0..self.block_count).rev() {
            let block_addr = self.start_addr + (i * self.block_size);

            // Ensure proper block alignment
            debug_assert_eq!(block_addr % self.block_align, 0);

            let block = block_addr as *mut FreeBlock;

            // SAFETY: Writing next pointer to free block during initialization.
            // - block_addr is within [start_addr, end_addr) bounds
            // - block_addr is properly aligned to block_align (assert above)
            // - Block is at least size_of::<*mut u8>() bytes (validated in with_config)
            // - This is the initial setup (&mut self), no concurrent access yet
            // - prev_block is either null or another valid block from this loop
            unsafe {
                (*block).next = prev_block;
            }

            prev_block = block;
        }

        // Set the head to point to the last block (first in the free list)
        self.free_head.store(prev_block, Ordering::Release);
        // Initialize the free block counter
        self.free_count.store(self.block_count, Ordering::Relaxed);
    }

    /// Attempts to allocate a block from the free list
    fn try_allocate_block(&self) -> Option<NonNull<u8>> {
        let mut backoff = if self.config.use_backoff {
            Some(Backoff::new())
        } else {
            None
        };
        let mut attempts = 0;

        loop {
            let head = self.free_head.load(Ordering::Acquire);

            if head.is_null() {
                // No free blocks available
                return None;
            }

            // Check retry limit
            if attempts >= self.config.max_retries {
                return None;
            }

            // SAFETY: Reading next pointer from free list head.
            // - head is non-null (checked above)
            // - head points to a valid FreeBlock (from previous successful allocation or init)
            // - Acquire ordering synchronizes with Release store in deallocate_block
            // - next pointer is valid (null or pointer to another free block)
            let next = unsafe { (*head).next };

            // Try to atomically update the head to point to the next block
            if let Ok(_) = self.free_head.compare_exchange_weak(
                head,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                // Successfully removed the block from free list
                self.free_count.fetch_sub(1, Ordering::Relaxed);

                // Update statistics
                if self.config.track_stats {
                    self.total_allocs.fetch_add(1, Ordering::Relaxed);
                    let current_used = self.used_memory();
                    atomic_max(&self.peak_usage, current_used);
                }

                // Convert pointer to NonNull with explicit check
                // head is non-null (checked at loop start), but use explicit check for safety
                return NonNull::new(head.cast::<u8>());
            } else {
                // Another thread modified the list, backoff and retry
                attempts += 1;
                if let Some(ref mut b) = backoff {
                    b.spin();
                }
                continue;
            }
        }
    }

    /// Returns a block to the free list
    fn deallocate_block(&self, ptr: NonNull<u8>) -> bool {
        // Validate that the pointer belongs to this pool
        if !self.contains(ptr.as_ptr()) {
            return false;
        }

        // Validate alignment and bounds
        let addr = ptr.as_ptr() as usize;
        if !(addr - self.start_addr).is_multiple_of(self.block_size) {
            return false; // Not aligned to block boundary
        }

        // Fill with dealloc pattern if debugging
        if let Some(pattern) = self.config.dealloc_pattern {
            // SAFETY: Writing debug pattern to block being deallocated.
            // - ptr is validated above (belongs to pool, aligned to block boundary)
            // - Block is currently allocated (being returned to free list)
            // - write_bytes fills entire block with pattern
            // - After this, block will be added back to free list
            unsafe {
                ptr::write_bytes(ptr.as_ptr(), pattern, self.block_size);
            }
        }

        let block = ptr.as_ptr().cast::<FreeBlock>();
        let mut backoff = if self.config.use_backoff {
            Some(Backoff::new())
        } else {
            None
        };

        loop {
            let head = self.free_head.load(Ordering::Acquire);

            // SAFETY: Writing next pointer to block being added to free list.
            // - block is a valid pointer (validated above: belongs to pool, aligned)
            // - Block is at least size_of::<*mut u8>() bytes (pool invariant)
            // - head is either null or pointer to current free list head
            // - This write happens before CAS, so no other thread sees inconsistent state
            unsafe {
                (*block).next = head;
            }

            // Try to atomically set this block as the new head
            if self
                .free_head
                .compare_exchange_weak(head, block, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                self.free_count.fetch_add(1, Ordering::Relaxed);

                // Update statistics
                if self.config.track_stats {
                    self.total_deallocs.fetch_add(1, Ordering::Relaxed);
                }

                return true;
            }
            // Retry with backoff
            if let Some(ref mut b) = backoff {
                b.spin();
            }
            continue;
        }
    }

    /// Get statistics (if tracking is enabled)
    pub fn stats(&self) -> Option<PoolStats> {
        if !self.config.track_stats {
            return None;
        }

        Some(PoolStats {
            total_allocs: self.total_allocs.load(Ordering::Relaxed),
            total_deallocs: self.total_deallocs.load(Ordering::Relaxed),
            peak_usage: self.peak_usage.load(Ordering::Relaxed),
            current_usage: self.used_memory(),
            block_size: self.block_size,
            block_count: self.block_count,
            free_blocks: self.free_blocks(),
        })
    }
}

// SAFETY: PoolAllocator implements Allocator for fixed-size blocks.
// - All blocks have same size (block_size) and alignment (block_align)
// - Free list managed via atomic CAS (thread-safe)
// - deallocate validates pointers (bounds + alignment)
// - reallocate can reuse block if new size fits
unsafe impl Allocator for PoolAllocator {
    /// # Safety
    ///
    /// Caller must ensure:
    /// - `layout.size()` <= `block_size` and `layout.align()` <= `block_align`
    /// - Returned pointer not used after allocator reset/drop
    /// - Pool allocator only supports fixed-size allocations
    unsafe fn allocate(&self, layout: Layout) -> AllocResult<NonNull<[u8]>> {
        // Check if the requested layout matches our pool configuration
        if layout.size() > self.block_size || layout.align() > self.block_align {
            return Err(AllocError::invalid_layout(
                "layout exceeds pool configuration",
            ));
        }

        // Handle zero-sized allocations
        if layout.size() == 0 {
            let ptr = NonNull::<u8>::dangling();
            return Ok(NonNull::slice_from_raw_parts(ptr, 0));
        }

        // Try to allocate a block
        if let Some(ptr) = self.try_allocate_block() {
            Ok(NonNull::slice_from_raw_parts(ptr, layout.size()))
        } else {
            Err(AllocError::allocation_failed(layout.size(), layout.align()))
        }
    }

    /// # Safety
    ///
    /// Caller must ensure:
    /// - `ptr` was allocated by this pool with matching `layout`
    /// - `ptr` is currently allocated (not already deallocated)
    /// - `layout` matches original allocation
    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        // Handle zero-sized deallocations
        if layout.size() == 0 {
            return;
        }

        // Return the block to the free list
        self.deallocate_block(ptr);
    }

    /// # Safety
    ///
    /// Caller must ensure:
    /// - `ptr` was allocated by this pool with `old_layout`
    /// - `old_layout` matches original allocation
    /// - `new_layout` fits within `block_size` and `block_align`
    /// - On failure, `ptr` remains valid with `old_layout`
    unsafe fn reallocate(
        &self,
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> AllocResult<NonNull<[u8]>> {
        // Pool allocator can only handle allocations of the configured size
        if new_layout.size() > self.block_size || new_layout.align() > self.block_align {
            return Err(AllocError::invalid_layout(
                "new layout exceeds pool configuration",
            ));
        }

        // If the new size fits within the same block, we can reuse it
        if new_layout.size() <= self.block_size && new_layout.align() <= self.block_align {
            // Just return the same pointer with the new size
            return Ok(NonNull::slice_from_raw_parts(ptr, new_layout.size()));
        }

        // This should not be reached given the checks above, but for completeness:
        // Fall back to allocate + copy + deallocate
        // SAFETY: new_layout is validated above (fits in block_size/align).
        let new_ptr = unsafe { self.allocate(new_layout)? };

        let copy_size = core::cmp::min(old_layout.size(), new_layout.size());
        if copy_size > 0 {
            // SAFETY: Copying data from old to new block.
            // - ptr is valid for old_layout.size() bytes (caller's contract)
            // - new_ptr is valid for new_layout.size() bytes (just allocated)
            // - copy_size is min of both sizes (no overflow)
            // - Regions don't overlap (different blocks from pool)
            unsafe {
                ptr::copy_nonoverlapping(ptr.as_ptr(), new_ptr.as_ptr().cast::<u8>(), copy_size);
            }
        }

        // SAFETY: ptr and old_layout match original allocation (caller's contract).
        unsafe { self.deallocate(ptr, old_layout) };
        Ok(new_ptr)
    }
}

impl MemoryUsage for PoolAllocator {
    fn used_memory(&self) -> usize {
        self.allocated_blocks() * self.block_size
    }

    fn available_memory(&self) -> Option<usize> {
        Some(self.free_blocks() * self.block_size)
    }

    fn total_memory(&self) -> Option<usize> {
        Some(self.capacity())
    }
}

impl Resettable for PoolAllocator {
    /// # Safety
    ///
    /// Caller must ensure:
    /// - No outstanding references to allocated blocks exist
    /// - All allocations from this pool are no longer in use
    /// - Reset is properly synchronized if pool is shared across threads
    unsafe fn reset(&self) {
        // Reset the free list to include all blocks
        let mut prev_block: *mut FreeBlock = ptr::null_mut();

        // Re-link all blocks together
        for i in (0..self.block_count).rev() {
            let block_addr = self.start_addr + (i * self.block_size);
            let block = block_addr as *mut FreeBlock;

            // SAFETY: Reinitializing free list during reset.
            // - block_addr is within [start_addr, end_addr) bounds
            // - Caller ensures no outstanding allocations (contract above)
            // - Same logic as initialize_free_list (validated there)
            // - prev_block is either null or another valid block from this loop
            unsafe {
                (*block).next = prev_block;
            }

            prev_block = block;
        }

        // Set the head to point to the first block
        self.free_head.store(prev_block, Ordering::Release);
        self.free_count.store(self.block_count, Ordering::Relaxed);

        // Reset statistics
        if self.config.track_stats {
            self.total_allocs.store(0, Ordering::Relaxed);
            self.total_deallocs.store(0, Ordering::Relaxed);
            self.peak_usage.store(0, Ordering::Relaxed);
        }
    }

    fn can_reset(&self) -> bool {
        true
    }
}

impl StatisticsProvider for PoolAllocator {
    fn statistics(&self) -> AllocatorStats {
        AllocatorStats {
            allocated_bytes: self.used_memory(),
            peak_allocated_bytes: if self.config.track_stats {
                self.peak_usage.load(Ordering::Relaxed)
            } else {
                self.used_memory()
            },
            allocation_count: self.total_allocs.load(Ordering::Relaxed) as usize,
            deallocation_count: self.total_deallocs.load(Ordering::Relaxed) as usize,
            reallocation_count: 0,
            failed_allocations: 0,
            total_bytes_allocated: self.total_allocs.load(Ordering::Relaxed) as usize
                * self.block_size,
            total_bytes_deallocated: self.total_deallocs.load(Ordering::Relaxed) as usize
                * self.block_size,
        }
    }

    fn reset_statistics(&self) {
        if self.config.track_stats {
            self.total_allocs.store(0, Ordering::Relaxed);
            self.total_deallocs.store(0, Ordering::Relaxed);
            self.peak_usage.store(0, Ordering::Relaxed);
        }
    }

    fn statistics_enabled(&self) -> bool {
        self.config.track_stats
    }
}

// SAFETY: PoolAllocator is Send because:
// - memory: Box<SyncUnsafeCell<[u8]>> is Send (SyncUnsafeCell is Send)
// - All atomics (free_head, free_count, total_allocs, etc.) are Send
// - Config and sizes are Copy/primitive types
// - All owned data can be safely transferred to another thread
unsafe impl Send for PoolAllocator {}

// SAFETY: PoolAllocator is Sync because:
// - All allocations go through atomic CAS operations (thread-safe)
// - Free list managed via AtomicPtr with proper ordering (AcqRel/Acquire)
// - Memory buffer wrapped in SyncUnsafeCell (proven Sync above)
// - No shared mutable state outside of atomics
// - Pointer validation (bounds + alignment) prevents double-free
unsafe impl Sync for PoolAllocator {}
