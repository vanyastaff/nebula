//! Tracked allocator implementation
//!
//! Provides an allocator that tracks memory usage statistics
//! by wrapping another allocator implementation.
//!
//! # Safety
//!
//! This module wraps an underlying allocator and tracks all allocation operations:
//! - All unsafe operations are forwarded to the inner allocator with proper contracts
//! - Statistics collection is thread-safe via atomic operations
//! - No memory safety invariants are added beyond those of the inner allocator
//! - Send/Sync/ThreadSafeAllocator traits are conditionally forwarded
//!
//! ## Invariants
//!
//! - Every successful allocation is tracked in stats
//! - Every deallocation adjusts stats to match
//! - Failed allocations don't affect memory counters (only failure count)
//! - Reallocation is treated as dealloc + alloc for stats purposes

use core::alloc::Layout;
use core::ptr::NonNull;

use super::{
    AllocResult, Allocator, AllocatorStats, AtomicAllocatorStats, BasicMemoryUsage, BulkAllocator,
    MemoryUsage, Resettable, StatisticsProvider, ThreadSafeAllocator,
};

/// A wrapper allocator that tracks memory usage statistics
///
/// This allocator acts as a transparent wrapper around any other allocator,
/// collecting detailed statistics about memory usage patterns without
/// affecting the underlying allocation behavior.
///
/// # Thread Safety
/// This allocator is thread-safe if the underlying allocator is thread-safe.
/// Statistics collection uses atomic operations for thread-safe tracking.
#[derive(Debug)]
pub struct TrackedAllocator<A> {
    /// The underlying allocator
    inner: A,
    /// Statistics collection
    stats: AtomicAllocatorStats,
}

impl<A> TrackedAllocator<A> {
    /// Creates a new TrackedAllocator wrapping the provided allocator
    pub fn new(allocator: A) -> Self {
        Self {
            inner: allocator,
            stats: AtomicAllocatorStats::new(),
        }
    }

    /// Gets a reference to the underlying allocator
    pub fn inner(&self) -> &A {
        &self.inner
    }

    /// Gets a mutable reference to the underlying allocator
    pub fn inner_mut(&mut self) -> &mut A {
        &mut self.inner
    }

    /// Consumes the tracker and returns the underlying allocator
    pub fn into_inner(self) -> A {
        self.inner
    }

    /// Returns the total bytes currently allocated
    pub fn allocated_bytes(&self) -> usize {
        self.stats.current_allocated()
    }

    /// Returns the peak bytes allocated
    pub fn peak_allocated_bytes(&self) -> usize {
        self.stats.peak_allocated()
    }

    /// Returns the total number of allocations performed
    pub fn allocation_count(&self) -> usize {
        self.stats.allocation_count()
    }

    /// Returns the total number of deallocations performed
    pub fn deallocation_count(&self) -> usize {
        self.stats.snapshot().deallocation_count
    }

    /// Returns the number of failed allocations
    pub fn failed_allocations(&self) -> usize {
        self.stats.failed_allocation_count()
    }

    /// Reset statistics while keeping current allocations
    pub fn reset_stats(&self) {
        self.stats.reset();
    }

    /// Get detailed statistics snapshot
    pub fn detailed_stats(&self) -> AllocatorStats {
        self.stats.snapshot()
    }

    /// Check if there are any memory leaks (allocations > deallocations)
    pub fn has_leaks(&self) -> bool {
        let stats = self.stats.snapshot();
        stats.allocation_count > stats.deallocation_count
    }

    /// Get the number of potentially leaked allocations
    pub fn potential_leaks(&self) -> usize {
        let stats = self.stats.snapshot();
        stats
            .allocation_count
            .saturating_sub(stats.deallocation_count)
    }
}

// SAFETY: TrackedAllocator implements Allocator by forwarding to inner allocator.
// - All unsafe trait methods forward to A's implementation with same contracts
// - Statistics tracking is side-effect only (no memory safety impact)
// - Layout and pointer validity requirements are preserved
unsafe impl<A: Allocator> Allocator for TrackedAllocator<A> {
    /// # Safety
    ///
    /// Caller must ensure:
    /// - `layout` has non-zero size (unless A::supports_zero_sized_allocs())
    /// - `layout.align()` is a power of two
    /// - `layout.size()` when rounded up to nearest multiple of align does not overflow isize
    unsafe fn allocate(&self, layout: Layout) -> AllocResult<NonNull<[u8]>> {
        // SAFETY: Forwarding to inner allocator's allocate.
        // - layout validity is enforced by caller's contract (see above)
        // - Inner allocator upholds Allocator trait safety requirements
        // - Statistics recording doesn't affect memory safety
        match unsafe { self.inner.allocate(layout) } {
            Ok(ptr) => {
                // Record successful allocation
                self.stats.record_allocation(layout.size());
                Ok(ptr)
            }
            Err(err) => {
                // Record failed allocation
                self.stats.record_allocation_failure();
                Err(err)
            }
        }
    }

    /// # Safety
    ///
    /// Caller must ensure:
    /// - `ptr` was allocated by this allocator (via inner) with the same `layout`
    /// - `ptr` is currently allocated (not already deallocated)
    /// - `layout` matches the layout used for allocation
    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        // SAFETY: Forwarding to inner allocator's deallocate.
        // - ptr was allocated by self.inner (TrackedAllocator doesn't allocate directly)
        // - layout matches the original allocation (caller's responsibility)
        // - Statistics recording happens after deallocation (safe)
        unsafe { self.inner.deallocate(ptr, layout) };

        // Record deallocation
        self.stats.record_deallocation(layout.size());
    }

    /// # Safety
    ///
    /// Caller must ensure:
    /// - `ptr` was allocated by this allocator with `old_layout`
    /// - `old_layout` matches the layout used for the original allocation
    /// - `new_layout.align()` equals `old_layout.align()`
    /// - If reallocation fails, `ptr` remains valid with `old_layout`
    unsafe fn reallocate(
        &self,
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> AllocResult<NonNull<[u8]>> {
        // SAFETY: Forwarding to inner allocator's reallocate.
        // - ptr was allocated by self.inner with old_layout (caller's contract)
        // - new_layout.align() == old_layout.align() (caller's contract)
        // - Inner allocator preserves ptr validity on failure
        match unsafe { self.inner.reallocate(ptr, old_layout, new_layout) } {
            Ok(new_ptr) => {
                // Record successful reallocation
                self.stats
                    .record_reallocation(old_layout.size(), new_layout.size());
                Ok(new_ptr)
            }
            Err(err) => {
                // Record failed reallocation as failed allocation
                self.stats.record_allocation_failure();
                Err(err)
            }
        }
    }

    fn max_allocation_size() -> usize {
        A::max_allocation_size()
    }

    fn supports_zero_sized_allocs() -> bool {
        A::supports_zero_sized_allocs()
    }
}

// SAFETY: TrackedAllocator implements BulkAllocator if inner does.
// - All bulk operations are forwarded to A's implementation
// - Statistics track total sizes (layout.size() * count)
// - No additional memory safety requirements beyond A's contracts
unsafe impl<A: BulkAllocator> BulkAllocator for TrackedAllocator<A> {
    /// # Safety
    ///
    /// Caller must ensure:
    /// - `layout` is valid (non-zero size, power-of-two align)
    /// - `count` > 0
    /// - `layout.size() * count` does not overflow
    /// - Returned memory is suitable for `count` consecutive objects of type T with `layout`
    unsafe fn allocate_contiguous(
        &self,
        layout: Layout,
        count: usize,
    ) -> AllocResult<NonNull<[u8]>> {
        let total_size = layout.size().saturating_mul(count);

        // SAFETY: Forwarding to inner's bulk allocate.
        // - layout and count validity enforced by caller (see contract above)
        // - Inner allocator returns properly aligned contiguous memory
        match unsafe { self.inner.allocate_contiguous(layout, count) } {
            Ok(ptr) => {
                self.stats.record_allocation(total_size);
                Ok(ptr)
            }
            Err(err) => {
                self.stats.record_allocation_failure();
                Err(err)
            }
        }
    }

    /// # Safety
    ///
    /// Caller must ensure:
    /// - `ptr` was allocated by this allocator via allocate_contiguous with same `layout` and `count`
    /// - `ptr` is currently allocated (not already deallocated)
    unsafe fn deallocate_contiguous(&self, ptr: NonNull<u8>, layout: Layout, count: usize) {
        // SAFETY: Forwarding to inner's bulk deallocate.
        // - ptr was allocated by self.inner.allocate_contiguous (caller's contract)
        // - layout and count match original allocation (caller's contract)
        unsafe { self.inner.deallocate_contiguous(ptr, layout, count) };

        let total_size = layout.size().saturating_mul(count);
        self.stats.record_deallocation(total_size);
    }

    /// # Safety
    ///
    /// Caller must ensure:
    /// - `ptr` was allocated via allocate_contiguous with `layout` and `old_count`
    /// - `layout` matches the original allocation
    /// - `old_count` matches the original count
    /// - If reallocation fails, `ptr` remains valid with original layout/count
    unsafe fn reallocate_contiguous(
        &self,
        ptr: NonNull<u8>,
        layout: Layout,
        old_count: usize,
        new_count: usize,
    ) -> AllocResult<NonNull<[u8]>> {
        let old_total_size = layout.size().saturating_mul(old_count);
        let new_total_size = layout.size().saturating_mul(new_count);

        // SAFETY: Forwarding to inner's bulk reallocate.
        // - ptr was allocated by self.inner with layout and old_count (caller's contract)
        // - Inner allocator preserves ptr validity on failure
        // - Statistics correctly track size change (old_total -> new_total)
        match unsafe {
            self.inner
                .reallocate_contiguous(ptr, layout, old_count, new_count)
        } {
            Ok(new_ptr) => {
                self.stats
                    .record_reallocation(old_total_size, new_total_size);
                Ok(new_ptr)
            }
            Err(err) => {
                self.stats.record_allocation_failure();
                Err(err)
            }
        }
    }
}

// Implement MemoryUsage by combining inner allocator data with our stats
impl<A: MemoryUsage> MemoryUsage for TrackedAllocator<A> {
    fn used_memory(&self) -> usize {
        // Use our tracked allocated bytes for accuracy
        self.allocated_bytes()
    }

    fn available_memory(&self) -> Option<usize> {
        // Forward to inner allocator
        self.inner.available_memory()
    }

    fn total_memory(&self) -> Option<usize> {
        // Forward to inner allocator
        self.inner.total_memory()
    }

    fn memory_usage_percent(&self) -> Option<f32> {
        // Use our tracked data with inner's total
        if let Some(total) = self.total_memory() {
            if total > 0 {
                Some((self.used_memory() as f32 / total as f32) * 100.0)
            } else {
                Some(0.0)
            }
        } else {
            None
        }
    }

    fn memory_usage(&self) -> BasicMemoryUsage {
        BasicMemoryUsage {
            used: self.used_memory(),
            available: self.available_memory(),
            total: self.total_memory(),
            usage_percent: self.memory_usage_percent(),
        }
    }
}

// Forward Resettable if the inner allocator supports it
impl<A: Resettable> Resettable for TrackedAllocator<A> {
    /// # Safety
    ///
    /// Caller must ensure:
    /// - No outstanding references to allocated memory exist
    /// - All allocated memory from this allocator is no longer in use
    /// - Reset is properly synchronized with other threads (if applicable)
    unsafe fn reset(&self) {
        // SAFETY: Forwarding to inner allocator's reset.
        // - Caller ensures no outstanding allocations are in use (contract above)
        // - Inner allocator's reset contract is satisfied by caller's contract
        // - Statistics reset is safe (atomic operations)
        unsafe { self.inner.reset() };

        // Reset our statistics
        self.stats.reset();
    }

    fn can_reset(&self) -> bool {
        self.inner.can_reset()
    }
}

// Implement StatisticsProvider
impl<A> StatisticsProvider for TrackedAllocator<A> {
    fn statistics(&self) -> AllocatorStats {
        self.stats.snapshot()
    }

    fn reset_statistics(&self) {
        self.stats.reset();
    }

    fn statistics_enabled(&self) -> bool {
        true
    }
}

// SAFETY: TrackedAllocator is Send if inner is Send.
// - inner: A is Send (by bound)
// - stats: AtomicAllocatorStats is Send (atomic primitives)
// - All owned data can be safely transferred to another thread
unsafe impl<A: Send> Send for TrackedAllocator<A> {}

// SAFETY: TrackedAllocator is Sync if inner is Sync.
// - inner: A is Sync (by bound)
// - stats: AtomicAllocatorStats is Sync (uses atomic operations for mutations)
// - All shared access is synchronized via inner's Sync + stats' atomics
unsafe impl<A: Sync> Sync for TrackedAllocator<A> {}

// SAFETY: TrackedAllocator is ThreadSafeAllocator if inner is.
// - All allocator operations forward to A which is ThreadSafeAllocator
// - Statistics tracking uses atomic operations (thread-safe)
// - No additional synchronization needed beyond what A provides
unsafe impl<A: ThreadSafeAllocator> ThreadSafeAllocator for TrackedAllocator<A> {}

// Convenience trait for easy wrapping
pub trait TrackExt: Sized {
    /// Wrap this allocator with statistics tracking
    fn with_tracking(self) -> TrackedAllocator<Self>;
}

impl<A> TrackExt for A {
    fn with_tracking(self) -> TrackedAllocator<Self> {
        TrackedAllocator::new(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::allocator::SystemAllocator;

    #[test]
    fn test_basic_tracking() {
        let allocator = SystemAllocator::new().with_tracking();
        let layout = Layout::new::<u64>();

        assert_eq!(allocator.allocation_count(), 0);
        assert_eq!(allocator.allocated_bytes(), 0);

        unsafe {
            let ptr = allocator.allocate(layout).unwrap();
            assert_eq!(allocator.allocation_count(), 1);
            assert_eq!(allocator.allocated_bytes(), 8);

            allocator.deallocate(ptr.cast(), layout);
            assert_eq!(allocator.deallocation_count(), 1);
            assert_eq!(allocator.allocated_bytes(), 0);
        }
    }

    #[test]
    fn test_peak_tracking() {
        let allocator = SystemAllocator::new().with_tracking();
        let layout = Layout::new::<u64>();

        unsafe {
            let ptr1 = allocator.allocate(layout).unwrap();
            assert_eq!(allocator.peak_allocated_bytes(), 8);

            let ptr2 = allocator.allocate(layout).unwrap();
            assert_eq!(allocator.peak_allocated_bytes(), 16);

            allocator.deallocate(ptr1.cast(), layout);
            assert_eq!(allocator.peak_allocated_bytes(), 16); // Peak should remain

            allocator.deallocate(ptr2.cast(), layout);
            assert_eq!(allocator.peak_allocated_bytes(), 16); // Still the same
        }
    }

    #[test]
    fn test_leak_detection() {
        let allocator = SystemAllocator::new().with_tracking();
        let layout = Layout::new::<u64>();

        assert!(!allocator.has_leaks());

        unsafe {
            let _ptr1 = allocator.allocate(layout).unwrap();
            let ptr2 = allocator.allocate(layout).unwrap();

            allocator.deallocate(ptr2.cast(), layout);

            assert!(allocator.has_leaks());
            assert_eq!(allocator.potential_leaks(), 1);
        }
    }

    #[test]
    fn test_statistics_provider() {
        let allocator = SystemAllocator::new().with_tracking();
        let layout = Layout::new::<u32>();

        unsafe {
            let _ptr = allocator.allocate(layout).unwrap();
        }

        let stats = allocator.statistics();
        assert_eq!(stats.allocation_count, 1);
        assert_eq!(stats.allocated_bytes, 4);

        allocator.reset_statistics();
        let stats = allocator.statistics();
        assert_eq!(stats.allocation_count, 0);
    }

    #[test]
    fn test_inner_access() {
        let system_alloc = SystemAllocator::new();
        let mut tracked = TrackedAllocator::new(system_alloc);

        // Test access to inner allocator
        let _inner_ref = tracked.inner();
        let _inner_mut = tracked.inner_mut();

        // Test consuming the tracker
        let _system_alloc = tracked.into_inner();
    }
}
