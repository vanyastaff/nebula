//! Thread-safe interior mutability primitives for allocators.
//!
//! This module provides `SyncUnsafeCell`, a wrapper around `UnsafeCell` that
//! implements `Sync` to allow safe sharing across threads when the user
//! guarantees external synchronization.

use core::cell::UnsafeCell;

/// A wrapper around `UnsafeCell<T>` that implements `Sync`.
///
/// # Safety
///
/// The caller must ensure that access to the inner value is properly synchronized.
/// This type is `Sync` because allocators guarantee thread-safety through their
/// own synchronization mechanisms (atomic operations, locks, etc.).
///
/// # Memory Layout
///
/// This type is `repr(transparent)` and has the same layout as `UnsafeCell<T>`.
#[repr(transparent)]
pub(crate) struct SyncUnsafeCell<T: ?Sized>(UnsafeCell<T>);

// SAFETY: SyncUnsafeCell<T> is Sync if T is Send.
// - The UnsafeCell wrapper doesn't add thread-local state
// - External synchronization is guaranteed by allocator implementations
// - T: Send ensures the value can be transferred between threads
unsafe impl<T: ?Sized + Send> Sync for SyncUnsafeCell<T> {}

// SAFETY: SyncUnsafeCell<T> is Send if T is Send.
// - Wrapper is repr(transparent), same layout as UnsafeCell<T>
// - T: Send bound ensures inner value can move between threads
// - No thread-local state in wrapper
unsafe impl<T: ?Sized + Send> Send for SyncUnsafeCell<T> {}

impl<T> SyncUnsafeCell<T> {
    /// Creates a new `SyncUnsafeCell` containing the given value.
    #[inline]
    pub(crate) const fn new(value: T) -> Self {
        Self(UnsafeCell::new(value))
    }
}

impl<T: ?Sized> SyncUnsafeCell<T> {
    /// Gets a mutable pointer to the wrapped value.
    ///
    /// # Safety
    ///
    /// The caller must ensure that access to the returned pointer is properly
    /// synchronized and doesn't violate Rust's aliasing rules.
    #[inline]
    pub(crate) fn get(&self) -> *mut T {
        self.0.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sync_unsafe_cell() {
        let cell = SyncUnsafeCell::new(42_i32);
        unsafe {
            assert_eq!(*cell.get(), 42);
            *cell.get() = 100;
            assert_eq!(*cell.get(), 100);
        }
    }

    #[test]
    fn test_send_sync_bounds() {
        fn assert_send<T: Send>() {}
        fn assert_sync<T: Sync>() {}

        assert_send::<SyncUnsafeCell<i32>>();
        assert_sync::<SyncUnsafeCell<i32>>();
    }
}
