//! Lock-free cell for the Resident topology.
//!
//! [`Cell`] wraps an [`ArcSwapOption`] to provide a simple, lock-free
//! store/load/take API for a single shared value.

use std::sync::Arc;

use arc_swap::ArcSwapOption;

/// A lock-free cell holding an optional `Arc<T>`.
///
/// Designed for the Resident topology where a single shared runtime
/// value is swapped atomically during hot-reload or shutdown.
#[derive(Debug)]
pub struct Cell<T> {
    inner: ArcSwapOption<T>,
}

impl<T> Cell<T> {
    /// Creates an empty cell.
    pub fn new() -> Self {
        Self {
            inner: ArcSwapOption::empty(),
        }
    }

    /// Stores a new value, replacing the previous one.
    pub fn store(&self, value: Arc<T>) {
        self.inner.store(Some(value));
    }

    /// Loads the current value, if any.
    pub fn load(&self) -> Option<Arc<T>> {
        self.inner.load_full()
    }

    /// Takes the value out, leaving the cell empty.
    pub fn take(&self) -> Option<Arc<T>> {
        self.inner.swap(None)
    }

    /// Returns `true` if the cell contains a value.
    pub fn is_some(&self) -> bool {
        self.inner.load().is_some()
    }
}

impl<T> Default for Cell<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_cell_is_empty() {
        let cell = Cell::<u32>::new();
        assert!(cell.load().is_none());
        assert!(!cell.is_some());
    }

    #[test]
    fn store_and_load() {
        let cell = Cell::new();
        cell.store(Arc::new(42));
        assert!(cell.is_some());
        let val = cell.load().unwrap();
        assert_eq!(*val, 42);
    }

    #[test]
    fn store_replaces_previous() {
        let cell = Cell::new();
        cell.store(Arc::new(1));
        cell.store(Arc::new(2));
        assert_eq!(*cell.load().unwrap(), 2);
    }

    #[test]
    fn take_removes_value() {
        let cell = Cell::new();
        cell.store(Arc::new(99));
        let taken = cell.take();
        assert_eq!(*taken.unwrap(), 99);
        assert!(!cell.is_some());
        assert!(cell.load().is_none());
    }

    #[test]
    fn take_on_empty_returns_none() {
        let cell = Cell::<String>::new();
        assert!(cell.take().is_none());
    }
}
