//! Tracked allocator implementation
//!
//! Provides an allocator that tracks memory usage statistics
//! by wrapping another allocator implementation.

use core::alloc::Layout;
use core::ptr::NonNull;

use super::{
    AllocResult, Allocator, AllocatorStats, AtomicAllocatorStats, BasicMemoryUsage,
    BulkAllocator, MemoryUsage, Resettable, StatisticsProvider, ThreadSafeAllocator,
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
        Self { inner: allocator, stats: AtomicAllocatorStats::new() }
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
        stats.allocation_count.saturating_sub(stats.deallocation_count)
    }
}

unsafe impl<A: Allocator> Allocator for TrackedAllocator<A> {
    unsafe fn allocate(&self, layout: Layout) -> AllocResult<NonNull<[u8]>> {
        // Forward allocation to inner allocator
        match unsafe { self.inner.allocate(layout) } {
            Ok(ptr) => {
                // Record successful allocation
                self.stats.record_allocation(layout.size());
                Ok(ptr)
            },
            Err(err) => {
                // Record failed allocation
                self.stats.record_allocation_failure();
                Err(err)
            },
        }
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        // Forward deallocation to inner allocator
        unsafe { self.inner.deallocate(ptr, layout) };

        // Record deallocation
        self.stats.record_deallocation(layout.size());
    }

    unsafe fn reallocate(
        &self,
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> AllocResult<NonNull<[u8]>> {
        // Forward reallocation to inner allocator
        match unsafe { self.inner.reallocate(ptr, old_layout, new_layout) } {
            Ok(new_ptr) => {
                // Record successful reallocation
                self.stats.record_reallocation(old_layout.size(), new_layout.size());
                Ok(new_ptr)
            },
            Err(err) => {
                // Record failed reallocation as failed allocation
                self.stats.record_allocation_failure();
                Err(err)
            },
        }
    }

    fn max_allocation_size() -> usize {
        A::max_allocation_size()
    }

    fn supports_zero_sized_allocs() -> bool {
        A::supports_zero_sized_allocs()
    }
}

// Forward BulkAllocator if the inner allocator supports it
unsafe impl<A: BulkAllocator> BulkAllocator for TrackedAllocator<A> {
    unsafe fn allocate_contiguous(
        &self,
        layout: Layout,
        count: usize,
    ) -> AllocResult<NonNull<[u8]>> {
        let total_size = layout.size().saturating_mul(count);

        match unsafe { self.inner.allocate_contiguous(layout, count) } {
            Ok(ptr) => {
                self.stats.record_allocation(total_size);
                Ok(ptr)
            },
            Err(err) => {
                self.stats.record_allocation_failure();
                Err(err)
            },
        }
    }

    unsafe fn deallocate_contiguous(&self, ptr: NonNull<u8>, layout: Layout, count: usize) {
        unsafe { self.inner.deallocate_contiguous(ptr, layout, count) };

        let total_size = layout.size().saturating_mul(count);
        self.stats.record_deallocation(total_size);
    }

    unsafe fn reallocate_contiguous(
        &self,
        ptr: NonNull<u8>,
        layout: Layout,
        old_count: usize,
        new_count: usize,
    ) -> AllocResult<NonNull<[u8]>> {
        let old_total_size = layout.size().saturating_mul(old_count);
        let new_total_size = layout.size().saturating_mul(new_count);

        match unsafe { self.inner.reallocate_contiguous(ptr, layout, old_count, new_count) } {
            Ok(new_ptr) => {
                self.stats.record_reallocation(old_total_size, new_total_size);
                Ok(new_ptr)
            },
            Err(err) => {
                self.stats.record_allocation_failure();
                Err(err)
            },
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
    unsafe fn reset(&self) {
        // Reset the inner allocator first
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

// Thread safety: TrackedAllocator is thread-safe if the inner allocator is
unsafe impl<A: Send> Send for TrackedAllocator<A> {}
unsafe impl<A: Sync> Sync for TrackedAllocator<A> {}
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
