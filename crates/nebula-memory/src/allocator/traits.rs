//! Advanced allocator traits with optimized operations
//!
//! Provides a comprehensive set of traits for memory allocation with:
//! - Efficient default implementations
//! - Clear safety contracts
//! - Platform-aware constraints
//! - Full support for both stable and nightly Rust
//! - Enhanced error handling with detailed diagnostics
//! - Debug and profiling capabilities

use core::alloc::Layout;
use core::ptr::NonNull;

use super::{AllocError, AllocErrorKind, AllocResult};

// Import safe utilities
use crate::utils::{is_power_of_two, BarrierType, memory_barrier_ex};

/// Validation of layout parameters
///
/// Performs comprehensive validation of allocation layout to catch
/// common errors early and provide detailed diagnostics.
#[inline]
fn validate_layout(layout: Layout) -> AllocResult<()> {
    // Use safe utility for power of two check
    if !is_power_of_two(layout.align()) {
        return Err(AllocError::with_kind_and_layout(AllocErrorKind::InvalidAlignment, layout));
    }

    // Check for zero-sized allocations (they're valid but need special handling)
    if layout.size() == 0 {
        // Additional check: ensure alignment is reasonable for zero-sized
        if layout.align() > (1 << 29) { // 512MB alignment is unreasonable
            return Err(AllocError::with_kind_and_layout(AllocErrorKind::InvalidAlignment, layout));
        }
        return Ok(());
    }

    // Check for potential overflow when adding padding
    if layout.size() > isize::MAX as usize - (layout.align() - 1) {
        return Err(AllocError::with_kind_and_layout(AllocErrorKind::SizeOverflow, layout));
    }

    Ok(())
}

/// Helper for safe memory copy operations with proper validation
///
/// # Safety
/// - `src` must be valid for reads of `size` bytes
/// - `dst` must be valid for writes of `size` bytes
/// - `src` and `dst` must not overlap
#[inline]
unsafe fn safe_copy_memory(src: *const u8, dst: *mut u8, size: usize) {
    if size == 0 {
        return;
    }

    // Basic sanity checks in debug mode
    debug_assert!(!src.is_null(), "Source pointer is null");
    debug_assert!(!dst.is_null(), "Destination pointer is null");
    debug_assert!(
        src as usize >= dst as usize + size || dst as usize >= src as usize + size,
        "Source and destination memory regions overlap"
    );

    // For small copies, just use direct copy (avoiding overhead)
    if size <= 64 {
        unsafe {
            core::ptr::copy_nonoverlapping(src, dst, size);
        }
        return;
    }

    // Try to use safe memory operations for larger copies
    #[cfg(feature = "std")]
    {
        use crate::utils::MEMORY;

        // Only attempt safe copy for reasonable sizes
        if size <= 4096 {
            // SAFETY: Caller guarantees pointers are valid
            unsafe {
                let src_slice = core::slice::from_raw_parts(src, size);
                let dst_slice = core::slice::from_raw_parts_mut(dst, size);

                // Try safe copy, fallback to direct copy if it fails
                if MEMORY.copy_slices(src_slice, dst_slice).is_err() {
                    core::ptr::copy_nonoverlapping(src, dst, size);
                }
            }
        } else {
            // For large copies, use direct copy
            unsafe {
                core::ptr::copy_nonoverlapping(src, dst, size);
            }
        }
    }

    #[cfg(not(feature = "std"))]
    {
        // Direct copy in no_std
        unsafe {
            core::ptr::copy_nonoverlapping(src, dst, size);
        }
    }
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
    #[track_caller]
    unsafe fn allocate(&self, layout: Layout) -> AllocResult<NonNull<[u8]>>;

    /// Deallocates memory at the given pointer with the specified layout
    ///
    /// # Safety
    /// - `ptr` must have been allocated by this allocator
    /// - `layout` must match the original allocation layout exactly
    /// - After this call, `ptr` becomes invalid and must not be used
    /// - Double-free is undefined behavior
    #[track_caller]
    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout);

    /// Checks if the allocator can grow an allocation in-place
    ///
    /// Returns true if the allocator might be able to expand the allocation
    /// without moving it. This is a hint and not a guarantee.
    fn can_grow_in_place(&self, _old_layout: Layout, _new_layout: Layout) -> bool {
        false
    }

    /// Checks if the allocator can shrink an allocation in-place
    ///
    /// Returns true if the allocator might be able to shrink the allocation
    /// without moving it. This is a hint and not a guarantee.
    fn can_shrink_in_place(&self, _old_layout: Layout, _new_layout: Layout) -> bool {
        false
    }

    /// Attempts to grow an allocation in-place without moving it
    ///
    /// # Safety
    /// - Same requirements as `grow`
    /// - Returns true if successful, false if reallocation is needed
    unsafe fn grow_in_place(
        &self,
        _ptr: NonNull<u8>,
        _old_layout: Layout,
        _new_layout: Layout,
    ) -> bool {
        false
    }

    /// Attempts to shrink an allocation in-place without moving it
    ///
    /// # Safety
    /// - Same requirements as `shrink`
    /// - Returns true if successful, false if reallocation is needed
    unsafe fn shrink_in_place(
        &self,
        _ptr: NonNull<u8>,
        _old_layout: Layout,
        _new_layout: Layout,
    ) -> bool {
        false
    }

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
        debug_assert!(!ptr.as_ptr().is_null(), "Null pointer passed to reallocate");
        debug_assert!(old_layout.size() > 0 || old_layout.size() == 0, "Invalid old layout");

        // Handle zero-sized allocations early
        if new_layout.size() == 0 {
            unsafe { self.deallocate(ptr, old_layout) };
            return Ok(NonNull::slice_from_raw_parts(NonNull::dangling(), 0));
        }

        // Optimization: if size and alignment are the same, return the same pointer
        if old_layout.size() == new_layout.size() && old_layout.align() == new_layout.align() {
            return Ok(NonNull::slice_from_raw_parts(ptr, new_layout.size()));
        }

        // Memory barrier to ensure all previous writes are visible
        memory_barrier_ex(BarrierType::Release);

        // Try in-place operations first
        let result = match new_layout.size().cmp(&old_layout.size()) {
            core::cmp::Ordering::Greater => {
                // Try in-place growth first
                if self.can_grow_in_place(old_layout, new_layout) {
                    if unsafe { self.grow_in_place(ptr, old_layout, new_layout) } {
                        Ok(NonNull::slice_from_raw_parts(ptr, new_layout.size()))
                    } else {
                        unsafe { self.grow(ptr, old_layout, new_layout) }
                    }
                } else {
                    unsafe { self.grow(ptr, old_layout, new_layout) }
                }
            }
            core::cmp::Ordering::Less => {
                // Try in-place shrink first
                if self.can_shrink_in_place(old_layout, new_layout) {
                    if unsafe { self.shrink_in_place(ptr, old_layout, new_layout) } {
                        Ok(NonNull::slice_from_raw_parts(ptr, new_layout.size()))
                    } else {
                        unsafe { self.shrink(ptr, old_layout, new_layout) }
                    }
                } else {
                    unsafe { self.shrink(ptr, old_layout, new_layout) }
                }
            }
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

        // Memory barrier to ensure we see all changes
        memory_barrier_ex(BarrierType::Acquire);

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
        debug_assert!(new_layout.size() >= old_layout.size(),
                      "Grow operation requires new size >= old size");
        debug_assert!(!ptr.as_ptr().is_null(), "Null pointer in grow operation");

        let new_ptr = unsafe { self.allocate(new_layout)? };

        // Additional validation in debug mode
        #[cfg(debug_assertions)]
        {
            debug_assert!(!new_ptr.as_ptr().is_null(), "Allocation returned null pointer");
            debug_assert_eq!(new_ptr.cast::<u8>().as_ptr() as usize % new_layout.align(), 0,
                             "New pointer not properly aligned");
        }

        // Use safe memory copy helper
        unsafe {
            safe_copy_memory(ptr.as_ptr(), new_ptr.as_ptr() as *mut u8, old_layout.size());
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
    /// Default implementation uses platform-specific information when available
    ///
    /// # Platform Notes
    /// - On 64-bit systems: typically around 2^47 bytes (128TB)
    /// - On 32-bit systems: typically around 2^31 - 1 bytes (2GB)
    /// - Embedded systems may have much smaller limits
    fn max_allocation_size() -> usize {
        // Conservative default for no_std environments
        #[cfg(not(feature = "std"))]
        {
            // Conservative limit for embedded/constrained environments
            const EMBEDDED_MAX_ALLOC: usize = 16 * 1024 * 1024; // 16MB
            EMBEDDED_MAX_ALLOC.min(isize::MAX as usize)
        }

        #[cfg(feature = "std")]
        {
            use crate::utils::PlatformInfo;

            if let Some(total_mem) = PlatformInfo::current().total_memory {
                // Leave some headroom for the system and allocator overhead
                let headroom = total_mem / 8; // 12.5% headroom
                (total_mem - headroom).min(isize::MAX as usize)
            } else {
                // Fallback to platform-specific reasonable limits
                #[cfg(target_pointer_width = "64")]
                { 1 << 47 } // 128TB on 64-bit systems

                #[cfg(target_pointer_width = "32")]
                { 1 << 30 } // 1GB on 32-bit systems

                #[cfg(not(any(target_pointer_width = "64", target_pointer_width = "32")))]
                { 16 * 1024 * 1024 } // 16MB fallback
            }
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
            return Ok(NonNull::slice_from_raw_parts(NonNull::dangling(), 0));
        }

        // Более прямое использование checked_mul для консистентности
        let total_size = layout.size().checked_mul(count).ok_or_else(|| {
            AllocError::with_kind_and_layout(AllocErrorKind::SizeOverflow, layout)
        })?;

        let total_layout = Layout::from_size_align(total_size, layout.align())
            .map_err(|_| AllocError::with_kind_and_layout(AllocErrorKind::InvalidLayout, layout))?;

        // Additional validation
        if total_layout.size() > Self::max_allocation_size() {
            return Err(AllocError::with_kind_and_layout(AllocErrorKind::ExceedsMaxSize, layout));
        }

        unsafe { self.allocate(total_layout) }
    }

    /// Deallocates contiguous blocks allocated with `allocate_contiguous`
    ///
    /// This method deallocates memory that was previously allocated using
    /// `allocate_contiguous` with the same layout and count parameters.
    ///
    /// # Safety
    /// - `ptr` must have been allocated by this allocator using `allocate_contiguous`
    /// - `layout` and `count` must match the original allocation parameters exactly
    /// - After this call, `ptr` becomes invalid and must not be used
    unsafe fn deallocate_contiguous(&self, ptr: NonNull<u8>, layout: Layout, count: usize) {
        if count == 0 {
            return; // Nothing to deallocate for zero count
        }

        // Reconstruct the total layout used for allocation (consistent with allocate_contiguous)
        if let Some(total_size) = layout.size().checked_mul(count) {
            if let Ok(total_layout) = Layout::from_size_align(total_size, layout.align()) {
                unsafe { self.deallocate(ptr, total_layout) };
            }
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

        // Calculate old and new total sizes using consistent method
        let old_total_size = layout.size().checked_mul(old_count).ok_or_else(|| {
            AllocError::with_kind_and_layout(AllocErrorKind::SizeOverflow, layout)
        })?;

        let new_total_size = layout.size().checked_mul(new_count).ok_or_else(|| {
            AllocError::with_kind_and_layout(AllocErrorKind::SizeOverflow, layout)
        })?;

        let old_layout = Layout::from_size_align(old_total_size, layout.align())
            .map_err(|_| AllocError::with_kind_and_layout(AllocErrorKind::InvalidLayout, layout))?;

        let new_layout = Layout::from_size_align(new_total_size, layout.align())
            .map_err(|_| AllocError::with_kind_and_layout(AllocErrorKind::InvalidLayout, layout))?;

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
#[derive(Debug, Default, Clone, Copy, PartialEq)]
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

impl BasicMemoryUsage {
    /// Creates a new memory usage info
    pub const fn new(used: usize, available: Option<usize>) -> Self {
        let total = match available {
            Some(avail) => Some(used + avail),
            None => None,
        };

        // Can't compute percentage in const fn
        Self {
            used,
            available,
            total,
            usage_percent: None,
        }
    }

    /// Updates the usage percentage
    pub fn with_percentage(mut self) -> Self {
        self.usage_percent = self.total.and_then(|total| {
            if total == 0 {
                Some(0.0)
            } else {
                Some((self.used as f32 / total as f32) * 100.0)
            }
        });
        self
    }
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
    /// 4. Use memory barriers for thread safety
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
            // Memory barrier to ensure all previous operations complete
            memory_barrier_ex(BarrierType::Release);

            unsafe { self.reset() };

            // Memory barrier to ensure reset is visible to all threads
            memory_barrier_ex(BarrierType::Acquire);

            true
        } else {
            false
        }
    }
}

// ============================================================================
// Blanket implementations for smart pointers
// ============================================================================

/// Blanket implementation of Allocator for Box<T>
#[cfg(feature = "std")]
unsafe impl<T: Allocator + ?Sized> Allocator for Box<T> {
    #[inline]
    unsafe fn allocate(&self, layout: Layout) -> AllocResult<NonNull<[u8]>> {
        unsafe { (**self).allocate(layout) }
    }

    #[inline]
    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        unsafe { (**self).deallocate(ptr, layout) }
    }

    #[inline]
    unsafe fn reallocate(
        &self,
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> AllocResult<NonNull<[u8]>> {
        unsafe { (**self).reallocate(ptr, old_layout, new_layout) }
    }
}

/// Blanket implementation of Allocator for Arc<T>
#[cfg(feature = "std")]
unsafe impl<T: Allocator + ?Sized> Allocator for std::sync::Arc<T> {
    #[inline]
    unsafe fn allocate(&self, layout: Layout) -> AllocResult<NonNull<[u8]>> {
        unsafe { (**self).allocate(layout) }
    }

    #[inline]
    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        unsafe { (**self).deallocate(ptr, layout) }
    }

    #[inline]
    unsafe fn reallocate(
        &self,
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> AllocResult<NonNull<[u8]>> {
        unsafe { (**self).reallocate(ptr, old_layout, new_layout) }
    }
}

// ============================================================================
// Scoped reset support
// ============================================================================

/// RAII guard for automatic allocator reset
///
/// Useful for temporary allocations that should be automatically cleaned up.
/// The guard will reset the allocator when dropped, unless explicitly disabled.
///
/// # Example
/// ```ignore
/// use your_crate::{Allocator, Resettable, ResetGuard};
///
/// fn process_temporary_data(allocator: &impl Resettable) {
///     let _guard = ResetGuard::new(allocator);
///
///     // All allocations here will be freed when guard is dropped
///     unsafe {
///         let temp_buffer = allocator.allocate(Layout::new::<[u8; 1024]>())?;
///         // ... use temp_buffer ...
///     }
///     // Automatic reset happens here
/// }
/// ```
pub struct ResetGuard<'a, A: Resettable + ?Sized> {
    allocator: &'a A,
    should_reset: bool,
}

impl<'a, A: Resettable + ?Sized> ResetGuard<'a, A> {
    /// Creates a new reset guard that will reset the allocator on a drop
    ///
    /// # Example
    /// ```ignore
    /// let guard = ResetGuard::new(&allocator);
    /// // ... use allocator ...
    /// // Automatic reset when guard goes out of scope
    /// ```
    pub fn new(allocator: &'a A) -> Self {
        Self {
            allocator,
            should_reset: true,
        }
    }

    /// Disables automatic reset on a drop
    ///
    /// Useful when you want to keep allocations after the guard scope
    ///
    /// # Example
    /// ```ignore
    /// let mut guard = ResetGuard::new(&allocator);
    /// // ... allocate some memory ...
    /// if should_keep_allocations {
    ///     guard.disable_reset();
    /// }
    /// // Reset only happens if not disabled
    /// ```
    pub fn disable_reset(&mut self) {
        self.should_reset = false;
    }

    /// Gets a reference to the guarded allocator
    pub fn allocator(&self) -> &A {
        self.allocator
    }

    /// Checks if the guard will reset on drop
    pub fn will_reset(&self) -> bool {
        self.should_reset
    }
}

impl<'a, A: Resettable + ?Sized> Drop for ResetGuard<'a, A> {
    fn drop(&mut self) {
        if self.should_reset {
            // Ignore result - best effort reset on drop
            unsafe { self.allocator.try_reset(); }
        }
    }
}

impl<'a, A: Resettable + ?Sized> core::ops::Deref for ResetGuard<'a, A> {
    type Target = A;

    fn deref(&self) -> &Self::Target {
        self.allocator
    }
}

// ============================================================================
// Typed allocation extensions
// ============================================================================

/// Type-safe allocation extensions for better ergonomics and safety
///
/// Provides typed wrappers around raw allocation operations, eliminating
/// manual size calculations and improving type safety.
///
/// # Example
/// ```ignore
/// use your_crate::{Allocator, TypedAllocExt};
///
/// let allocator = MyAllocator::new();
///
/// unsafe {
///     // Allocate a single value
///     let ptr: NonNull<u32> = allocator.alloc_one()?;
///     ptr.as_ptr().write(42);
///
///     // Allocate an array
///     let array: NonNull<[u32]> = allocator.alloc_array(10)?;
///     let array_ptr = NonNull::new_unchecked(array.as_ptr() as *mut u32);
///     for i in 0..10 {
///         array_ptr.as_ptr().add(i).write(i as u32);
///     }
///
///     // Clean up
///     allocator.dealloc_one(ptr);
///     allocator.dealloc_array(array_ptr, 10);
/// }
/// ```
pub trait TypedAllocExt: Allocator {
    /// Allocates memory for a single value of type T
    ///
    /// # Safety
    /// - Returned pointer is uninitialized and must be written before reading
    /// - Caller must ensure proper deallocation with `dealloc_one`
    /// - Type T must be valid for the allocated memory region
    ///
    /// # Example
    /// ```ignore
    /// let allocator = MyAllocator::new();
    /// unsafe {
    ///     let ptr: NonNull<MyType> = allocator.alloc_one()?;
    ///     ptr.as_ptr().write(MyType::new());
    ///     // ... use the value ...
    ///     allocator.dealloc_one(ptr);
    /// }
    /// ```
    #[inline]
    #[track_caller]
    unsafe fn alloc_one<T>(&self) -> AllocResult<NonNull<T>> {
        let layout = Layout::new::<T>();
        let ptr = unsafe { self.allocate(layout)? };
        // Cast NonNull<[u8]> to NonNull<T>
        Ok(NonNull::new_unchecked(ptr.as_ptr() as *mut T))
    }

    /// Allocates memory for an array of n elements of type T
    ///
    /// # Safety
    /// - Returned memory is uninitialized
    /// - Caller must ensure proper deallocation with matching count using `dealloc_array`
    /// - All elements must be properly initialized before use
    ///
    /// # Example
    /// ```ignore
    /// let allocator = MyAllocator::new();
    /// unsafe {
    ///     let array: NonNull<[u32]> = allocator.alloc_array(100)?;
    ///     let ptr = NonNull::new_unchecked(array.as_ptr() as *mut u32);
    ///
    ///     // Initialize the array
    ///     for i in 0..100 {
    ///         ptr.as_ptr().add(i).write(i as u32);
    ///     }
    ///
    ///     // ... use the array ...
    ///     allocator.dealloc_array(ptr, 100);
    /// }
    /// ```
    #[inline]
    #[track_caller]
    unsafe fn alloc_array<T>(&self, n: usize) -> AllocResult<NonNull<[T]>> {
        if n == 0 {
            return Ok(NonNull::slice_from_raw_parts(NonNull::dangling(), 0));
        }

        // Use Layout::array for overflow checking
        let layout = Layout::array::<T>(n)
            .map_err(|_| {
                // Create a dummy layout for error reporting
                let dummy_layout = Layout::new::<T>();
                AllocError::with_kind_and_layout(AllocErrorKind::SizeOverflow, dummy_layout)
            })?;

        let ptr = unsafe { self.allocate(layout)? };
        // Cast NonNull<[u8]> to NonNull<[T]>
        Ok(NonNull::slice_from_raw_parts(
            NonNull::new_unchecked(ptr.as_ptr() as *mut T),
            n
        ))
    }

    /// Reallocates an array to a new size
    ///
    /// # Safety
    /// - `ptr` must have been allocated by this allocator with `alloc_array`
    /// - `old_n` must match the original allocation count exactly
    /// - Data beyond min(old_n, new_n) is uninitialized after reallocation
    /// - Existing data up to min(old_n, new_n) is preserved
    ///
    /// # Example
    /// ```ignore
    /// let allocator = MyAllocator::new();
    /// unsafe {
    ///     let array: NonNull<[u32]> = allocator.alloc_array(10)?;
    ///     let mut ptr = NonNull::new_unchecked(array.as_ptr() as *mut u32);
    ///     // ... initialize and use ...
    ///
    ///     // Grow the array
    ///     let new_array = allocator.realloc_array(ptr, 10, 20)?;
    ///     ptr = NonNull::new_unchecked(new_array.as_ptr() as *mut u32);
    ///     // Initialize new elements...
    ///
    ///     allocator.dealloc_array(ptr, 20);
    /// }
    /// ```
    #[inline]
    #[track_caller]
    unsafe fn realloc_array<T>(
        &self,
        ptr: NonNull<T>,
        old_n: usize,
        new_n: usize,
    ) -> AllocResult<NonNull<[T]>> {
        let old_layout = Layout::array::<T>(old_n)
            .map_err(|_| {
                let dummy_layout = Layout::new::<T>();
                AllocError::with_kind_and_layout(AllocErrorKind::InvalidLayout, dummy_layout)
            })?;
        let new_layout = Layout::array::<T>(new_n)
            .map_err(|_| {
                let dummy_layout = Layout::new::<T>();
                AllocError::with_kind_and_layout(AllocErrorKind::SizeOverflow, dummy_layout)
            })?;

        let new_ptr = unsafe { self.reallocate(ptr.cast(), old_layout, new_layout)? };
        // Cast NonNull<[u8]> to NonNull<[T]>
        Ok(NonNull::slice_from_raw_parts(
            NonNull::new_unchecked(new_ptr.as_ptr() as *mut T),
            new_n
        ))
    }

    /// Deallocates a single value
    ///
    /// # Safety
    /// - `ptr` must have been allocated by this allocator with `alloc_one`
    /// - The value must have been properly dropped before deallocation
    /// - Double-free is undefined behavior
    #[inline]
    #[track_caller]
    unsafe fn dealloc_one<T>(&self, ptr: NonNull<T>) {
        let layout = Layout::new::<T>();
        unsafe { self.deallocate(ptr.cast(), layout) };
    }

    /// Deallocates an array
    ///
    /// # Safety
    /// - `ptr` must have been allocated by this allocator with `alloc_array`
    /// - `n` must match the original allocation count exactly
    /// - All elements must have been properly dropped before deallocation
    /// - Double-free is undefined behavior
    ///
    /// # Example
    /// ```ignore
    /// let allocator = MyAllocator::new();
    /// unsafe {
    ///     let array: NonNull<[u32]> = allocator.alloc_array(100)?;
    ///     let ptr = NonNull::new_unchecked(array.as_ptr() as *mut u32);
    ///
    ///     // ... use the array ...
    ///
    ///     allocator.dealloc_array(ptr, 100);
    /// }
    /// ```
    #[inline]
    #[track_caller]
    unsafe fn dealloc_array<T>(&self, ptr: NonNull<T>, n: usize) {
        if n == 0 {
            return;
        }

        if let Ok(layout) = Layout::array::<T>(n) {
            unsafe { self.deallocate(ptr.cast(), layout) };
        }
    }
}

// Blanket implementation for all allocators
impl<A: Allocator + ?Sized> TypedAllocExt for A {}

// ============================================================================
// Additional helper traits
// ============================================================================

/// Statistics provider trait for comprehensive allocator metrics
///
/// This trait is imported from the stats module when available
#[cfg(feature = "stats")]
pub use super::stats::StatisticsProvider;

/// Default implementation placeholder when stats are disabled
#[cfg(not(feature = "stats"))]
pub trait StatisticsProvider {
    /// Statistics data type
    type Stats: Default + core::fmt::Debug;

    /// Get current statistics
    fn statistics(&self) -> Self::Stats {
        Default::default()
    }

    /// Reset all statistics counters
    fn reset_statistics(&self) {}

    /// Check if statistics collection is enabled
    fn statistics_enabled(&self) -> bool {
        false
    }

    /// Enable or disable statistics collection
    fn set_statistics_enabled(&self, _enabled: bool) {}
}

// Provide a simple default stats type when feature is disabled
#[cfg(not(feature = "stats"))]
#[derive(Debug, Default, Clone, Copy)]
pub struct AllocatorStats;

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