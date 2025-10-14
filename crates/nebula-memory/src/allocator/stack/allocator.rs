//! Main stack allocator implementation
//!
//! # Safety
//!
//! This module implements a thread-safe LIFO stack allocator:
//! - Memory buffer wrapped in `SyncUnsafeCell` for interior mutability
//! - Atomic top pointer for lock-free allocation via CAS
//! - Deallocations only supported for most recent allocation (stack discipline)
//! - Markers enable batch deallocation by restoring stack position
//!
//! ## Invariants
//!
//! - All pointers within [`start_addr`, `end_addr`) bounds
//! - top pointer moves monotonically upward (or resets to start)
//! - Allocations aligned properly via `align_up`
//! - CAS loop ensures exclusive ownership of allocated ranges
//! - `try_pop` verifies pointer matches most recent allocation
//! - `restore_to_marker` validates marker is within valid range
//!
//! ## Thread Safety
//!
//! - Atomic top pointer with `AcqRel` ordering for allocation CAS
//! - Acquire loads ensure visibility of previous allocations
//! - Release stores ensure new allocations visible to other threads
//! - Statistics tracked with atomic operations (Relaxed ordering)
//!
//! ## Memory Ordering
//!
//! - Acquire: Loading top pointer (read barrier, see previous writes)
//! - Release: Storing top pointer (write barrier, publish changes)
//! - `AcqRel`: CAS success (both acquire and release semantics)
//! - Relaxed: Statistics updates (ordering not critical)

use core::alloc::Layout;
use core::cell::UnsafeCell;
use core::ptr::{self, NonNull};
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use super::{StackConfig, StackMarker};
use crate::allocator::{
    AllocError, AllocResult, Allocator, AllocatorStats, MemoryUsage, Resettable, StatisticsProvider,
};
use crate::utils::{Backoff, align_up, atomic_max};

/// Thread-safe wrapper for memory buffer with interior mutability
#[repr(transparent)]
struct SyncUnsafeCell<T: ?Sized>(UnsafeCell<T>);

// SAFETY: SyncUnsafeCell provides interior mutability with external synchronization.
// - UnsafeCell allows shared mutation via raw pointers
// - Atomic top pointer ensures allocations don't overlap
// - CAS operations provide exclusive access to allocated ranges
// - repr(transparent) ensures same layout as UnsafeCell<T>
unsafe impl<T: ?Sized> Sync for SyncUnsafeCell<T> {}

// SAFETY: SyncUnsafeCell can be sent between threads if T: Send.
// - Wraps UnsafeCell<T>, inherits Send requirement from T
// - No thread-local state or thread-specific invariants
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

/// Stack allocator that supports LIFO allocation and deallocation
///
/// This allocator maintains a stack-like structure where memory can only
/// be deallocated in reverse order of allocation. It's more flexible than
/// a bump allocator but still very efficient.
///
/// # Memory Layout
/// ```text
/// [start]----[alloc1]----[alloc2]----[alloc3]----[top]----[free]----[end]
///             <------ allocated ------>         <-- available -->
/// ```
///
/// Deallocations must happen in reverse order: alloc3, then alloc2, then alloc1.
pub struct StackAllocator {
    /// Owned memory buffer with interior mutability
    memory: Box<SyncUnsafeCell<[u8]>>,

    /// Configuration
    config: StackConfig,

    /// Start of the memory region (cached for performance)
    start_addr: usize,

    /// Current top of stack (atomic for thread safety)
    top: AtomicUsize,

    /// End address (cached for performance)
    end_addr: usize,

    /// Capacity for convenience
    capacity: usize,

    /// Statistics (optional, only tracked if enabled)
    total_allocs: AtomicU32,
    total_deallocs: AtomicU32,
    peak_usage: AtomicUsize,
}

impl StackAllocator {
    /// Creates a new stack allocator with custom configuration
    pub fn with_config(capacity: usize, config: StackConfig) -> AllocResult<Self> {
        if capacity == 0 {
            return Err(AllocError::invalid_layout("capacity cannot be zero"));
        }

        let mut vec = vec![0u8; capacity];

        // Fill with alloc pattern if debugging
        if let Some(pattern) = config.alloc_pattern {
            vec.fill(pattern);
        }

        // Wrap in SyncUnsafeCell for interior mutability
        let boxed_slice = vec.into_boxed_slice();
        let len = boxed_slice.len();
        let ptr = Box::into_raw(boxed_slice).cast::<u8>();
        // SAFETY: Transmuting Box<[u8]> to Box<SyncUnsafeCell<[u8]>>.
        // - SyncUnsafeCell is repr(transparent), has same layout as UnsafeCell<T>
        // - UnsafeCell<T> is repr(transparent), has same layout as T
        // - Therefore SyncUnsafeCell<[u8]> has same layout as [u8]
        // - Box ownership transferred via into_raw/from_raw
        // - Pointer cast is safe due to layout guarantee
        // - Length preserved correctly
        let memory: Box<SyncUnsafeCell<[u8]>> = unsafe {
            Box::from_raw(core::ptr::slice_from_raw_parts_mut(ptr, len) as *mut SyncUnsafeCell<[u8]>)
        };

        // SAFETY: Getting raw pointer from memory buffer.
        // - memory.get() returns *mut [u8] pointing to buffer
        // - Dereferencing to get slice, then as_ptr() to get element pointer
        // - Casting to usize for address arithmetic
        // - Buffer is valid (just allocated above)
        let start_addr = unsafe { (*memory.get()).as_ptr() as usize };
        let end_addr = start_addr + capacity;

        Ok(Self {
            memory,
            config,
            start_addr,
            top: AtomicUsize::new(start_addr),
            end_addr,
            capacity,
            total_allocs: AtomicU32::new(0),
            total_deallocs: AtomicU32::new(0),
            peak_usage: AtomicUsize::new(0),
        })
    }

    /// Creates a new stack allocator with default configuration
    pub fn new(capacity: usize) -> AllocResult<Self> {
        Self::with_config(capacity, StackConfig::default())
    }

    /// Creates a production-optimized stack allocator
    pub fn production(capacity: usize) -> AllocResult<Self> {
        Self::with_config(capacity, StackConfig::production())
    }

    /// Creates a debug-optimized stack allocator
    pub fn debug(capacity: usize) -> AllocResult<Self> {
        Self::with_config(capacity, StackConfig::debug())
    }

    /// Creates a performance-optimized stack allocator
    pub fn performance(capacity: usize) -> AllocResult<Self> {
        Self::with_config(capacity, StackConfig::performance())
    }

    /// Creates a stack allocator from a pre-allocated boxed slice
    #[must_use]
    pub fn from_boxed_slice(memory: Box<[u8]>) -> Self {
        let capacity = memory.len();

        // Wrap in SyncUnsafeCell for interior mutability
        let len = memory.len();
        let ptr = Box::into_raw(memory).cast::<u8>();
        // SAFETY: Transmuting Box<[u8]> to Box<SyncUnsafeCell<[u8]>>.
        // - SyncUnsafeCell is repr(transparent) over UnsafeCell
        // - UnsafeCell is repr(transparent) over T
        // - Therefore SyncUnsafeCell<[u8]> has same layout as [u8]
        // - Box ownership transferred via into_raw/from_raw
        // - Pointer cast safe due to layout guarantee
        let memory: Box<SyncUnsafeCell<[u8]>> = unsafe {
            Box::from_raw(core::ptr::slice_from_raw_parts_mut(ptr, len) as *mut SyncUnsafeCell<[u8]>)
        };

        // SAFETY: Getting raw pointer from memory buffer.
        // - memory.get() returns *mut [u8] to buffer
        // - Dereferencing to access slice, then as_ptr() for element pointer
        // - Casting to usize for address arithmetic
        let start_addr = unsafe { (*memory.get()).as_ptr() as usize };
        let end_addr = start_addr + capacity;

        Self {
            memory,
            config: StackConfig::default(),
            start_addr,
            top: AtomicUsize::new(start_addr),
            end_addr,
            capacity,
            total_allocs: AtomicU32::new(0),
            total_deallocs: AtomicU32::new(0),
            peak_usage: AtomicUsize::new(0),
        }
    }

    /// Returns the total capacity of the allocator
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Returns the amount of memory currently allocated
    pub fn used(&self) -> usize {
        let top = self.top.load(Ordering::Acquire);
        top.saturating_sub(self.start_addr)
    }

    /// Returns the amount of memory available for allocation
    pub fn available(&self) -> usize {
        self.capacity().saturating_sub(self.used())
    }

    /// Returns the current top of stack position
    pub fn current_top(&self) -> usize {
        self.top.load(Ordering::Acquire)
    }

    /// Creates a marker at the current stack position
    ///
    /// This marker can be used later to restore the stack to this position,
    /// effectively deallocating all allocations made after this point.
    pub fn mark(&self) -> StackMarker {
        StackMarker {
            position: self.top.load(Ordering::Acquire),
        }
    }

    /// Restores the stack to a previous marker position
    ///
    /// This deallocates all allocations made after the marker was created.
    ///
    /// # Safety
    /// - The marker must be valid (created by this allocator)
    /// - All pointers to memory allocated after the marker become invalid
    /// - The marker position must not be in the future (greater than current
    ///   top)
    pub unsafe fn restore_to_marker(&self, marker: StackMarker) -> Result<(), AllocError> {
        let current_top = self.top.load(Ordering::Acquire);

        if marker.position > current_top {
            return Err(AllocError::invalid_layout("invalid layout")); // marker from the future
        }
        if marker.position < self.start_addr || marker.position > self.end_addr {
            return Err(AllocError::invalid_layout("invalid layout")); // out of bounds
        }

        self.top.store(marker.position, Ordering::Release);
        Ok(())
    }

    /// Pops the most recent allocation if it matches the given pointer and
    /// layout
    ///
    /// This provides a safe way to deallocate the most recent allocation.
    /// Returns true if the deallocation was successful, false if the pointer
    /// doesn't match the most recent allocation.
    ///
    /// # Safety
    /// - The pointer must have been allocated by this allocator
    /// - The layout must match the original allocation layout
    pub unsafe fn try_pop(&self, ptr: NonNull<u8>, layout: Layout) -> bool {
        // SAFETY: Attempting to pop most recent allocation.
        // - Caller guarantees ptr was allocated by this allocator (contract)
        // - Caller guarantees layout matches original allocation (contract)
        unsafe {
            let current_top = self.top.load(Ordering::Acquire);
            let expected_start = current_top.saturating_sub(layout.size());

            // Check if this pointer matches the most recent allocation
            if ptr.as_ptr() as usize == align_up(expected_start, layout.align()) {
                // Fill with dealloc pattern if debugging
                // SAFETY: Writing pattern to allocated memory before deallocation.
                // - ptr is valid (verified to match most recent allocation above)
                // - layout.size() is the original allocation size
                // - Memory about to be freed, overwriting is safe
                if let Some(pattern) = self.config.dealloc_pattern {
                    ptr::write_bytes(ptr.as_ptr(), pattern, layout.size());
                }

                // This is the most recent allocation, we can safely pop it
                self.top.store(expected_start, Ordering::Release);

                // Update statistics
                if self.config.track_stats {
                    self.total_deallocs.fetch_add(1, Ordering::Relaxed);
                }

                true
            } else {
                // Not the most recent allocation, cannot pop
                false
            }
        }
    }

    /// Aligns a size up to the specified alignment
    #[inline]
    fn align_up(size: usize, align: usize) -> usize {
        (size + align - 1) & !(align - 1)
    }

    /// Attempts to allocate memory with adaptive backoff and statistics tracking
    fn try_allocate(&self, size: usize, align: usize) -> Option<NonNull<u8>> {
        let mut backoff = if self.config.use_backoff {
            Some(Backoff::new())
        } else {
            None
        };
        let mut attempts = 0;

        loop {
            // Check retry limit
            if attempts >= self.config.max_retries {
                return None;
            }

            let current_top = self.top.load(Ordering::Acquire);
            let aligned_addr = align_up(current_top, align);
            let new_top = aligned_addr.checked_add(size)?;

            // Check if we have enough space
            if new_top > self.end_addr {
                return None;
            }

            // Try to update the top atomically
            let result = if attempts == 0 {
                self.top
                    .compare_exchange(current_top, new_top, Ordering::AcqRel, Ordering::Acquire)
            } else {
                self.top.compare_exchange_weak(
                    current_top,
                    new_top,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
            };

            if result.is_ok() {
                // Update statistics if tracking is enabled
                if self.config.track_stats {
                    self.total_allocs.fetch_add(1, Ordering::Relaxed);
                    let current_used = new_top - self.start_addr;
                    atomic_max(&self.peak_usage, current_used);
                }

                // Fill with alloc pattern if debugging
                // SAFETY: Writing pattern to newly allocated memory.
                // - CAS success guarantees exclusive ownership of [aligned_addr, new_top)
                // - aligned_addr is valid (within [start_addr, end_addr), checked above)
                // - size bytes available (new_top = aligned_addr + size, validated)
                // - Memory is ours to initialize
                if let Some(pattern) = self.config.alloc_pattern {
                    unsafe {
                        ptr::write_bytes(aligned_addr as *mut u8, pattern, size);
                    }
                }

                // Convert address to NonNull with explicit check
                // aligned_addr is non-zero (start_addr > 0), but use explicit check for safety
                return NonNull::new(aligned_addr as *mut u8);
            }
            // Increment attempts and use backoff
            attempts += 1;
            if let Some(ref mut b) = backoff {
                b.spin();
            }
        }
    }

    /// Convenience constructors
    pub fn small() -> AllocResult<Self> {
        Self::new(32 * 1024)
    } // 32KB
    pub fn medium() -> AllocResult<Self> {
        Self::new(512 * 1024)
    } // 512KB
    pub fn large() -> AllocResult<Self> {
        Self::new(8 * 1024 * 1024)
    } // 8MB
}

// SAFETY: StackAllocator implements Allocator with stack discipline.
// - allocate returns valid, aligned, non-overlapping pointers
// - Atomic CAS ensures exclusive ownership of allocated ranges
// - deallocate only works for most recent allocation (stack LIFO)
// - All operations properly synchronized via atomic top pointer
unsafe impl Allocator for StackAllocator {
    unsafe fn allocate(&self, layout: Layout) -> AllocResult<NonNull<[u8]>> {
        if layout.size() == 0 {
            // Handle zero-sized allocations
            let ptr = NonNull::<u8>::dangling();
            return Ok(NonNull::slice_from_raw_parts(ptr, 0));
        }

        if let Some(ptr) = self.try_allocate(layout.size(), layout.align()) {
            Ok(NonNull::slice_from_raw_parts(ptr, layout.size()))
        } else {
            Err(AllocError::allocation_failed(layout.size(), layout.align()))
        }
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        // Try to pop if this is the most recent allocation
        // This is a "best effort" - if it's not the most recent, it's a no-op
        // SAFETY: Forwarding to try_pop.
        // - ptr was allocated by this allocator (caller contract)
        // - layout matches original allocation (caller contract)
        // - try_pop validates pointer matches most recent allocation
        unsafe { self.try_pop(ptr, layout) };
    }

    unsafe fn reallocate(
        &self,
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> AllocResult<NonNull<[u8]>> {
        // Check if this is the most recent allocation and we can extend in-place
        let current_top = self.top.load(Ordering::Acquire);
        let ptr_addr = ptr.as_ptr() as usize;
        let expected_start = current_top.saturating_sub(old_layout.size());

        if ptr_addr == Self::align_up(expected_start, old_layout.align())
            && new_layout.align() <= old_layout.align()
            && new_layout.size() >= old_layout.size()
        {
            // This is the most recent allocation, try to extend in-place
            let additional_size = new_layout.size() - old_layout.size();
            let new_top = current_top + additional_size;

            if new_top <= self.end_addr {
                // We have space to extend
                if self
                    .top
                    .compare_exchange(current_top, new_top, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
                {
                    return Ok(NonNull::slice_from_raw_parts(ptr, new_layout.size()));
                }
            }
        }

        // Fall back to allocate + copy + deallocate
        // SAFETY: Allocating new memory with new_layout.
        // - new_layout is valid (validated by caller)
        // - allocate returns valid pointer or error
        let new_ptr = unsafe { self.allocate(new_layout)? };

        let copy_size = core::cmp::min(old_layout.size(), new_layout.size());
        if copy_size > 0 {
            // SAFETY: Copying data from old to new allocation.
            // - ptr is valid for reads of old_layout.size() bytes (caller contract)
            // - new_ptr is valid for writes of new_layout.size() bytes (just allocated)
            // - copy_size <= min(old, new), so both accesses in bounds
            // - Regions may overlap if old allocation still on stack, but we use
            //   copy_nonoverlapping which requires non-overlap. However, if allocate
            //   succeeded, new_ptr is beyond old ptr (stack grows up), so no overlap.
            unsafe {
                ptr::copy_nonoverlapping(ptr.as_ptr(), new_ptr.as_ptr().cast::<u8>(), copy_size);
            }
        }

        // Try to deallocate old memory (will succeed if it's still the most recent)
        // SAFETY: Deallocating old memory.
        // - ptr was allocated by this allocator (caller contract)
        // - old_layout matches original allocation (caller contract)
        // - Data has been copied to new location
        // - deallocate is best-effort for stack allocator (may be no-op)
        unsafe { self.deallocate(ptr, old_layout) };

        Ok(new_ptr)
    }
}

impl MemoryUsage for StackAllocator {
    fn used_memory(&self) -> usize {
        self.used()
    }

    fn available_memory(&self) -> Option<usize> {
        Some(self.available())
    }

    fn total_memory(&self) -> Option<usize> {
        Some(self.capacity())
    }
}

impl Resettable for StackAllocator {
    unsafe fn reset(&self) {
        // SAFETY: Resetting stack to initial state.
        // - Caller guarantees no outstanding pointers to allocated memory (contract)
        // - Resetting top to start_addr invalidates all previous allocations
        // - Release ordering ensures reset is visible to all threads
        self.top.store(self.start_addr, Ordering::Release);

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

impl StatisticsProvider for StackAllocator {
    fn statistics(&self) -> AllocatorStats {
        AllocatorStats {
            allocated_bytes: self.used(),
            peak_allocated_bytes: if self.config.track_stats {
                self.peak_usage.load(Ordering::Relaxed)
            } else {
                self.used()
            },
            allocation_count: self.total_allocs.load(Ordering::Relaxed) as usize,
            deallocation_count: self.total_deallocs.load(Ordering::Relaxed) as usize,
            reallocation_count: 0,
            failed_allocations: 0,
            total_bytes_allocated: 0, // Stack doesn't track this granularly
            total_bytes_deallocated: 0,
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
// SAFETY: StackAllocator can be sent between threads.
// - All fields are Send (Box, AtomicUsize, AtomicU32, primitive types)
// - No thread-local state or thread-specific invariants
// - Owned memory buffer (Box) is Send
unsafe impl Send for StackAllocator {}

// SAFETY: StackAllocator can be shared between threads.
// - All allocations synchronized via atomic top pointer with CAS
// - Memory buffer wrapped in SyncUnsafeCell (interior mutability)
// - CAS operations provide exclusive access to allocated ranges
// - Statistics tracked with atomic operations
// - All operations use proper memory ordering (Acquire/Release/AcqRel)
unsafe impl Sync for StackAllocator {}
