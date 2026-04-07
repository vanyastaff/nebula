//! Cursor for bump allocator
//!
//! Atomic cursor for thread-safe bump pointer tracking.
//! On x86, `Relaxed` atomic ops compile to plain load/store (zero overhead).

use core::sync::atomic::{AtomicUsize, Ordering};

/// Atomic cursor for thread-safe bump pointer tracking.
///
/// Used directly as a concrete field — no trait object overhead.
/// On x86, `Relaxed` atomics compile to plain `mov`, so there is no performance
/// difference vs a raw `usize` in single-threaded use.
pub(super) struct AtomicCursor(AtomicUsize);

impl AtomicCursor {
    pub fn new(val: usize) -> Self {
        Self(AtomicUsize::new(val))
    }

    #[inline]
    pub fn load(&self, ordering: Ordering) -> usize {
        self.0.load(ordering)
    }

    #[inline]
    pub fn store(&self, val: usize, ordering: Ordering) {
        self.0.store(val, ordering);
    }

    #[inline]
    pub fn compare_exchange_weak(
        &self,
        current: usize,
        new: usize,
        success: Ordering,
        failure: Ordering,
    ) -> Result<usize, usize> {
        self.0.compare_exchange_weak(current, new, success, failure)
    }
}
