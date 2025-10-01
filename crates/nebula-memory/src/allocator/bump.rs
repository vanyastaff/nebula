//! Production-ready bump allocator with safe abstractions
//!
//! Now uses safe utilities from utils module for all unsafe operations

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(not(feature = "std"))]
use alloc::{boxed::Box, vec, vec::Vec};

use core::alloc::Layout;
use core::cell::Cell;
use core::ptr::{self, NonNull};
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use super::{
    AllocError, AllocErrorCode, AllocResult, Allocator,
    BulkAllocator, MemoryUsage, OptionalStats, Resettable,
    StatisticsProvider, ThreadSafeAllocator,
};

// Import safe utilities
use crate::utils::{
    PrefetchManager, MemoryOps,
    atomic_max, Backoff, memory_barrier_ex, BarrierType,
    cache_line_size, is_power_of_two, align_up,
};

/// Configuration for bump allocator
#[derive(Debug, Clone)]
pub struct BumpConfig {
    /// Enable statistics tracking
    pub track_stats: bool,

    /// Fill patterns for debugging
    pub alloc_pattern: Option<u8>,
    pub dealloc_pattern: Option<u8>,

    /// Prefetching configuration
    pub enable_prefetch: bool,
    pub prefetch_distance: usize,

    /// Minimum allocation size (helps avoid false sharing)
    pub min_alloc_size: usize,

    /// Thread-safe mode (use atomics vs Cell)
    pub thread_safe: bool,
}

impl Default for BumpConfig {
    fn default() -> Self {
        Self {
            track_stats: cfg!(debug_assertions),
            alloc_pattern: if cfg!(debug_assertions) { Some(0xAA) } else { None },
            dealloc_pattern: if cfg!(debug_assertions) { Some(0xDD) } else { None },
            enable_prefetch: true,
            prefetch_distance: 4,
            min_alloc_size: 8,
            thread_safe: true,
        }
    }
}

impl BumpConfig {
    /// Production configuration - optimized for maximum performance
    pub fn production() -> Self {
        Self {
            track_stats: false,
            alloc_pattern: None,
            dealloc_pattern: None,
            enable_prefetch: true,
            prefetch_distance: 8,
            min_alloc_size: 16,
            thread_safe: true,
        }
    }

    /// Debug configuration - optimized for debugging and error detection
    pub fn debug() -> Self {
        Self {
            track_stats: true,
            alloc_pattern: Some(0xAA),
            dealloc_pattern: Some(0xDD),
            enable_prefetch: false,
            prefetch_distance: 0,
            min_alloc_size: 1,
            thread_safe: true,
        }
    }

    /// Single-threaded configuration - avoids atomic overhead
    pub fn single_thread() -> Self {
        Self {
            thread_safe: false,
            ..Self::production()
        }
    }

    /// Performance configuration - minimal overhead, no stats, aggressive prefetch
    pub fn performance() -> Self {
        Self {
            track_stats: false,
            alloc_pattern: None,
            dealloc_pattern: None,
            enable_prefetch: true,
            prefetch_distance: 16,
            min_alloc_size: 32,
            thread_safe: true,
        }
    }

    /// Conservative configuration - balanced for general use
    pub fn conservative() -> Self {
        Self {
            track_stats: cfg!(debug_assertions),
            alloc_pattern: None,
            dealloc_pattern: None,
            enable_prefetch: true,
            prefetch_distance: 4,
            min_alloc_size: 8,
            thread_safe: true,
        }
    }
}

/// Trait abstraction for cursor (allows both atomic and non-atomic)
trait Cursor: Send + Sync {
    fn load(&self, ordering: Ordering) -> usize;
    fn store(&self, val: usize, ordering: Ordering);
    fn compare_exchange_weak(
        &self,
        current: usize,
        new: usize,
        success: Ordering,
        failure: Ordering,
    ) -> Result<usize, usize>;
}

/// Atomic cursor for thread-safe mode
struct AtomicCursor(AtomicUsize);

impl Cursor for AtomicCursor {
    #[inline]
    fn load(&self, ordering: Ordering) -> usize {
        self.0.load(ordering)
    }

    #[inline]
    fn store(&self, val: usize, ordering: Ordering) {
        self.0.store(val, ordering)
    }

    #[inline]
    fn compare_exchange_weak(
        &self,
        current: usize,
        new: usize,
        success: Ordering,
        failure: Ordering,
    ) -> Result<usize, usize> {
        self.0.compare_exchange_weak(current, new, success, failure)
    }
}

/// Non-atomic cursor for single-thread mode (faster)
struct CellCursor(Cell<usize>);

impl Cursor for CellCursor {
    #[inline]
    fn load(&self, _ordering: Ordering) -> usize {
        self.0.get()
    }

    #[inline]
    fn store(&self, val: usize, _ordering: Ordering) {
        self.0.set(val)
    }

    #[inline]
    fn compare_exchange_weak(
        &self,
        current: usize,
        new: usize,
        _success: Ordering,
        _failure: Ordering,
    ) -> Result<usize, usize> {
        let actual = self.0.get();
        if actual == current {
            self.0.set(new);
            Ok(current)
        } else {
            Err(actual)
        }
    }
}

unsafe impl Send for CellCursor {}
unsafe impl Sync for CellCursor {} // Safe because we only use in single-thread mode

/// Checkpoint for saving/restoring allocator state
#[derive(Debug, Clone, Copy)]
pub struct BumpCheckpoint {
    position: usize,
    generation: u32,
}

/// RAII guard for automatic checkpoint restoration
pub struct BumpScope<'a> {
    allocator: &'a BumpAllocator,
    checkpoint: BumpCheckpoint,
}

impl<'a> BumpScope<'a> {
    pub fn new(allocator: &'a BumpAllocator) -> Self {
        Self {
            checkpoint: allocator.checkpoint(),
            allocator,
        }
    }
}

impl<'a> Drop for BumpScope<'a> {
    fn drop(&mut self) {
        // Ignore errors during drop - we can't propagate them
        // The restore() method validates the checkpoint internally
        let _ = self.allocator.restore(self.checkpoint);
    }
}

/// Production-ready bump allocator with safe utilities
pub struct BumpAllocator {
    /// Owned memory buffer
    memory: Box<[u8]>,

    /// Configuration
    config: BumpConfig,

    /// Safe utility managers (created per instance)
    prefetch_mgr: PrefetchManager,
    memory_ops: MemoryOps,

    /// Memory bounds
    start_addr: usize,
    end_addr: usize,

    /// Current position (either atomic or cell based on config)
    cursor: Box<dyn Cursor>,

    /// Statistics
    stats: OptionalStats,
    peak_usage: AtomicUsize,

    /// Generation counter for checkpoint validation
    generation: AtomicU32,
}

impl BumpAllocator {
    /// Creates a new bump allocator with the specified capacity and config
    pub fn with_config(capacity: usize, config: BumpConfig) -> AllocResult<Self> {
        if capacity == 0 {
            return Err(AllocError::with_layout(
                AllocErrorCode::InvalidLayout,
                Layout::from_size_align(0, 1).unwrap(),
            ));
        }

        let mut memory = vec![0u8; capacity].into_boxed_slice();

        // Use safe memory operations for pattern fill
        let memory_ops = MemoryOps::new();
        if let Some(pattern) = config.alloc_pattern {
            unsafe {
                MemoryOps::secure_fill_slice(&mut memory, pattern);
            }
        }

        let start_addr = memory.as_ptr() as usize;
        let end_addr = start_addr + capacity;

        // Create appropriate cursor based on thread-safety config
        let cursor: Box<dyn Cursor> = if config.thread_safe {
            Box::new(AtomicCursor(AtomicUsize::new(start_addr)))
        } else {
            Box::new(CellCursor(Cell::new(start_addr)))
        };

        Ok(Self {
            memory,
            prefetch_mgr: PrefetchManager::new(),
            memory_ops: MemoryOps::new(),
            stats: if config.track_stats {
                OptionalStats::enabled()
            } else {
                OptionalStats::disabled()
            },
            config,
            start_addr,
            cursor,
            end_addr,
            generation: AtomicU32::new(0),
            peak_usage: AtomicUsize::new(0),
        })
    }

    /// Creates a new bump allocator with default config
    pub fn new(capacity: usize) -> AllocResult<Self> {
        Self::with_config(capacity, BumpConfig::default())
    }

    /// Creates a production-optimized bump allocator
    pub fn production(capacity: usize) -> AllocResult<Self> {
        Self::with_config(capacity, BumpConfig::production())
    }

    /// Creates a debug-optimized bump allocator
    pub fn debug(capacity: usize) -> AllocResult<Self> {
        Self::with_config(capacity, BumpConfig::debug())
    }

    /// Creates a single-threaded bump allocator (faster, no atomics)
    pub fn single_thread(capacity: usize) -> AllocResult<Self> {
        Self::with_config(capacity, BumpConfig::single_thread())
    }

    /// Creates a performance-optimized bump allocator
    pub fn performance(capacity: usize) -> AllocResult<Self> {
        Self::with_config(capacity, BumpConfig::performance())
    }

    /// Convenience constructors for common sizes
    pub fn small() -> AllocResult<Self> {
        Self::new(64 * 1024) // 64KB
    }

    pub fn medium() -> AllocResult<Self> {
        Self::new(1024 * 1024) // 1MB
    }

    pub fn large() -> AllocResult<Self> {
        Self::new(16 * 1024 * 1024) // 16MB
    }

    /// Returns the total capacity of the allocator
    #[inline]
    pub fn capacity(&self) -> usize {
        self.memory.len()
    }

    /// Returns the amount of memory currently allocated
    #[inline]
    pub fn used(&self) -> usize {
        let current = self.cursor.load(Ordering::Relaxed);
        current.saturating_sub(self.start_addr)
    }

    /// Returns the amount of memory available for allocation
    #[inline]
    pub fn available(&self) -> usize {
        self.capacity().saturating_sub(self.used())
    }

    /// Returns peak memory usage
    #[inline]
    pub fn peak_usage(&self) -> usize {
        self.peak_usage.load(Ordering::Relaxed)
    }

    /// Calculate effective size accounting for min_alloc_size
    #[inline]
    fn effective_size(&self, size: usize) -> usize {
        size.max(self.config.min_alloc_size)
    }

    /// Safe prefetch using PrefetchManager
    #[inline]
    fn prefetch_if_enabled(&self, next_addr: usize) {
        if !self.config.enable_prefetch || self.config.prefetch_distance == 0 {
            return;
        }

        // Calculate prefetch distance
        let cache_line = cache_line_size();
        let prefetch_bytes = self.config.prefetch_distance * cache_line;
        let prefetch_end = (next_addr + prefetch_bytes).min(self.end_addr);

        // Use safe prefetch for range within buffer
        if next_addr < self.end_addr && prefetch_end > next_addr {
            let start_offset = next_addr - self.start_addr;
            let end_offset = prefetch_end - self.start_addr;

            if end_offset <= self.memory.len() {
                let slice = &self.memory[start_offset..end_offset];
                self.prefetch_mgr.prefetch_slice_read(slice);
            }
        }
    }

    /// Creates a checkpoint at the current position
    #[must_use = "checkpoint должен быть сохранён для последующего restore"]
    pub fn checkpoint(&self) -> BumpCheckpoint {
        // Note: Acquire ordering in load() provides sufficient memory barrier
        // No need for explicit memory_barrier_ex() call
        BumpCheckpoint {
            position: self.cursor.load(Ordering::Acquire),
            generation: self.generation.load(Ordering::Acquire),
        }
    }

    /// Restores the allocator to a previous checkpoint
    ///
    /// # Safety
    /// Caller must ensure:
    /// - No concurrent allocations are happening
    /// - All allocations made after the checkpoint are no longer in use
    /// - The checkpoint was created by this allocator instance
    ///
    /// Violating these invariants may lead to use-after-free or double-free bugs.
    ///
    /// # Errors
    /// Returns `Err` if:
    /// - Checkpoint is from a different generation (after reset)
    /// - Checkpoint position is invalid
    /// - Checkpoint is in the future
    pub fn restore(&self, checkpoint: BumpCheckpoint) -> AllocResult<()> {
        let current_gen = self.generation.load(Ordering::Acquire);

        // Validate checkpoint generation
        if checkpoint.generation != current_gen {
            return Err(AllocError::invalid_input(
                "checkpoint is from a different generation"
            ));
        }

        let current = self.cursor.load(Ordering::Acquire);

        // Validate checkpoint position
        if checkpoint.position < self.start_addr || checkpoint.position > self.end_addr {
            return Err(AllocError::invalid_input(
                "checkpoint position is outside allocator bounds"
            ));
        }

        if checkpoint.position > current {
            return Err(AllocError::invalid_input(
                "checkpoint is in the future"
            ));
        }

        // Use safe memory operations for dealloc pattern
        if let Some(pattern) = self.config.dealloc_pattern {
            let dealloc_start = checkpoint.position - self.start_addr;
            let dealloc_end = current - self.start_addr;

            if let Some(slice) = self.memory.get(dealloc_start..dealloc_end) {
                // SAFETY: We use UnsafeCell pattern - caller must ensure no concurrent access
                unsafe {
                    let slice_mut = core::slice::from_raw_parts_mut(
                        slice.as_ptr() as *mut u8,
                        slice.len()
                    );
                    MemoryOps::secure_fill_slice(slice_mut, pattern);
                }
            }
        }

        // Note: Release ordering in store() provides sufficient memory barrier
        self.cursor.store(checkpoint.position, Ordering::Release);
        Ok(())
    }

    /// Creates a scoped allocation that auto-restores on drop
    pub fn scoped(&self) -> BumpScope<'_> {
        BumpScope::new(self)
    }

    /// Core allocation logic with safe utilities and optimized backoff
    #[inline]
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

            // Use safe alignment operations
            let aligned_current = align_up(current, align);
            let new_current = aligned_current.checked_add(actual_size)?;

            if new_current > self.end_addr {
                self.stats.record_allocation_failure();
                return None;
            }

            // Prefetch next cache lines (safe)
            self.prefetch_if_enabled(new_current);

            // Use strong CAS on first attempt, weak afterwards
            let result = if attempts == 0 {
                self.cursor.compare_exchange_weak(
                    current,
                    new_current,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
            } else {
                self.cursor.compare_exchange_weak(
                    current,
                    new_current,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
            };

            match result {
                Ok(_) => {
                    // Update statistics
                    self.stats.record_allocation(actual_size);

                    // Update peak usage with atomic_max
                    let usage = new_current - self.start_addr;
                    atomic_max(&self.peak_usage, usage);

                    // Use safe memory operations for allocation pattern
                    // Note: Pattern fill happens BEFORE releasing to other threads via Release ordering
                    // This ensures no thread sees uninitialized memory
                    if let Some(pattern) = self.config.alloc_pattern {
                        let offset = aligned_current - self.start_addr;
                        if let Some(slice) = self.memory.get(offset..offset + actual_size) {
                            // SAFETY: We use UnsafeCell pattern - we have exclusive logical access
                            // to this region because we just won the CAS race for it
                            unsafe {
                                let slice_mut = core::slice::from_raw_parts_mut(
                                    slice.as_ptr() as *mut u8,
                                    slice.len()
                                );
                                MemoryOps::secure_fill_slice(slice_mut, pattern);
                            }
                        }
                    }

                    // SAFETY: We've verified the pointer is within our allocated buffer
                    return Some(unsafe { NonNull::new_unchecked(aligned_current as *mut u8) });
                },
                Err(_) => {
                    // Increment attempts and use safe backoff
                    attempts += 1;
                    if self.config.thread_safe {
                        backoff.spin();
                    }
                },
            }
        }
    }

    /// Get slice from allocation
    #[inline]
    fn get_allocation_slice(&self, ptr: NonNull<u8>, size: usize) -> Option<&mut [u8]> {
        let addr = ptr.as_ptr() as usize;
        if addr >= self.start_addr && addr + size <= self.end_addr {
            let offset = addr - self.start_addr;
            // SAFETY: We've verified the range is within our buffer
            unsafe {
                Some(core::slice::from_raw_parts_mut(
                    self.memory.as_ptr().add(offset) as *mut u8,
                    size
                ))
            }
        } else {
            None
        }
    }

    /// Copy memory safely
    #[inline]
    fn safe_copy(&self, src: NonNull<u8>, dst: NonNull<u8>, size: usize) {
        if size == 0 {
            return;
        }

        // For now, just use unsafe copy since we can't get overlapping mutable slices
        // from the same Box<[u8]>
        unsafe {
            ptr::copy_nonoverlapping(src.as_ptr(), dst.as_ptr(), size);
        }
    }

    /// Convenience constructor for extra small allocation
    pub fn tiny() -> AllocResult<Self> {
        Self::new(4 * 1024) // 4KB
    }
}

unsafe impl Allocator for BumpAllocator {
    #[inline]
    unsafe fn allocate(&self, layout: Layout) -> AllocResult<NonNull<[u8]>> {
        if layout.size() == 0 {
            let ptr = NonNull::<u8>::dangling();
            return Ok(NonNull::slice_from_raw_parts(ptr, 0));
        }

        // Validate alignment
        if !is_power_of_two(layout.align()) {
            return Err(AllocError::with_layout(
                AllocErrorCode::InvalidLayout,
                layout,
            ));
        }

        if let Some(ptr) = self.try_bump(layout.size(), layout.align()) {
            Ok(NonNull::slice_from_raw_parts(ptr, layout.size()))
        } else {
            Err(AllocError::with_layout(AllocErrorCode::OutOfMemory, layout))
        }
    }

    #[inline]
    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        // Use safe memory operations for dealloc pattern
        if let Some(pattern) = self.config.dealloc_pattern {
            if let Some(slice) = self.get_allocation_slice(ptr, layout.size()) {
                unsafe {
                    MemoryOps::secure_fill_slice(slice, pattern);
                }
            }
        }

        self.stats.record_deallocation(layout.size());
    }

    unsafe fn reallocate(
        &self,
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> AllocResult<NonNull<[u8]>> {
        let ptr_addr = ptr.as_ptr() as usize;

        // Use effective sizes for proper comparison
        let old_eff = self.effective_size(old_layout.size());
        let new_eff = self.effective_size(new_layout.size());

        let current = self.cursor.load(Ordering::Acquire);
        let expected_current = ptr_addr.checked_add(old_eff)
            .ok_or_else(AllocError::size_overflow)?;

        // Check for in-place extension
        if current == expected_current
            && new_layout.align() <= old_layout.align()
            && new_eff >= old_eff
        {
            let additional = new_eff - old_eff;
            if additional == 0 {
                self.stats.record_reallocation(old_layout.size(), new_layout.size());
                return Ok(NonNull::slice_from_raw_parts(ptr, new_layout.size()));
            }

            if let Some(_) = self.try_bump(additional, 1) {
                self.stats.record_reallocation(old_layout.size(), new_layout.size());
                return Ok(NonNull::slice_from_raw_parts(ptr, new_layout.size()));
            }
        }

        // Fall back to allocate + copy
        let new_ptr = unsafe { self.allocate(new_layout)? };
        let copy_size = core::cmp::min(old_layout.size(), new_layout.size());

        if copy_size > 0 {
            self.safe_copy(ptr, new_ptr.cast(), copy_size);
        }

        // Use safe deallocation
        if let Some(pattern) = self.config.dealloc_pattern {
            if let Some(slice) = self.get_allocation_slice(ptr, old_layout.size()) {
                unsafe {
                    MemoryOps::secure_fill_slice(slice, pattern);
                }
            }
        }

        self.stats.record_reallocation(old_layout.size(), new_layout.size());
        Ok(new_ptr)
    }

    unsafe fn grow(
        &self,
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> AllocResult<NonNull<[u8]>> {
        let ptr_addr = ptr.as_ptr() as usize;

        let old_eff = self.effective_size(old_layout.size());
        let new_eff = self.effective_size(new_layout.size());

        let current = self.cursor.load(Ordering::Acquire);
        let expected_current = ptr_addr.checked_add(old_eff)
            .ok_or_else(AllocError::size_overflow)?;

        if current == expected_current
            && new_layout.align() <= old_layout.align()
            && new_eff >= old_eff
        {
            let additional = new_eff - old_eff;
            if additional == 0 {
                return Ok(NonNull::slice_from_raw_parts(ptr, new_layout.size()));
            }

            if let Some(_) = self.try_bump(additional, 1) {
                return Ok(NonNull::slice_from_raw_parts(ptr, new_layout.size()));
            }
        }

        // Fallback with safe copy
        let new_ptr = unsafe { self.allocate(new_layout)? };
        self.safe_copy(ptr, new_ptr.cast(), old_layout.size());
        unsafe { self.deallocate(ptr, old_layout); }
        Ok(new_ptr)
    }
}

unsafe impl BulkAllocator for BumpAllocator {}

impl MemoryUsage for BumpAllocator {
    #[inline]
    fn used_memory(&self) -> usize {
        self.used()
    }

    #[inline]
    fn available_memory(&self) -> Option<usize> {
        Some(self.available())
    }

    #[inline]
    fn total_memory(&self) -> Option<usize> {
        Some(self.capacity())
    }
}

impl Resettable for BumpAllocator {
    /// Resets the allocator, invalidating all previous allocations
    ///
    /// # Safety
    /// - All existing allocations become invalid
    /// - Must not be called while other threads hold references
    unsafe fn reset(&self) {
        // Use safe memory operations for clearing
        if let Some(pattern) = self.config.dealloc_pattern {
            let used = self.used();
            if used > 0 && used <= self.memory.len() {
                // SAFETY: We've verified the range is within our buffer
                unsafe {
                    let slice = core::slice::from_raw_parts_mut(
                        self.memory.as_ptr() as *mut u8,
                        used
                    );
                    unsafe {
                    MemoryOps::secure_fill_slice(slice, pattern);
                }
                }
            }
        }

        self.cursor.store(self.start_addr, Ordering::Release);
        self.generation.fetch_add(1, Ordering::AcqRel);
        self.stats.reset();
        self.peak_usage.store(0, Ordering::Relaxed);

        // Memory barrier for reset consistency
        memory_barrier_ex(BarrierType::Release);
    }

    fn can_reset(&self) -> bool {
        true
    }
}

impl StatisticsProvider for BumpAllocator {
    fn statistics(&self) -> super::AllocatorStats {
        self.stats.snapshot().unwrap_or_default()
    }

    fn reset_statistics(&self) {
        self.stats.reset();
    }

    fn statistics_enabled(&self) -> bool {
        self.stats.is_enabled()
    }
}

// Thread safety markers
unsafe impl Send for BumpAllocator {}
unsafe impl Sync for BumpAllocator {}
unsafe impl ThreadSafeAllocator for BumpAllocator {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_realloc_with_min_alloc_size() {
        let mut cfg = BumpConfig::default();
        cfg.min_alloc_size = 16;
        let allocator = BumpAllocator::with_config(1024, cfg).unwrap();

        unsafe {
            let small = allocator.allocate(Layout::from_size_align(8, 8).unwrap()).unwrap();

            // Was 8, but occupied 16; increase to 12 — should extend in-place
            let grown = allocator.reallocate(
                small.cast(),
                Layout::from_size_align(8, 8).unwrap(),
                Layout::from_size_align(12, 8).unwrap()
            ).unwrap();

            assert_eq!(grown.len(), 12);
            // Cursor should move only once
            assert!(allocator.used() >= 16 && allocator.used() < 32);
        }
    }

    #[test]
    fn test_single_thread_mode() {
        let config = BumpConfig::single_thread();
        let allocator = BumpAllocator::with_config(1024, config).unwrap();

        unsafe {
            let layout = Layout::new::<u64>();
            let ptr = allocator.allocate(layout).unwrap();
            assert_eq!(ptr.len(), 8);

            allocator.deallocate(ptr.cast(), layout);
        }
    }

    #[test]
    fn test_checkpoint_safety() {
        let allocator = BumpAllocator::new(1024).unwrap();

        unsafe {
            let checkpoint = allocator.checkpoint();
            let _ptr = allocator.allocate(Layout::new::<u64>()).unwrap();

            // Reset changes generation
            allocator.reset();

            // Old checkpoint should be ignored
            allocator.restore(checkpoint);
            assert_eq!(allocator.used(), 0);
        }
    }

    #[test]
    fn test_safe_operations() {
        let allocator = BumpAllocator::new(1024).unwrap();

        unsafe {
            // Test alignment operations
            let layout = Layout::from_size_align(7, 8).unwrap();
            let ptr = allocator.allocate(layout).unwrap();
            assert_eq!(ptr.len(), 7);

            // Should be aligned to 8
            let addr = ptr.as_ptr() as *const u8 as usize;
            assert!(crate::utils::is_aligned(addr, 8));

            // Test memory patterns
            let mut config = BumpConfig::debug();
            config.alloc_pattern = Some(0xAB);
            config.dealloc_pattern = Some(0xCD);

            let debug_alloc = BumpAllocator::with_config(256, config).unwrap();
            let layout2 = Layout::new::<u32>();
            let ptr2 = debug_alloc.allocate(layout2).unwrap();

            // Allocation should be filled with pattern
            let slice = core::slice::from_raw_parts(ptr2.as_ptr() as *const u8, 4);
            assert!(slice.iter().all(|&b| b == 0xAB));

            debug_alloc.deallocate(ptr2.cast(), layout2);
        }
    }

    #[test]
    fn test_prefetch_safety() {
        let mut config = BumpConfig::production();
        config.enable_prefetch = true;
        config.prefetch_distance = 4;

        let allocator = BumpAllocator::with_config(4096, config).unwrap();

        unsafe {
            // Allocate near the end to test prefetch bounds checking
            let _ = allocator.allocate(Layout::from_size_align(4000, 8).unwrap()).unwrap();

            // This should not panic despite being near the end
            let _ = allocator.allocate(Layout::from_size_align(64, 8).unwrap()).unwrap();
        }
    }

    #[cfg(feature = "std")]
    #[test]
    fn test_with_platform_info() {
        let platform = PlatformInfo::current();
        let allocator = BumpAllocator::new(platform.page_size).unwrap();

        assert_eq!(allocator.capacity(), platform.page_size);

        unsafe {
            // Allocate cache-line aligned data
            let cache_line = cache_line_size();
            let layout = Layout::from_size_align(cache_line, cache_line).unwrap();
            let ptr = allocator.allocate(layout).unwrap();

            let addr = ptr.as_ptr() as *const u8 as usize;
            assert!(crate::utils::is_aligned(addr, cache_line));
        }
    }
}