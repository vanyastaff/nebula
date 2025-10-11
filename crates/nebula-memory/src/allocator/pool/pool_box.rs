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
    /// Creates a new PoolBox by allocating from the given pool
    #[must_use = "allocated value must be used"]
    pub fn new_in(value: T, allocator: &PoolAllocator) -> Result<Self, AllocError> {
        let layout = Layout::new::<T>();

        unsafe {
            let ptr = allocator.allocate(layout)?;
            let typed_ptr = ptr.as_ptr() as *mut T;
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
    pub fn as_ref(&self) -> &T {
        unsafe { self.ptr.as_ref() }
    }

    /// Gets a mutable reference to the contained value
    pub fn as_mut(&mut self) -> &mut T {
        unsafe { self.ptr.as_mut() }
    }

    /// Consumes the PoolBox and returns the contained value
    pub fn into_inner(self) -> T {
        let value = unsafe { ptr::read(self.ptr.as_ptr()) };

        // Deallocate without running the destructor
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
        unsafe {
            // Run the destructor
            ptr::drop_in_place(self.ptr.as_ptr());

            // Deallocate the memory
            let layout = Layout::new::<T>();
            self.allocator.as_ref().deallocate(self.ptr.cast(), layout);
        }
    }
}
