//! Cursor implementations for bump allocator
//!
//! Provides both atomic (thread-safe) and cell-based (single-thread) cursors.

use core::cell::Cell;
use core::sync::atomic::{AtomicUsize, Ordering};

/// Trait for cursor abstraction (atomic or cell-based)
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

/// Atomic cursor for multi-threaded access
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

/// Cell-based cursor for single-threaded access (faster, no atomic overhead)
pub(super) struct CellCursor(Cell<usize>);

impl CellCursor {
    pub fn new(val: usize) -> Self {
        Self(Cell::new(val))
    }
}

impl Cursor for CellCursor {
    #[inline]
    fn load(&self, _ordering: Ordering) -> usize {
        self.0.get()
    }

    #[inline]
    fn store(&self, val: usize, _ordering: Ordering) {
        self.0.set(val);
    }

    #[inline]
    fn compare_exchange_weak(
        &self,
        current: usize,
        new: usize,
        _success: Ordering,
        _failure: Ordering,
    ) -> Result<usize, usize> {
        let actual = self.0.get();
        if actual == current {
            self.0.set(new);
            Ok(actual)
        } else {
            Err(actual)
        }
    }
}

// SAFETY: CellCursor is only used in single-threaded mode
unsafe impl Send for CellCursor {}
unsafe impl Sync for CellCursor {}
