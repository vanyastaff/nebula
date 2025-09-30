//! Advanced allocator traits with optimized operations
//!
//! Provides a comprehensive set of traits for memory allocation with:
//! - Efficient default implementations
//! - Clear safety contracts
//! - Platform-aware constraints
//! - Full support for both stable and nightly Rust
//! - Enhanced error handling with detailed diagnostics
//! - Debug and profiling capabilities
//!
//! The system is built around several core traits:
//! - `Allocator`: Basic allocation/deallocation operations
//! - `BulkAllocator`: Optimized bulk allocation support
//! - `ThreadSafeAllocator`: Marker for thread-safe allocators
//! - `MemoryUsage`: Memory tracking capabilities
//! - `Resettable`: Allocator reset functionality

use core::alloc::Layout;
use core::ptr::NonNull;

use super::{AllocError, AllocErrorCode, AllocResult};

/// Validation of layout parameters
///
/// Performs comprehensive validation of allocation layout to catch
/// common errors early and provide detailed diagnostics.
#[inline]
fn validate_layout(layout: Layout) -> AllocResult<()> {
    // Check that alignment is a power of two
    if !layout.align().is_power_of_two() {
        return Err(AllocError::with_layout(AllocErrorCode::InvalidAlignment, layout));
    }

    // Check for zero-sized allocations (they're valid but need special handling)
    if layout.size() == 0 {
        return Ok(());
    }

    // Check for potential overflow when adding padding
    if layout.size() > isize::MAX as usize - (layout.align() - 1) {
        return Err(AllocError::with_layout(AllocErrorCode::SizeOverflow, layout));
    }

    Ok(())
}

/// Allocator trait with optimized resize operations
///
/// Provides fundamental memory allocation capabilities with optimized paths
/// for resizing existing allocations. All methods are unsafe as they deal
/// with raw pointers and have specific safety requirements.
///
/// # Safety Requirements
///
/// Implementors must ensure that:
/// - Returned pointers are valid for the requested lifetime
/// - Memory is properly aligned according to the layout
/// - Deallocation only occurs for previously allocated pointers
/// - Layout information matches between allocation and deallocation
pub unsafe trait Allocator {
    /// Allocates memory with the given layout
    ///
    /// This is the main entry point for memory allocation that includes
    /// validation and hook integration.
    ///
    /// # Safety
    /// - Returned pointer must be valid for reads and writes
    /// - Pointer must be properly aligned according to layout
    /// - Memory content is uninitialized and must be initialized before use
    ///
    /// # Errors
    /// - Returns error if memory cannot be allocated
    /// - Returns error for invalid layout parameters
    unsafe fn allocate(&self, layout: Layout) -> AllocResult<NonNull<[u8]>>;

    /// Deallocates memory at the given pointer with the specified layout
    ///
    /// # Safety
    /// - `ptr` must have been allocated by this allocator
    /// - `layout` must match the original allocation layout exactly
    /// - After this call, `ptr` becomes invalid and must not be used
    /// - Double-free is undefined behavior
    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout);

    /// Attempts to extend or shrink an existing allocation
    ///
    /// Provides an optimized path for resizing that may avoid copies when
    /// possible. This is the preferred method for changing allocation
    /// sizes.
    ///
    /// # Safety
    /// - `ptr` must have been allocated by this allocator
    /// - `old_layout` must match the original allocation layout exactly
    /// - `new_layout` must be valid for the reallocation
    /// - If successful, the old pointer becomes invalid
    ///
    /// # Implementation Notes
    /// The default implementation provides several optimizations:
    /// - Same size and alignment: returns the same pointer
    /// - Size changes: delegates to `grow` or `shrink`
    /// - Alignment changes: may require reallocation
    unsafe fn reallocate(
        &self,
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> AllocResult<NonNull<[u8]>> {
        validate_layout(new_layout)?;

        // Optimization: if size and alignment are the same, return the same pointer
        if old_layout.size() == new_layout.size() && old_layout.align() == new_layout.align() {
            return Ok(NonNull::slice_from_raw_parts(ptr, new_layout.size()));
        }

        let result = match new_layout.size().cmp(&old_layout.size()) {
            core::cmp::Ordering::Greater => unsafe { self.grow(ptr, old_layout, new_layout) },
            core::cmp::Ordering::Less => unsafe { self.shrink(ptr, old_layout, new_layout) },
            core::cmp::Ordering::Equal => {
                // Sizes are equal, but alignment might differ
                if new_layout.align() <= old_layout.align() {
                    Ok(NonNull::slice_from_raw_parts(ptr, new_layout.size()))
                } else {
                    // Need to reallocate for stricter alignment
                    unsafe { self.grow(ptr, old_layout, new_layout) }
                }
            },
        };

        result
    }

    /// Attempts to extend an existing allocation
    ///
    /// Default implementation allocates new memory and copies contents.
    /// Implementations should override this with more efficient approaches when
    /// possible, such as expanding in-place when memory is available.
    ///
    /// # Safety
    /// - Same requirements as `reallocate`
    /// - `new_layout.size()` should be greater than or equal to
    ///   `old_layout.size()`
    ///
    /// # Performance Notes
    /// This operation involves:
    /// 1. Allocating new memory with `new_layout`
    /// 2. Copying `old_layout.size()` bytes from old to new location
    /// 3. Deallocating the old memory
    unsafe fn grow(
        &self,
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> AllocResult<NonNull<[u8]>> {
        debug_assert!(new_layout.size() >= old_layout.size());

        let new_ptr = unsafe { self.allocate(new_layout)? };

        unsafe {
            #[cfg(feature = "nightly")]
            {
                core::intrinsics::copy_nonoverlapping(
                    ptr.as_ptr(),
                    new_ptr.as_mut_ptr(),
                    old_layout.size(),
                );
            }

            #[cfg(not(feature = "nightly"))]
            {
                core::ptr::copy_nonoverlapping(
                    ptr.as_ptr(),
                    new_ptr.as_ptr() as *mut u8,
                    old_layout.size(),
                );
            }
        }

        unsafe { self.deallocate(ptr, old_layout) };
        Ok(new_ptr)
    }

    /// Attempts to shrink an existing allocation
    ///
    /// Default implementation either returns original pointer (if alignment
    /// allows) or falls back to reallocation. Many allocators can implement
    /// this more efficiently by shrinking in-place.
    ///
    /// # Safety
    /// - Same requirements as `reallocate`
    /// - `new_layout.size()` should be less than or equal to
    ///   `old_layout.size()`
    ///
    /// # Alignment Considerations
    /// Shrinking can be done in-place only if the new alignment requirement
    /// is not stricter than the old one. If stricter alignment is required,
    /// reallocation is necessary.
    unsafe fn shrink(
        &self,
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> AllocResult<NonNull<[u8]>> {
        debug_assert!(new_layout.size() <= old_layout.size());

        // Can shrink in-place if:
        // 1. New alignment is not stricter than old alignment
        // 2. New size is not larger than old size (already checked above)
        if new_layout.align() <= old_layout.align() && new_layout.size() <= old_layout.size() {
            Ok(NonNull::slice_from_raw_parts(ptr, new_layout.size()))
        } else {
            // Need reallocation for stricter alignment
            unsafe { self.grow(ptr, old_layout, new_layout) }
        }
    }

    /// Returns maximum supported allocation size for this allocator
    ///
    /// Default implementation uses platform-specific maximum on nightly,
    /// falls back to `isize::MAX` on stable Rust.
    ///
    /// # Platform Notes
    /// - On 64-bit systems: typically around 2^63 - 1 bytes
    /// - On 32-bit systems: typically around 2^31 - 1 bytes
    /// - Some allocators may have smaller limits due to implementation
    ///   constraints
    fn max_allocation_size() -> usize {
        #[cfg(feature = "nightly")]
        {
            core::alloc::Layout::max_size()
        }

        #[cfg(not(feature = "nightly"))]
        {
            isize::MAX as usize
        }
    }

    /// Checks if the allocator supports zero-sized allocations
    ///
    /// Most allocators support zero-sized allocations by returning a non-null
    /// dangling pointer. Some specialized allocators might not support this.
    fn supports_zero_sized_allocs() -> bool {
        true
    }
}

/// Extension trait for allocators supporting bulk operations
///
/// Provides optimized operations for allocating multiple contiguous blocks.
/// This trait is useful for scenarios where you need to allocate many objects
/// of the same type efficiently.
///
/// # Use Cases
/// - Array allocations
/// - Buffer pools
/// - Batch processing scenarios
/// - Memory pool implementations
pub unsafe trait BulkAllocator: Allocator {
    /// Allocates multiple contiguous blocks with the same layout
    ///
    /// This method allocates a single contiguous memory region that can hold
    /// `count` objects of the specified layout. The total allocation size is
    /// `layout.size() * count` with alignment according to `layout.align()`.
    ///
    /// # Safety
    /// - Same safety requirements as `allocate`
    /// - Total size must be within platform allocation limits
    /// - The resulting memory can be treated as an array of `count` elements
    ///
    /// # Parameters
    /// - `layout`: Layout for each individual block
    /// - `count`: Number of blocks to allocate
    ///
    /// # Returns
    /// - `Ok(ptr)`: Pointer to the beginning of the contiguous memory region
    /// - `Err(AllocError)`: If allocation fails or parameters are invalid
    unsafe fn allocate_contiguous(
        &self,
        layout: Layout,
        count: usize,
    ) -> AllocResult<NonNull<[u8]>> {
        if count == 0 {
            // For zero count, return a valid dangling pointer
            let dangling = if layout.size() == 0 {
                #[cfg(feature = "nightly")]
                {
                    layout.dangling()
                }
                #[cfg(not(feature = "nightly"))]
                {
                    NonNull::<u8>::dangling()
                }
            } else {
                NonNull::<u8>::dangling()
            };
            return Ok(NonNull::slice_from_raw_parts(dangling, 0));
        }

        // Check for overflow when multiplying size by count
        let total_size = layout.size().checked_mul(count).ok_or_else(|| {
            AllocError::with_layout(AllocErrorCode::SizeOverflow, layout)
        })?;

        // Check against maximum allocation size
        if total_size > Self::max_allocation_size() {
            return Err(AllocError::with_layout(AllocErrorCode::ExceedsMaxSize, layout));
        }

        // Create layout for the total allocation
        let total_layout = Layout::from_size_align(total_size, layout.align())
            .map_err(|_| AllocError::with_layout(AllocErrorCode::InvalidLayout, layout))?;

        unsafe { self.allocate(total_layout) }
    }

    /// Deallocates contiguous blocks allocated with `allocate_contiguous`
    ///
    /// This method deallocates memory that was previously allocated using
    /// `allocate_contiguous` with the same layout and count parameters.
    ///
    /// # Safety
    /// - `ptr` must have been allocated by this allocator using
    ///   `allocate_contiguous`
    /// - `layout` and `count` must match the original allocation parameters
    ///   exactly
    /// - After this call, `ptr` becomes invalid and must not be used
    unsafe fn deallocate_contiguous(&self, ptr: NonNull<u8>, layout: Layout, count: usize) {
        if count == 0 {
            return; // Nothing to deallocate for zero count
        }

        // Reconstruct the total layout used for allocation
        let total_size = layout.size().saturating_mul(count);
        if let Ok(total_layout) = Layout::from_size_align(total_size, layout.align()) {
            unsafe { self.deallocate(ptr, total_layout) };
        }
        // If layout reconstruction fails, we can't safely deallocate
        // This should never happen if allocate_contiguous succeeded originally
    }

    /// Reallocates contiguous blocks to a new count
    ///
    /// Changes the number of allocated blocks while preserving existing data.
    /// When growing, the new blocks are uninitialized. When shrinking,
    /// excess data is discarded.
    ///
    /// # Safety
    /// - Same requirements as `reallocate`
    /// - `old_count` must match the original allocation
    /// - Data beyond `min(old_count, new_count)` blocks is unspecified
    unsafe fn reallocate_contiguous(
        &self,
        ptr: NonNull<u8>,
        layout: Layout,
        old_count: usize,
        new_count: usize,
    ) -> AllocResult<NonNull<[u8]>> {
        if old_count == new_count {
            // No change in count, return same pointer
            let total_size = layout.size().saturating_mul(new_count);
            return Ok(NonNull::slice_from_raw_parts(ptr, total_size));
        }

        // Calculate old and new total layouts
        let old_total_size = layout.size().checked_mul(old_count).ok_or_else(|| {
            AllocError::with_layout(AllocErrorCode::SizeOverflow, layout)
        })?;

        let new_total_size = layout.size().checked_mul(new_count).ok_or_else(|| {
            AllocError::with_layout(AllocErrorCode::SizeOverflow, layout)
        })?;

        let old_layout = Layout::from_size_align(old_total_size, layout.align())
            .map_err(|_| AllocError::with_layout(AllocErrorCode::InvalidLayout, layout))?;

        let new_layout = Layout::from_size_align(new_total_size, layout.align())
            .map_err(|_| AllocError::with_layout(AllocErrorCode::InvalidLayout, layout))?;

        unsafe { self.reallocate(ptr, old_layout, new_layout) }
    }
}

/// Thread-safe allocator marker trait
///
/// Indicates that an allocator can be safely shared between threads.
/// This is a marker trait that combines `Allocator` with `Send + Sync`
/// requirements.
///
/// # Safety
/// Implementors must ensure that all allocator operations are thread-safe:
/// - Concurrent allocations from different threads must be safe
/// - Concurrent deallocations from different threads must be safe
/// - Mixed concurrent allocations and deallocations must be safe
/// - Internal state must be properly synchronized
///
/// # Implementation Notes
/// Most allocators achieve thread safety through:
/// - Lock-free algorithms (preferred for performance)
/// - Mutex/RwLock protection (simpler to implement)
/// - Thread-local allocators (no sharing, inherently safe)
/// - Delegating to thread-safe system allocators
pub unsafe trait ThreadSafeAllocator: Allocator + Sync + Send {}

/// Memory usage reporting trait
///
/// Allows allocators to report their current memory usage statistics.
/// This is useful for monitoring, debugging, and implementing memory limits.
/// This trait focuses on basic capacity management rather than detailed
/// profiling.
///
/// For comprehensive statistics and profiling, see `StatisticsProvider`.
///
/// # Use Cases
/// - Memory usage monitoring and alerts
/// - Implementing allocation limits
/// - Basic performance profiling
/// - Resource management in constrained environments
pub trait MemoryUsage {
    /// Returns current allocated memory in bytes
    ///
    /// This should include all memory currently allocated by this allocator
    /// and not yet deallocated. Does not include memory overhead or
    /// internal allocator data structures unless specified by the
    /// implementation.
    fn used_memory(&self) -> usize;

    /// Returns total available memory in bytes
    ///
    /// Returns `None` if the allocator has no inherent memory limit
    /// (e.g., system allocators). Returns `Some(bytes)` if there's a
    /// specific limit (e.g., pool allocators, embedded systems).
    fn available_memory(&self) -> Option<usize>;

    /// Returns total memory capacity in bytes
    ///
    /// This is the sum of used and available memory. Returns `None`
    /// if the allocator has no inherent limit.
    fn total_memory(&self) -> Option<usize> {
        match (self.used_memory(), self.available_memory()) {
            (used, Some(available)) => Some(used + available),
            _ => None,
        }
    }

    /// Returns memory usage as a percentage (0.0 to 100.0)
    ///
    /// Returns `None` if total memory is unknown or zero.
    /// Useful for implementing memory pressure warnings.
    fn memory_usage_percent(&self) -> Option<f32> {
        self.total_memory().and_then(|total| {
            if total == 0 {
                Some(0.0)
            } else {
                Some((self.used_memory() as f32 / total as f32) * 100.0)
            }
        })
    }

    /// Checks if memory usage is above the specified percentage threshold
    ///
    /// Returns `None` if usage percentage cannot be determined.
    /// This is a convenience method for implementing memory pressure handling.
    fn is_memory_pressure(&self, threshold_percent: f32) -> Option<bool> {
        self.memory_usage_percent().map(|usage| usage >= threshold_percent)
    }

    /// Returns detailed memory usage information
    ///
    /// Provides a basic view of memory usage. Default implementation
    /// uses the other trait methods, but allocators can override this for
    /// more detailed reporting.
    fn memory_usage(&self) -> BasicMemoryUsage {
        BasicMemoryUsage {
            used: self.used_memory(),
            available: self.available_memory(),
            total: self.total_memory(),
            usage_percent: self.memory_usage_percent(),
        }
    }
}

/// Basic memory usage information for simple allocators
///
/// This is a simplified view of memory usage that focuses on capacity
/// management. For detailed metrics and profiling, use the `MemoryMetrics` type
/// from the stats module.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BasicMemoryUsage {
    /// Currently used memory in bytes
    pub used: usize,
    /// Available memory in bytes (None if unlimited)
    pub available: Option<usize>,
    /// Total memory capacity in bytes (None if unlimited)
    pub total: Option<usize>,
    /// Memory usage as percentage (None if cannot be calculated)
    pub usage_percent: Option<f32>,
}

impl core::fmt::Display for BasicMemoryUsage {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "used: {} bytes", self.used)?;

        if let Some(total) = self.total {
            write!(f, ", total: {} bytes", total)?;
        }

        if let Some(percent) = self.usage_percent {
            write!(f, " ({:.1}%)", percent)?;
        }

        Ok(())
    }
}

/// Resettable allocator trait
///
/// Allocators implementing this trait can be reset, invalidating all previous
/// allocations. This is useful for:
/// - Arena/bump allocators that can reset to beginning
/// - Pool allocators that can return all memory to the pool
/// - Temporary allocators for scoped operations
/// - Memory debugging scenarios
///
/// # Safety Considerations
/// Resetting an allocator is an inherently dangerous operation because it
/// invalidates all existing allocations. Users must ensure no live references
/// exist before reset.
pub trait Resettable {
    /// Resets the allocator, invalidating all previous allocations
    ///
    /// # Safety
    /// - All pointers from previous allocations become invalid immediately
    /// - Using invalidated pointers results in undefined behavior
    /// - Caller must ensure no live references exist before calling this method
    /// - After reset, the allocator should be ready for new allocations
    ///
    /// # Implementation Notes
    /// Implementations should:
    /// 1. Mark all allocated memory as available
    /// 2. Reset internal state to initial condition
    /// 3. Ensure the allocator is ready for new allocations
    unsafe fn reset(&self);

    /// Checks if the allocator can be safely reset
    ///
    /// Default implementation always returns `true`, but specific allocators
    /// may have conditions where reset is not possible or advisable.
    ///
    /// # Examples of when reset might not be safe:
    /// - Allocator is currently being used by other threads
    /// - Allocator has active external references
    /// - Allocator is in an inconsistent state
    fn can_reset(&self) -> bool {
        true
    }

    /// Resets the allocator only if it's safe to do so
    ///
    /// This is a safer alternative to `reset()` that checks `can_reset()`
    /// first. Returns `true` if reset was performed, `false` if it was
    /// skipped.
    ///
    /// # Safety
    /// Same safety requirements as `reset()`, but only applies if reset is
    /// actually performed.
    unsafe fn try_reset(&self) -> bool {
        if self.can_reset() {
            unsafe { self.reset() };
            true
        } else {
            false
        }
    }
}

// ============================================================================
// Blanket implementations for references
// ============================================================================

/// Blanket implementation of Allocator for references
///
/// This allows using `&T` where `T: Allocator` is expected, which is convenient
/// for many use cases where you don't need to own the allocator.
unsafe impl<T: Allocator + ?Sized> Allocator for &T {
    unsafe fn allocate(&self, layout: Layout) -> AllocResult<NonNull<[u8]>> {
        unsafe { (**self).allocate(layout) }
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        unsafe { (**self).deallocate(ptr, layout) }
    }

    unsafe fn reallocate(
        &self,
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> AllocResult<NonNull<[u8]>> {
        unsafe { (**self).reallocate(ptr, old_layout, new_layout) }
    }

    unsafe fn grow(
        &self,
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> AllocResult<NonNull<[u8]>> {
        unsafe { (**self).grow(ptr, old_layout, new_layout) }
    }

    unsafe fn shrink(
        &self,
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> AllocResult<NonNull<[u8]>> {
        unsafe { (**self).shrink(ptr, old_layout, new_layout) }
    }
}

/// Blanket implementation of BulkAllocator for references
unsafe impl<T: BulkAllocator + ?Sized> BulkAllocator for &T {
    unsafe fn allocate_contiguous(
        &self,
        layout: Layout,
        count: usize,
    ) -> AllocResult<NonNull<[u8]>> {
        unsafe { (**self).allocate_contiguous(layout, count) }
    }

    unsafe fn deallocate_contiguous(&self, ptr: NonNull<u8>, layout: Layout, count: usize) {
        unsafe { (**self).deallocate_contiguous(ptr, layout, count) }
    }

    unsafe fn reallocate_contiguous(
        &self,
        ptr: NonNull<u8>,
        layout: Layout,
        old_count: usize,
        new_count: usize,
    ) -> AllocResult<NonNull<[u8]>> {
        unsafe { (**self).reallocate_contiguous(ptr, layout, old_count, new_count) }
    }
}

/// Blanket implementation of MemoryUsage for references
impl<T: MemoryUsage + ?Sized> MemoryUsage for &T {
    fn used_memory(&self) -> usize {
        (**self).used_memory()
    }

    fn available_memory(&self) -> Option<usize> {
        (**self).available_memory()
    }

    fn total_memory(&self) -> Option<usize> {
        (**self).total_memory()
    }

    fn memory_usage_percent(&self) -> Option<f32> {
        (**self).memory_usage_percent()
    }

    fn memory_usage(&self) -> BasicMemoryUsage {
        (**self).memory_usage()
    }
}

/// Blanket implementation of Resettable for references
impl<T: Resettable + ?Sized> Resettable for &T {
    unsafe fn reset(&self) {
        unsafe { (**self).reset() }
    }

    fn can_reset(&self) -> bool {
        (**self).can_reset()
    }

    unsafe fn try_reset(&self) -> bool {
        unsafe { (**self).try_reset() }
    }
}
