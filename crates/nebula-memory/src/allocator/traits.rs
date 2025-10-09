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
//!
//! Common traits are re-exported from `core::traits`:
//! - `MemoryUsage`: Memory tracking capabilities (from core)
//! - `Resettable`: Allocator reset functionality (from core)

use core::alloc::Layout;
use core::ptr::NonNull;

use super::{AllocError, AllocErrorCode, AllocResult};

// Re-export core traits for convenience
pub use crate::core::traits::{BasicMemoryUsage, MemoryUsage, Resettable};

/// Validation of layout parameters
///
/// Performs comprehensive validation of allocation layout to catch
/// common errors early and provide detailed diagnostics.
#[inline]
fn validate_layout(layout: Layout) -> AllocResult<()> {
    // Check that alignment is a power of two
    if !layout.align().is_power_of_two() {
        return Err(AllocError::with_layout(
            AllocErrorCode::InvalidAlignment,
            layout,
        ));
    }

    // Check for zero-sized allocations (they're valid but need special handling)
    if layout.size() == 0 {
        return Ok(());
    }

    // Check for potential overflow when adding padding
    if layout.size() > isize::MAX as usize - (layout.align() - 1) {
        return Err(AllocError::with_layout(
            AllocErrorCode::SizeOverflow,
            layout,
        ));
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
            }
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
            core::ptr::copy_nonoverlapping(
                ptr.as_ptr() as *const u8,
                new_ptr.as_ptr() as *mut u8,
                old_layout.size(),
            );
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
        // Layout::max_size() is unstable, use safe maximum
        isize::MAX as usize
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
            let dangling = NonNull::<u8>::dangling();
            return Ok(NonNull::slice_from_raw_parts(dangling, 0));
        }

        // Check for overflow when multiplying size by count
        let total_size = layout
            .size()
            .checked_mul(count)
            .ok_or_else(|| AllocError::with_layout(AllocErrorCode::SizeOverflow, layout))?;

        // Check against maximum allocation size
        if total_size > Self::max_allocation_size() {
            return Err(AllocError::with_layout(
                AllocErrorCode::ExceedsMaxSize,
                layout,
            ));
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
        let old_total_size = layout
            .size()
            .checked_mul(old_count)
            .ok_or_else(|| AllocError::with_layout(AllocErrorCode::SizeOverflow, layout))?;

        let new_total_size = layout
            .size()
            .checked_mul(new_count)
            .ok_or_else(|| AllocError::with_layout(AllocErrorCode::SizeOverflow, layout))?;

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

/// Type-safe allocator trait providing compile-time size/alignment guarantees
///
/// This trait extends [`Allocator`] with type-safe methods that eliminate common
/// allocation mistakes by leveraging Rust's type system. All size and alignment
/// calculations are performed at compile time where possible.
///
/// # Benefits
/// - **Compile-time correctness**: Layout is derived from type `T`, preventing errors
/// - **Zero overhead**: All methods inline and delegate to base [`Allocator`]
/// - **Type safety**: Cannot accidentally pass wrong layout for a type
/// - **Ergonomics**: Cleaner API for typed allocations
///
/// # Examples
/// ```rust
/// use nebula_memory::prelude::*;
/// use nebula_memory::allocator::TypedAllocator;
///
/// let allocator = BumpAllocator::new(4096)?;
///
/// // Type-safe allocation
/// let ptr = unsafe { allocator.alloc_typed::<u64>()? };
///
/// // Initialize with value
/// let ptr = unsafe { allocator.alloc_init(42u64)? };
///
/// // Array allocation
/// let array = unsafe { allocator.alloc_array::<u32>(10)? };
///
/// # Ok::<(), nebula_memory::AllocError>(())
/// ```
///
/// # Safety
/// Same safety requirements as [`Allocator`] apply. The caller must:
/// - Not use pointers after deallocation
/// - Pass correct pointer and type to deallocation methods
/// - Ensure proper alignment and initialization
pub trait TypedAllocator: Allocator {
    /// Allocates memory for a single instance of type `T`
    ///
    /// Returns a properly aligned pointer suitable for storing a `T`.
    /// The memory is **not initialized**.
    ///
    /// # Safety
    /// The caller must initialize the memory before reading from it,
    /// and must deallocate it with `dealloc_typed::<T>()` when done.
    ///
    /// # Examples
    /// ```rust
    /// use nebula_memory::prelude::*;
    /// use nebula_memory::allocator::TypedAllocator;
    ///
    /// let allocator = BumpAllocator::new(1024)?;
    /// let ptr = unsafe { allocator.alloc_typed::<u64>()? };
    /// unsafe {
    ///     ptr.as_ptr().write(42);
    ///     assert_eq!(*ptr.as_ptr(), 42);
    /// }
    /// # Ok::<(), nebula_memory::AllocError>(())
    /// ```
    #[inline]
    unsafe fn alloc_typed<T>(&self) -> AllocResult<NonNull<T>> {
        let layout = Layout::new::<T>();
        let ptr = unsafe { self.allocate(layout)? };
        Ok(unsafe { NonNull::new_unchecked(ptr.as_ptr() as *mut T) })
    }

    /// Allocates and initializes memory for a single instance of type `T`
    ///
    /// This is a convenience method that allocates memory and writes the
    /// provided value into it.
    ///
    /// # Safety
    /// The caller must deallocate with `dealloc_typed::<T>()` when done.
    /// The value is moved into the allocation, so `T` need not be `Copy`.
    ///
    /// # Examples
    /// ```rust
    /// use nebula_memory::prelude::*;
    /// use nebula_memory::allocator::TypedAllocator;
    ///
    /// let allocator = BumpAllocator::new(1024)?;
    /// let ptr = unsafe { allocator.alloc_init(String::from("hello"))? };
    /// unsafe {
    ///     assert_eq!(&*ptr.as_ptr(), "hello");
    /// }
    /// # Ok::<(), nebula_memory::AllocError>(())
    /// ```
    #[inline]
    unsafe fn alloc_init<T>(&self, value: T) -> AllocResult<NonNull<T>> {
        let ptr = unsafe { self.alloc_typed::<T>()? };
        unsafe {
            ptr.as_ptr().write(value);
        }
        Ok(ptr)
    }

    /// Allocates memory for an array of `count` instances of type `T`
    ///
    /// Returns a pointer to the first element. The memory is **not initialized**.
    ///
    /// # Safety
    /// - Caller must initialize all elements before reading
    /// - Must deallocate with `dealloc_array::<T>()` passing same count
    /// - Count must not cause size overflow
    ///
    /// # Examples
    /// ```rust
    /// use nebula_memory::prelude::*;
    /// use nebula_memory::allocator::TypedAllocator;
    ///
    /// let allocator = BumpAllocator::new(1024)?;
    /// let ptr = unsafe { allocator.alloc_array::<u32>(10)? };
    /// unsafe {
    ///     for i in 0..10 {
    ///         ptr.as_ptr().add(i).write(i as u32);
    ///     }
    /// }
    /// # Ok::<(), nebula_memory::AllocError>(())
    /// ```
    #[inline]
    unsafe fn alloc_array<T>(&self, count: usize) -> AllocResult<NonNull<T>> {
        if count == 0 {
            // For zero-sized allocation, return a dangling but properly aligned pointer
            return Ok(NonNull::dangling());
        }

        let layout =
            Layout::array::<T>(count).map_err(|_| AllocError::new(AllocErrorCode::SizeOverflow))?;

        let ptr = unsafe { self.allocate(layout)? };
        Ok(unsafe { NonNull::new_unchecked(ptr.as_ptr() as *mut T) })
    }

    /// Allocates and initializes an array by cloning a value
    ///
    /// Creates an array of `count` elements, each initialized by cloning `value`.
    ///
    /// # Safety
    /// Must deallocate with `dealloc_array::<T>()` passing same count.
    ///
    /// # Examples
    /// ```rust
    /// use nebula_memory::prelude::*;
    /// use nebula_memory::allocator::TypedAllocator;
    ///
    /// let allocator = BumpAllocator::new(1024)?;
    /// let ptr = unsafe { allocator.alloc_array_with::<u32>(5, 42)? };
    /// unsafe {
    ///     for i in 0..5 {
    ///         assert_eq!(*ptr.as_ptr().add(i), 42);
    ///     }
    /// }
    /// # Ok::<(), nebula_memory::AllocError>(())
    /// ```
    #[inline]
    unsafe fn alloc_array_with<T: Clone>(&self, count: usize, value: T) -> AllocResult<NonNull<T>> {
        let ptr = unsafe { self.alloc_array::<T>(count)? };

        if count > 0 {
            unsafe {
                for i in 0..count {
                    ptr.as_ptr().add(i).write(value.clone());
                }
            }
        }

        Ok(ptr)
    }

    /// Deallocates memory for a single instance of type `T`
    ///
    /// # Safety
    /// - `ptr` must have been allocated by `alloc_typed::<T>()` or `alloc_init::<T>()`
    /// - `ptr` must not be used after this call
    /// - If `T` has a destructor, caller must run it before deallocation
    #[inline]
    unsafe fn dealloc_typed<T>(&self, ptr: NonNull<T>) {
        let layout = Layout::new::<T>();
        unsafe { self.deallocate(ptr.cast(), layout) }
    }

    /// Deallocates memory for an array of type `T`
    ///
    /// # Safety
    /// - `ptr` must have been allocated by `alloc_array::<T>()` with same `count`
    /// - `ptr` must not be used after this call
    /// - If `T` has a destructor, caller must run it for all elements before deallocation
    #[inline]
    unsafe fn dealloc_array<T>(&self, ptr: NonNull<T>, count: usize) {
        if count == 0 {
            return;
        }

        let layout = Layout::array::<T>(count).expect("layout must be valid for deallocation");
        unsafe { self.deallocate(ptr.cast(), layout) }
    }
}

/// Blanket implementation: all Allocators automatically implement TypedAllocator
///
/// This means any type implementing [`Allocator`] gets the type-safe API for free.
impl<A: Allocator + ?Sized> TypedAllocator for A {}

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
