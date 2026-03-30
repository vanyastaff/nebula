//! Cursor implementations for bump allocator
//!
//! Provides atomic cursor for thread-safe bump pointer tracking.

use core::sync::atomic::{AtomicUsize, Ordering};

/// Trait for cursor abstraction
pub(super) trait Cursor: Send + Sync {
    fn load(&self, ordering: Ordering) -> usize;
    fn store(&self, val: usize, ordering: Ordering);
    fn compare_exchange_weak(
        &self,
        current: usize,
        new: usize,
        success: Ordering,
        failure: Ordering,
    ) -> Result<usize, usize>;
}

/// Atomic cursor for thread-safe access
///
/// On x86, `Relaxed` atomic operations compile to plain load/store instructions,
/// so there is zero overhead compared to a non-atomic Cell-based approach.
pub(super) struct AtomicCursor(AtomicUsize);

impl AtomicCursor {
    pub fn new(val: usize) -> Self {
        Self(AtomicUsize::new(val))
    }
}

impl Cursor for AtomicCursor {
    #[inline]
    fn load(&self, ordering: Ordering) -> usize {
        self.0.load(ordering)
    }

    #[inline]
    fn store(&self, val: usize, ordering: Ordering) {
        self.0.store(val, ordering);
    }

    #[inline]
    fn compare_exchange_weak(
        &self,
        current: usize,
        new: usize,
        success: Ordering,
        failure: Ordering,
    ) -> Result<usize, usize> {
        self.0.compare_exchange_weak(current, new, success, failure)
    }
}
