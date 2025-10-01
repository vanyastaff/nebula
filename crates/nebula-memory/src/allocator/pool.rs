//! Production-ready pool allocator with safe abstractions
//!
//! A pool allocator pre-allocates a fixed number of equally-sized blocks
//! and manages them through a lock-free free list. This provides O(1) allocation
//! and deallocation for objects of a specific size, making it ideal for
//! scenarios with frequent allocation/deallocation of same-sized objects.
//!
//! # Use Cases
//! - Object pools for game entities, particles, etc.
//! - Network packet buffers
//! - Database record caching
//! - Any scenario with frequent alloc/dealloc of fixed-size objects
//! - Memory-constrained environments where fragmentation is a concern

use core::alloc::Layout;
use core::ptr::{self, NonNull};
use core::sync::atomic::{AtomicPtr, AtomicUsize, AtomicU32, Ordering};

use crate::core::traits::{
    MemoryUsage, Resettable, StatisticsProvider,
};

use super::{
    AllocError, AllocErrorCode, AllocResult, Allocator,
};

// Import safe utilities
use crate::utils::{
    align_up, is_power_of_two,
    Backoff, atomic_max,
};

/// Configuration for pool allocator
#[derive(Debug, Clone)]
pub struct PoolConfig {
    /// Enable statistics tracking
    pub track_stats: bool,

    /// Fill patterns for debugging
    pub alloc_pattern: Option<u8>,
    pub dealloc_pattern: Option<u8>,

    /// Use exponential backoff for CAS retries
    pub use_backoff: bool,

    /// Maximum CAS retry attempts before failing
    pub max_retries: usize,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            track_stats: cfg!(debug_assertions),
            alloc_pattern: if cfg!(debug_assertions) { Some(0xBB) } else { None },
            dealloc_pattern: if cfg!(debug_assertions) { Some(0xDD) } else { None },
            use_backoff: true,
            max_retries: 1000,
        }
    }
}

impl PoolConfig {
    /// Production configuration - optimized for performance
    pub fn production() -> Self {
        Self {
            track_stats: false,
            alloc_pattern: None,
            dealloc_pattern: None,
            use_backoff: true,
            max_retries: 10000,
        }
    }

    /// Debug configuration - optimized for debugging
    pub fn debug() -> Self {
        Self {
            track_stats: true,
            alloc_pattern: Some(0xBB),
            dealloc_pattern: Some(0xDD),
            use_backoff: false,
            max_retries: 100,
        }
    }

    /// Performance configuration - minimal overhead
    pub fn performance() -> Self {
        Self {
            track_stats: false,
            alloc_pattern: None,
            dealloc_pattern: None,
            use_backoff: false,
            max_retries: 100,
        }
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
    /// Owned memory buffer containing all blocks
    memory: Box<[u8]>,

    /// Size of each individual block
    block_size: usize,

    /// Alignment requirement for blocks
    block_align: usize,

    /// Total number of blocks in the pool
    block_count: usize,

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
    /// - `block_size`: Size of each block in bytes (must be >= size_of::<*mut u8>())
    /// - `block_align`: Alignment requirement for blocks (must be power of 2)
    /// - `block_count`: Number of blocks to allocate in the pool
    /// - `config`: Configuration for the allocator
    ///
    /// # Errors
    /// Returns an error if:
    /// - block_size is too small to hold a pointer
    /// - block_align is not a power of 2
    /// - Memory allocation fails
    pub fn with_config(
        block_size: usize,
        block_align: usize,
        block_count: usize,
        config: PoolConfig,
    ) -> AllocResult<Self> {
        // Validate parameters
        if block_size < core::mem::size_of::<*mut u8>() {
            return Err(AllocError::with_layout(
                AllocErrorCode::InvalidLayout,
                Layout::from_size_align(block_size, block_align).unwrap_or(Layout::new::<u8>()),
            ));
        }

        if !is_power_of_two(block_align) {
            return Err(AllocError::with_layout(
                AllocErrorCode::InvalidAlignment,
                Layout::from_size_align(block_size, block_align).unwrap_or(Layout::new::<u8>()),
            ));
        }

        if block_count == 0 {
            return Err(AllocError::invalid_layout());
        }

        // Calculate total memory needed
        let aligned_block_size = align_up(block_size, block_align);
        let total_size = aligned_block_size
            .checked_mul(block_count)
            .ok_or_else(|| AllocError::size_overflow())?;

        // Allocate memory buffer
        let mut memory = vec![0u8; total_size].into_boxed_slice();

        // Fill with alloc pattern if debugging
        if let Some(pattern) = config.alloc_pattern {
            memory.fill(pattern);
        }

        let start_addr = memory.as_ptr() as usize;
        let end_addr = start_addr + total_size;

        let mut allocator = Self {
            memory,
            block_size: aligned_block_size,
            block_align,
            block_count,
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
    pub fn production(block_size: usize, block_align: usize, block_count: usize) -> AllocResult<Self> {
        Self::with_config(block_size, block_align, block_count, PoolConfig::production())
    }

    /// Creates a pool allocator with debug config - optimized for debugging
    pub fn debug(block_size: usize, block_align: usize, block_count: usize) -> AllocResult<Self> {
        Self::with_config(block_size, block_align, block_count, PoolConfig::debug())
    }

    /// Creates a pool allocator with performance config - minimal overhead
    pub fn performance(block_size: usize, block_align: usize, block_count: usize) -> AllocResult<Self> {
        Self::with_config(block_size, block_align, block_count, PoolConfig::performance())
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
        let mut backoff = if self.config.use_backoff { Some(Backoff::new()) } else { None };
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

            // Get the next free block
            let next = unsafe { (*head).next };

            // Try to atomically update the head to point to the next block
            match self.free_head.compare_exchange_weak(
                head,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    // Successfully removed the block from free list
                    self.free_count.fetch_sub(1, Ordering::Relaxed);

                    // Update statistics
                    if self.config.track_stats {
                        self.total_allocs.fetch_add(1, Ordering::Relaxed);
                        let current_used = self.used_memory();
                        atomic_max(&self.peak_usage, current_used);
                    }

                    return Some(unsafe { NonNull::new_unchecked(head as *mut u8) });
                },
                Err(_) => {
                    // Another thread modified the list, backoff and retry
                    attempts += 1;
                    if let Some(ref mut b) = backoff {
                        b.spin();
                    }
                    continue;
                },
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
        if (addr - self.start_addr) % self.block_size != 0 {
            return false; // Not aligned to block boundary
        }

        // Fill with dealloc pattern if debugging
        if let Some(pattern) = self.config.dealloc_pattern {
            unsafe {
                ptr::write_bytes(ptr.as_ptr(), pattern, self.block_size);
            }
        }

        let block = ptr.as_ptr() as *mut FreeBlock;
        let mut backoff = if self.config.use_backoff { Some(Backoff::new()) } else { None };

        loop {
            let head = self.free_head.load(Ordering::Acquire);

            unsafe {
                (*block).next = head;
            }

            // Try to atomically set this block as the new head
            match self.free_head.compare_exchange_weak(
                head,
                block,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    self.free_count.fetch_add(1, Ordering::Relaxed);

                    // Update statistics
                    if self.config.track_stats {
                        self.total_deallocs.fetch_add(1, Ordering::Relaxed);
                    }

                    return true
                },
                Err(_) => {
                    // Retry with backoff
                    if let Some(ref mut b) = backoff {
                        b.spin();
                    }
                    continue;
                },
            }
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

/// Statistics for pool allocator
#[derive(Debug, Clone, Copy)]
pub struct PoolStats {
    /// Total allocations performed
    pub total_allocs: u32,
    /// Total deallocations performed
    pub total_deallocs: u32,
    /// Peak memory usage in bytes
    pub peak_usage: usize,
    /// Current memory usage in bytes
    pub current_usage: usize,
    /// Size of each block
    pub block_size: usize,
    /// Total number of blocks
    pub block_count: usize,
    /// Currently free blocks
    pub free_blocks: usize,
}

unsafe impl Allocator for PoolAllocator {
    unsafe fn allocate(&self, layout: Layout) -> AllocResult<NonNull<[u8]>> {
        // Check if the requested layout matches our pool configuration
        if layout.size() > self.block_size || layout.align() > self.block_align {
            return Err(AllocError::with_layout(AllocErrorCode::InvalidLayout, layout));
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
            Err(AllocError::with_layout(AllocErrorCode::OutOfMemory, layout))
        }
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        // Handle zero-sized deallocations
        if layout.size() == 0 {
            return;
        }

        // Return the block to the free list
        self.deallocate_block(ptr);
    }

    unsafe fn reallocate(
        &self,
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> AllocResult<NonNull<[u8]>> {
        // Pool allocator can only handle allocations of the configured size
        if new_layout.size() > self.block_size || new_layout.align() > self.block_align {
            return Err(AllocError::with_layout(
                AllocErrorCode::InvalidLayout,
                new_layout,
            ));
        }

        // If the new size fits within the same block, we can reuse it
        if new_layout.size() <= self.block_size && new_layout.align() <= self.block_align {
            // Just return the same pointer with the new size
            return Ok(NonNull::slice_from_raw_parts(ptr, new_layout.size()));
        }

        // This should not be reached given the checks above, but for completeness:
        // Fall back to allocate + copy + deallocate
        let new_ptr = unsafe { self.allocate(new_layout)? };

        let copy_size = core::cmp::min(old_layout.size(), new_layout.size());
        if copy_size > 0 {
            unsafe {
                ptr::copy_nonoverlapping(ptr.as_ptr(), new_ptr.as_ptr() as *mut u8, copy_size);
            }
        }

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
    unsafe fn reset(&self) {
        // Reset the free list to include all blocks
        let mut prev_block: *mut FreeBlock = ptr::null_mut();

        // Re-link all blocks together
        for i in (0..self.block_count).rev() {
            let block_addr = self.start_addr + (i * self.block_size);
            let block = block_addr as *mut FreeBlock;

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
    fn statistics(&self) -> super::AllocatorStats {
        super::AllocatorStats {
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
            total_bytes_allocated: self.total_allocs.load(Ordering::Relaxed) as usize * self.block_size,
            total_bytes_deallocated: self.total_deallocs.load(Ordering::Relaxed) as usize * self.block_size,
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

// Thread safety
unsafe impl Send for PoolAllocator {}
unsafe impl Sync for PoolAllocator {}

/// RAII helper for pool-allocated objects
///
/// This struct provides automatic deallocation when the object goes out of
/// scope.
pub struct PoolBox<T> {
    ptr: NonNull<T>,
    allocator: NonNull<PoolAllocator>,
}

impl<T> PoolBox<T> {
    /// Creates a new PoolBox by allocating from the given pool
    pub fn new_in(value: T, allocator: &PoolAllocator) -> Result<Self, AllocError> {
        let layout = Layout::new::<T>();

        unsafe {
            let ptr = allocator.allocate(layout)?;
            let typed_ptr = ptr.as_ptr() as *mut T;
            typed_ptr.write(value);

            Ok(Self {
                ptr: NonNull::new_unchecked(typed_ptr),
                allocator: NonNull::new_unchecked(allocator as *const _ as *mut _),
            })
        }
    }

    /// Gets a reference to the contained value
    pub fn as_ref(&self) -> &T {
        unsafe { self.ptr.as_ref() }
    }

    /// Gets a mutable reference to the contained value
    pub fn as_mut(&mut self) -> &mut T {
        unsafe { self.ptr.as_mut() }
    }

    /// Consumes the PoolBox and returns the contained value
    pub fn into_inner(mut self) -> T {
        let value = unsafe { ptr::read(self.ptr.as_ptr()) };

        // Deallocate without running the destructor
        unsafe {
            let layout = Layout::new::<T>();
            self.allocator.as_ref().deallocate(self.ptr.cast(), layout);
        }

        // Prevent double-free
        core::mem::forget(self);

        value
    }
}

impl<T> core::ops::Deref for PoolBox<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

impl<T> core::ops::DerefMut for PoolBox<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.as_mut()
    }
}

impl<T> Drop for PoolBox<T> {
    fn drop(&mut self) {
        unsafe {
            // Run the destructor
            ptr::drop_in_place(self.ptr.as_ptr());

            // Deallocate the memory
            let layout = Layout::new::<T>();
            self.allocator.as_ref().deallocate(self.ptr.cast(), layout);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reallocate_simple() {
        // Create a pool with large enough blocks
        let allocator = PoolAllocator::new(64, 8, 10).unwrap(); // 64-byte blocks, 8-byte aligned

        unsafe {
            let old_layout = Layout::from_size_align(16, 4).unwrap();
            let ptr = allocator.allocate(old_layout).unwrap();

            // Write test data
            (ptr.as_ptr() as *mut u64).write(0xDEADBEEF);

            // Reallocate to larger size that still fits in the 64-byte block
            let new_layout = Layout::from_size_align(32, 4).unwrap();
            let new_ptr = allocator.reallocate(ptr.cast(), old_layout, new_layout).unwrap();

            // Should be the same pointer since it fits in the same block
            assert_eq!(ptr.as_ptr() as *const u8, new_ptr.as_ptr() as *const u8);
            assert_eq!(new_ptr.len(), 32);

            // Data should be preserved
            assert_eq!((new_ptr.as_ptr() as *const u64).read(), 0xDEADBEEF);

            // Try to reallocate to a size larger than block size - should fail
            let too_large_layout = Layout::from_size_align(128, 4).unwrap();
            let result = allocator.reallocate(new_ptr.cast(), new_layout, too_large_layout);
            assert!(result.is_err());

            // Clean up
            allocator.deallocate(new_ptr.cast(), new_layout);
        }
    }

    #[test]
    fn test_basic_allocation() {
        let allocator = PoolAllocator::for_type::<u64>(10).unwrap();
        let layout = Layout::new::<u64>();

        unsafe {
            // Allocate a few blocks
            let ptr1 = allocator.allocate(layout).unwrap();
            let ptr2 = allocator.allocate(layout).unwrap();
            let ptr3 = allocator.allocate(layout).unwrap();

            assert_eq!(ptr1.len(), 8);
            assert_eq!(ptr2.len(), 8);
            assert_eq!(ptr3.len(), 8);

            // All pointers should be different
            assert_ne!(ptr1.as_ptr(), ptr2.as_ptr());
            assert_ne!(ptr2.as_ptr(), ptr3.as_ptr());
            assert_ne!(ptr1.as_ptr(), ptr3.as_ptr());

            // Deallocate
            allocator.deallocate(ptr1.cast(), layout);
            allocator.deallocate(ptr2.cast(), layout);
            allocator.deallocate(ptr3.cast(), layout);
        }
    }

    #[test]
    fn test_pool_exhaustion() {
        let allocator = PoolAllocator::for_type::<u32>(2).unwrap(); // Very small pool
        let layout = Layout::new::<u32>();

        // Debug info
        println!("Pool block size: {}, align: {}", allocator.block_size(), allocator.block_align());
        println!("Layout size: {}, align: {}", layout.size(), layout.align());

        unsafe {
            // Allocate all blocks
            let ptr1 = allocator.allocate(layout).unwrap();
            let ptr2 = allocator.allocate(layout).unwrap();

            // Next allocation should fail
            let result = allocator.allocate(layout);
            assert!(result.is_err());
            assert!(allocator.is_full());

            // Deallocate one block
            allocator.deallocate(ptr1.cast(), layout);
            assert!(!allocator.is_full());

            // Should be able to allocate again
            let ptr3 = allocator.allocate(layout).unwrap();
            assert!(ptr3.len() > 0);

            // Clean up
            allocator.deallocate(ptr2.cast(), layout);
            allocator.deallocate(ptr3.cast(), layout);
        }
    }

    #[test]
    fn test_free_list_integrity() {
        let allocator = PoolAllocator::for_type::<u64>(5).unwrap();
        let layout = Layout::new::<u64>();

        unsafe {
            // Allocate all blocks
            let mut ptrs = Vec::new();
            for _ in 0..5 {
                ptrs.push(allocator.allocate(layout).unwrap());
            }

            assert_eq!(allocator.free_blocks(), 0);
            assert_eq!(allocator.allocated_blocks(), 5);

            // Deallocate in random order
            allocator.deallocate(ptrs[2].cast(), layout);
            allocator.deallocate(ptrs[0].cast(), layout);
            allocator.deallocate(ptrs[4].cast(), layout);

            assert_eq!(allocator.free_blocks(), 3);
            assert_eq!(allocator.allocated_blocks(), 2);

            // Allocate again - should reuse freed blocks
            let new_ptr1 = allocator.allocate(layout).unwrap();
            let new_ptr2 = allocator.allocate(layout).unwrap();

            assert_eq!(allocator.free_blocks(), 1);
            assert_eq!(allocator.allocated_blocks(), 4);

            // Clean up remaining
            allocator.deallocate(ptrs[1].cast(), layout);
            allocator.deallocate(ptrs[3].cast(), layout);
            allocator.deallocate(new_ptr1.cast(), layout);
            allocator.deallocate(new_ptr2.cast(), layout);
        }
    }

    #[test]
    fn test_invalid_layout() {
        let allocator = PoolAllocator::for_type::<u32>(10).unwrap(); // 4-byte blocks

        // Try to allocate something larger than block size
        let large_layout = Layout::new::<[u8; 16]>();
        unsafe {
            let result = allocator.allocate(large_layout);
            assert!(result.is_err());
        }

        // Try to allocate with stricter alignment
        let aligned_layout = Layout::from_size_align(4, 16).unwrap();
        unsafe {
            let result = allocator.allocate(aligned_layout);
            // This might succeed or fail depending on pool alignment
        }
    }

    #[test]
    fn test_reset() {
        let allocator = PoolAllocator::for_type::<u64>(5).unwrap();
        let layout = Layout::new::<u64>();

        unsafe {
            // Allocate some blocks
            let _ptr1 = allocator.allocate(layout).unwrap();
            let _ptr2 = allocator.allocate(layout).unwrap();

            assert_eq!(allocator.allocated_blocks(), 2);
            assert_eq!(allocator.free_blocks(), 3);

            // Reset the pool
            allocator.reset();

            assert_eq!(allocator.allocated_blocks(), 0);
            assert_eq!(allocator.free_blocks(), 5);

            // Should be able to allocate all blocks again
            for _ in 0..5 {
                let ptr = allocator.allocate(layout).unwrap();
                assert!(ptr.len() > 0);
            }
        }
    }

    #[test]
    fn test_pool_box() {
        let allocator = PoolAllocator::for_type::<i32>(10).unwrap();

        // Create a PoolBox
        let mut pool_box = PoolBox::new_in(42i32, &allocator).unwrap();
        assert_eq!(*pool_box, 42);

        // Modify the value
        *pool_box = 100;
        assert_eq!(*pool_box, 100);

        // Extract the value
        let value = pool_box.into_inner();
        assert_eq!(value, 100);

        // Pool should have the block available again
        assert_eq!(allocator.allocated_blocks(), 0);
    }

    #[test]
    fn test_thread_safety() {
        use std::sync::Arc;
        use std::thread;

        let allocator = Arc::new(PoolAllocator::for_type::<u64>(100).unwrap());
        let layout = Layout::new::<u64>();

        let handles: Vec<_> = (0..4)
            .map(|thread_id| {
                let alloc = allocator.clone();
                thread::spawn(move || {
                    let mut allocations = Vec::new();

                    // Each thread allocates 10 blocks
                    for i in 0..10 {
                        unsafe {
                            if let Ok(ptr) = alloc.allocate(layout) {
                                // Write thread-specific data
                                (ptr.as_ptr() as *mut u64).write(thread_id * 1000 + i);
                                allocations.push(ptr);
                            }
                        }
                    }

                    // Verify data integrity
                    for (i, ptr) in allocations.iter().enumerate() {
                        unsafe {
                            let value = (ptr.as_ptr() as *const u64).read();
                            assert_eq!(value, thread_id * 1000 + i as u64);
                        }
                    }

                    // Deallocate all
                    for ptr in allocations {
                        unsafe {
                            alloc.deallocate(ptr.cast(), layout);
                        }
                    }
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }

        // All blocks should be free again
        assert_eq!(allocator.allocated_blocks(), 0);
        assert_eq!(allocator.free_blocks(), 100);
    }

    #[test]
    fn test_reallocate() {
        let allocator = PoolAllocator::for_type::<[u8; 16]>(10).unwrap();

        unsafe {
            let old_layout = Layout::from_size_align(8, 1).unwrap();
            let ptr = allocator.allocate(old_layout).unwrap();

            // Write test data
            (ptr.as_ptr() as *mut u64).write(0xDEADBEEF);

            // Reallocate to larger size within the same block capacity
            let new_layout = Layout::from_size_align(12, 1).unwrap();
            let new_ptr = allocator.reallocate(ptr.cast(), old_layout, new_layout).unwrap();

            // Compare addresses (not slice metadata)
            assert_eq!(ptr.as_ptr() as *const u8, new_ptr.as_ptr() as *const u8);
            assert_eq!(new_ptr.len(), 12);

            // Data should be preserved
            assert_eq!((new_ptr.as_ptr() as *const u64).read(), 0xDEADBEEF);

            // Clean up
            allocator.deallocate(new_ptr.cast(), new_layout);
        }
    }
}
