//! System allocator implementation
//!
//! Provides an allocator that wraps the system's default memory allocator.
//! This implementation delegates to the global system allocator while providing
//! additional safety checks and optimizations.
//!
//! # Safety
//!
//! This module wraps the standard library's System allocator (GlobalAlloc trait):
//! - All allocations delegate to std::alloc::System
//! - Zero-sized allocations handled specially (dangling pointer, no actual alloc)
//! - realloc optimization used when alignment matches (falls back to alloc+copy+dealloc)
//! - Thread safety guaranteed by underlying system allocator
//!
//! ## Invariants
//!
//! - Non-null pointers for non-zero sized allocations
//! - Dangling pointer for zero-sized allocations (no actual memory)
//! - Proper alignment via Layout enforcement
//! - Thread-safe operations (system allocator is inherently concurrent)

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

// SAFETY: SystemAllocator implements Allocator by delegating to System (GlobalAlloc).
// - System allocator provides thread-safe allocations
// - Zero-sized allocations handled specially (no actual allocation)
// - All pointers are properly aligned as guaranteed by System
unsafe impl Allocator for SystemAllocator {
    /// # Safety
    ///
    /// Caller must ensure:
    /// - `layout` has valid size and alignment (align is power of two)
    /// - `layout.size()` when rounded to align doesn't overflow isize
    /// - Returned pointer must be deallocated with same layout
    #[inline]
    unsafe fn allocate(&self, layout: Layout) -> AllocResult<NonNull<[u8]>> {
        if layout.size() == 0 {
            // Handle zero-sized allocations by returning a well-aligned dangling pointer
            let ptr = NonNull::<u8>::dangling();
            return Ok(NonNull::slice_from_raw_parts(ptr, 0));
        }

        // SAFETY: Delegating to System allocator (GlobalAlloc trait).
        // - layout is valid (checked by caller's contract)
        // - System.alloc returns null on failure (checked below)
        // - Returned pointer is properly aligned for layout
        let ptr = unsafe { System.alloc(layout) };

        if ptr.is_null() {
            Err(AllocError::allocation_failed(layout.size(), layout.align()))
        } else {
            // SAFETY: We just checked that ptr is not null.
            // System.alloc guarantees valid pointer or null.
            let non_null = unsafe { NonNull::new_unchecked(ptr) };
            Ok(NonNull::slice_from_raw_parts(non_null, layout.size()))
        }
    }

    /// # Safety
    ///
    /// Caller must ensure:
    /// - `ptr` was allocated by this allocator (System) with `layout`
    /// - `ptr` is currently allocated (not already deallocated)
    /// - `layout` matches the layout used for allocation
    #[inline]
    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        if layout.size() == 0 {
            return; // Nothing to deallocate for zero-sized allocations (dangling pointer)
        }

        // SAFETY: Delegating to System allocator's dealloc.
        // - ptr was allocated by System.alloc with this layout (caller's contract)
        // - layout matches the original allocation (caller's contract)
        // - System.dealloc handles deallocation safely
        unsafe { System.dealloc(ptr.as_ptr(), layout) };
    }

    /// # Safety
    ///
    /// Caller must ensure:
    /// - `ptr` was allocated by this allocator with `old_layout`
    /// - `old_layout` matches the original allocation layout
    /// - `new_layout.align()` equals `old_layout.align()`
    /// - On failure, `ptr` remains valid with `old_layout`
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
                // SAFETY: Using System.realloc for same-alignment resize.
                // - ptr was allocated by System.alloc (caller's contract)
                // - old_layout matches original allocation (caller's contract)
                // - Alignment unchanged (checked above)
                // - Returns null on failure (checked below)
                let new_ptr =
                    unsafe { System.realloc(ptr.as_ptr(), old_layout, new_layout.size()) };
                if !new_ptr.is_null() {
                    // SAFETY: We just checked new_ptr is not null.
                    let non_null = unsafe { NonNull::new_unchecked(new_ptr) };
                    return Ok(NonNull::slice_from_raw_parts(non_null, new_layout.size()));
                }
            }
        }

        // Fall back to allocate + copy + deallocate
        // SAFETY: new_layout is valid (caller's contract).
        let new_ptr = unsafe { self.allocate(new_layout)? };

        let copy_size = core::cmp::min(old_layout.size(), new_layout.size());
        if copy_size > 0 {
            // SAFETY: Copying data from old to new allocation.
            // - ptr is valid for old_layout.size() bytes (caller's contract)
            // - new_ptr is valid for new_layout.size() bytes (just allocated)
            // - copy_size is min of both sizes (no overflow)
            // - Regions don't overlap (new_ptr is freshly allocated)
            unsafe {
                core::ptr::copy_nonoverlapping(
                    ptr.as_ptr(),
                    new_ptr.as_ptr() as *mut u8,
                    copy_size,
                );
            }
        }

        // SAFETY: ptr and old_layout match original allocation (caller's contract).
        unsafe { self.deallocate(ptr, old_layout) };
        Ok(new_ptr)
    }
}

// SAFETY: SystemAllocator implements BulkAllocator using default trait methods.
// - Default bulk methods delegate to allocate/deallocate
// - System allocator has no special bulk optimizations
// - All safety contracts forwarded to base Allocator impl
unsafe impl BulkAllocator for SystemAllocator {}

// SAFETY: SystemAllocator is ThreadSafeAllocator because:
// - System allocator (GlobalAlloc) is inherently thread-safe
// - All platforms provide concurrent malloc/free implementations
// - No shared mutable state in SystemAllocator itself (zero-sized type)
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

// SAFETY: SystemAllocator is Send because:
// - It's a zero-sized type (no data to send)
// - System allocator is globally available on all threads
// - No thread-local state
unsafe impl Send for SystemAllocator {}

// SAFETY: SystemAllocator is Sync because:
// - It's a zero-sized type (no shared state)
// - System allocator handles concurrency internally (thread-safe malloc/free)
// - All operations delegate to GlobalAlloc which is Sync
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
