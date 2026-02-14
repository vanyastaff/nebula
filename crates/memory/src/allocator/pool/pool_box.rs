//! Smart pointer for pool-allocated objects

use core::alloc::Layout;
use core::ptr::{self, NonNull};

use super::PoolAllocator;
use crate::allocator::{AllocError, Allocator};

/// RAII smart pointer for pool-allocated values
///
/// Automatically returns memory to the pool when dropped.
/// Similar to `Box` but backed by a pool allocator.
pub struct PoolBox<T> {
    ptr: NonNull<T>,
    allocator: NonNull<PoolAllocator>,
}

impl<T> PoolBox<T> {
    /// Creates a new `PoolBox` by allocating from the given pool
    #[must_use = "allocated value must be used"]
    pub fn new_in(value: T, allocator: &PoolAllocator) -> Result<Self, AllocError> {
        let layout = Layout::new::<T>();

        // SAFETY: Pool allocation and initialization sequence.
        // 1. allocate() returns valid, aligned memory or error
        // 2. cast::<T>() preserves pointer validity and alignment
        // 3. write(value) initializes memory (moves value, doesn't drop)
        // 4. NonNull::new() validates non-null (defensive check)
        // 5. allocator reference converted to NonNull (always valid for &T)
        // 6. No aliasing: ptr is exclusive until Drop runs
        unsafe {
            let ptr = allocator.allocate(layout)?;
            let typed_ptr = ptr.as_ptr().cast::<T>();
            typed_ptr.write(value);

            // typed_ptr is non-null (from successful allocation), but use explicit check
            let ptr_non_null = NonNull::new(typed_ptr)
                .ok_or_else(|| AllocError::allocation_failed(layout.size(), layout.align()))?;

            // Convert reference to NonNull (references are always non-null)
            let allocator_non_null = NonNull::from(allocator);

            Ok(Self {
                ptr: ptr_non_null,
                allocator: allocator_non_null.cast(),
            })
        }
    }

    /// Gets a reference to the contained value
    #[must_use]
    // Custom implementation needed for pool semantics, not using std AsRef trait
    #[expect(clippy::should_implement_trait)]
    pub fn as_ref(&self) -> &T {
        // SAFETY: Dereferencing self.ptr as shared reference.
        // - self.ptr is NonNull, guaranteed non-null
        // - Points to initialized T (from new_in)
        // - PoolBox owns the allocation exclusively
        // - Lifetime tied to &self, prevents use-after-free
        unsafe { self.ptr.as_ref() }
    }

    /// Gets a mutable reference to the contained value
    // Custom implementation needed for pool semantics, not using std AsMut trait
    #[expect(clippy::should_implement_trait)]
    pub fn as_mut(&mut self) -> &mut T {
        // SAFETY: Dereferencing self.ptr as mutable reference.
        // - self.ptr is NonNull, guaranteed non-null
        // - Points to initialized T (from new_in)
        // - &mut self ensures exclusive access (no aliasing)
        // - Lifetime tied to &mut self, prevents use-after-free
        unsafe { self.ptr.as_mut() }
    }

    /// Consumes the `PoolBox` and returns the contained value
    #[must_use]
    pub fn into_inner(self) -> T {
        // SAFETY: Reading value from owned allocation.
        // - self.ptr points to initialized T
        // - ptr::read performs bitwise copy (doesn't drop)
        // - Ownership of T transferred to caller
        // - No double-drop: mem::forget(self) prevents Drop::drop
        let value = unsafe { ptr::read(self.ptr.as_ptr()) };

        // SAFETY: Deallocating memory without dropping T.
        // - self.allocator points to valid PoolAllocator (from new_in)
        // - self.ptr matches original allocation
        // - Layout matches allocation layout
        // - T already moved out (ptr::read above), no drop needed
        // - mem::forget below prevents Drop from running
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
        // SAFETY: Dropping value and returning memory to pool.
        // 1. drop_in_place runs T's destructor:
        //    - self.ptr points to initialized T
        //    - Exclusive access via &mut self (no aliasing)
        // 2. deallocate returns memory to pool:
        //    - self.allocator is valid PoolAllocator reference
        //    - self.ptr matches original allocation from new_in
        //    - Layout matches original allocation
        //    - T already dropped, safe to reclaim memory
        unsafe {
            // Run the destructor
            ptr::drop_in_place(self.ptr.as_ptr());

            // Deallocate the memory
            let layout = Layout::new::<T>();
            self.allocator.as_ref().deallocate(self.ptr.cast(), layout);
        }
    }
}
