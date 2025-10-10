//! System allocator implementation
//!
//! Provides an allocator that wraps the system's default memory allocator.
//! This implementation delegates to the global system allocator while providing
//! additional safety checks and optimizations.

use core::alloc::{GlobalAlloc, Layout};
use core::ptr::NonNull;
// Import System allocator - available in std or alloc crate
#[cfg(feature = "std")]
use std::alloc::System;

// Fallback for no_std environments without alloc
#[cfg(not(feature = "std"))]
compile_error!("SystemAllocator requires either 'std' or 'alloc' feature to be enabled");

use super::{AllocError, AllocResult, Allocator, BulkAllocator, MemoryUsage, ThreadSafeAllocator};

/// Wrapper for the system's default allocator
///
/// This allocator delegates all operations to the system's global allocator
/// while providing enhanced error handling and integration with the custom
/// allocator trait system.
///
/// # Thread Safety
/// The system allocator is inherently thread-safe as it uses the platform's
/// default memory management which handles concurrent allocations properly.
///
/// # Performance
/// Performance characteristics match the underlying system allocator:
/// - Usually optimized for general-purpose workloads
/// - May use different strategies (malloc, jemalloc, etc.) depending on
///   platform
/// - Generally provides good average-case performance with reasonable overhead
#[derive(Debug, Clone, Copy)]
pub struct SystemAllocator;

impl SystemAllocator {
    /// Creates a new SystemAllocator
    ///
    /// This is a zero-cost operation as the SystemAllocator contains no state.
    #[inline]
    pub const fn new() -> Self {
        SystemAllocator
    }

    /// Returns information about the system allocator
    pub fn info() -> &'static str {
        #[cfg(target_os = "linux")]
        return "Linux system allocator (typically glibc malloc or musl)";

        #[cfg(target_os = "windows")]
        return "Windows HeapAlloc";

        #[cfg(target_os = "macos")]
        return "macOS system allocator (libsystem_malloc)";

        #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
        return "Platform-specific system allocator";
    }
}

impl Default for SystemAllocator {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

unsafe impl Allocator for SystemAllocator {
    #[inline]
    unsafe fn allocate(&self, layout: Layout) -> AllocResult<NonNull<[u8]>> {
        if layout.size() == 0 {
            // Handle zero-sized allocations by returning a well-aligned dangling pointer
            let ptr = NonNull::<u8>::dangling();
            return Ok(NonNull::slice_from_raw_parts(ptr, 0));
        }

        // Delegate to system allocator
        let ptr = unsafe { System.alloc(layout) };

        if ptr.is_null() {
            Err(AllocError::allocation_failed(layout.size(), layout.align()))
        } else {
            let non_null = unsafe { NonNull::new_unchecked(ptr) };
            Ok(NonNull::slice_from_raw_parts(non_null, layout.size()))
        }
    }

    #[inline]
    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        if layout.size() == 0 {
            return; // Nothing to deallocate for zero-sized allocations
        }

        unsafe { System.dealloc(ptr.as_ptr(), layout) };
    }

    // Override reallocate to use system realloc when possible
    unsafe fn reallocate(
        &self,
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> AllocResult<NonNull<[u8]>> {
        // Try to use system realloc if alignment requirements match
        if old_layout.align() == new_layout.align()
            && old_layout.size() > 0
            && new_layout.size() > 0
        {
            #[cfg(feature = "std")]
            {
                let new_ptr =
                    unsafe { System.realloc(ptr.as_ptr(), old_layout, new_layout.size()) };
                if !new_ptr.is_null() {
                    let non_null = unsafe { NonNull::new_unchecked(new_ptr) };
                    return Ok(NonNull::slice_from_raw_parts(non_null, new_layout.size()));
                }
            }
        }

        // Fall back to allocate + copy + deallocate
        let new_ptr = unsafe { self.allocate(new_layout)? };

        let copy_size = core::cmp::min(old_layout.size(), new_layout.size());
        if copy_size > 0 {
            unsafe {
                core::ptr::copy_nonoverlapping(
                    ptr.as_ptr(),
                    new_ptr.as_ptr() as *mut u8,
                    copy_size,
                );
            }
        }

        unsafe { self.deallocate(ptr, old_layout) };
        Ok(new_ptr)
    }
}

// Implement BulkAllocator with default implementations
// System allocator doesn't have special bulk allocation optimizations,
// so we rely on the default implementations from the trait
unsafe impl BulkAllocator for SystemAllocator {}

// System allocator is inherently thread-safe
unsafe impl ThreadSafeAllocator for SystemAllocator {}

// System allocator cannot provide meaningful memory usage statistics
// as it doesn't track allocations itself
impl MemoryUsage for SystemAllocator {
    fn used_memory(&self) -> usize {
        // Cannot determine used memory without tracking allocations
        0
    }

    fn available_memory(&self) -> Option<usize> {
        // System allocator doesn't have a fixed limit in most cases
        None
    }

    fn total_memory(&self) -> Option<usize> {
        // No fixed total memory limit
        None
    }

    fn memory_usage_percent(&self) -> Option<f32> {
        // Cannot calculate without known limits
        None
    }
}

// SystemAllocator is thread-safe - these are automatically derived for Copy
// types, but we make it explicit for clarity
unsafe impl Send for SystemAllocator {}
unsafe impl Sync for SystemAllocator {}

#[cfg(test)]
mod tests {
    use core::alloc::Layout;

    use super::*;

    #[test]
    fn test_basic_allocation() {
        let allocator = SystemAllocator::new();
        let layout = Layout::new::<u64>();

        unsafe {
            let ptr = allocator.allocate(layout).unwrap();
            assert!(!ptr.as_ptr().is_null());
            assert_eq!(ptr.len(), layout.size());

            allocator.deallocate(ptr.cast(), layout);
        }
    }

    #[test]
    fn test_zero_sized_allocation() {
        let allocator = SystemAllocator::new();
        let layout = Layout::new::<()>();

        unsafe {
            let ptr = allocator.allocate(layout).unwrap();
            assert_eq!(ptr.len(), 0);
            // Should not crash
            allocator.deallocate(ptr.cast(), layout);
        }
    }

    #[test]
    fn test_reallocation() {
        let allocator = SystemAllocator::new();
        let old_layout = Layout::new::<u32>();
        let new_layout = Layout::new::<u64>();

        unsafe {
            let ptr = allocator.allocate(old_layout).unwrap();

            // Write some data
            *(ptr.as_ptr() as *mut u32) = 0x12345678;

            let new_ptr = allocator
                .reallocate(ptr.cast(), old_layout, new_layout)
                .unwrap();

            // Data should be preserved
            assert_eq!(*(new_ptr.as_ptr() as *const u32), 0x12345678);

            allocator.deallocate(new_ptr.cast(), new_layout);
        }
    }

    #[test]
    fn test_invalid_alignment() {
        let allocator = SystemAllocator::new();

        // Try to create layout with invalid alignment
        if let Ok(layout) = Layout::from_size_align(8, 3) {
            // 3 is not power of 2
            unsafe {
                let result = allocator.allocate(layout);
                assert!(result.is_err());
                if let Err(err) = result {
                    assert!(err.is_invalid_alignment());
                }
            }
        }
    }

    #[test]
    fn test_bulk_allocation() {
        let allocator = SystemAllocator::new();
        let layout = Layout::new::<u32>();
        let count = 10;

        unsafe {
            let ptr = allocator.allocate_contiguous(layout, count).unwrap();
            assert_eq!(ptr.len(), layout.size() * count);

            allocator.deallocate_contiguous(ptr.cast(), layout, count);
        }
    }

    #[test]
    fn test_memory_usage() {
        let allocator = SystemAllocator::new();
        let usage = allocator.memory_usage();

        // System allocator can't track usage
        assert_eq!(usage.used, 0);
        assert_eq!(usage.available, None);
        assert_eq!(usage.total, None);
        assert_eq!(usage.usage_percent, None);
    }

    #[test]
    fn test_max_allocation_size() {
        let max_size = SystemAllocator::max_allocation_size();
        assert!(max_size > 0);
        assert!(max_size <= isize::MAX as usize);
    }

    #[test]
    fn test_thread_safety_markers() {
        fn assert_send<T: Send>() {}
        fn assert_sync<T: Sync>() {}

        assert_send::<SystemAllocator>();
        assert_sync::<SystemAllocator>();
    }
}
