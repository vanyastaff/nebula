//! Per-slot runtime storage for a resolved credential.
//!
//! A resource declares `#[credential]` slots; the engine resolves each into a
//! `CredentialGuard<C>` and stores it here before `Resource::create`. On
//! rotation the engine swaps a fresh guard in without `&mut` on the
//! resource (the `&self` refresh-hook model, ADR-0067). Lock-free via
//! `arc-swap`.

use arc_swap::ArcSwapOption;
use std::sync::Arc;

/// Lock-free interior-mutable holder for one resolved credential slot.
///
/// Holds `Arc<S>`: a real slot value is `CredentialGuard<C>`, which is
/// `!Clone` and zeroizes on `Drop`, so the `Arc` indirection lets the engine
/// swap a rotated guard in with no secret-byte clone.
#[derive(Debug)]
pub struct SlotCell<S> {
    inner: ArcSwapOption<S>,
}

impl<S> SlotCell<S> {
    /// An unresolved slot.
    pub fn empty() -> Self {
        Self {
            inner: ArcSwapOption::empty(),
        }
    }

    /// Install (or replace) the resolved value.
    pub fn store(&self, value: Arc<S>) {
        self.inner.store(Some(value));
    }

    /// Snapshot the current value, if resolved.
    pub fn load(&self) -> Option<Arc<S>> {
        self.inner.load_full()
    }

    /// Revoke the slot, returning the previously held value (if any).
    pub fn take(&self) -> Option<Arc<S>> {
        self.inner.swap(None)
    }

    /// Returns `true` if the slot currently holds a resolved value.
    pub fn is_some(&self) -> bool {
        self.inner.load().is_some()
    }
}

impl<S> Default for SlotCell<S> {
    fn default() -> Self {
        Self::empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[derive(Default)]
    struct FakeGuard(u32);
    impl zeroize::Zeroize for FakeGuard {
        fn zeroize(&mut self) {
            self.0 = 0;
        }
    }

    #[test]
    fn slot_cell_swaps_without_clone_and_reads_latest() {
        let cell: SlotCell<FakeGuard> = SlotCell::empty();
        assert!(cell.load().is_none());
        cell.store(Arc::new(FakeGuard(1)));
        assert_eq!(cell.load().expect("v1").0, 1);
        cell.store(Arc::new(FakeGuard(2)));
        assert_eq!(cell.load().expect("v2").0, 2);
    }

    #[test]
    fn take_and_is_some() {
        let cell: SlotCell<FakeGuard> = SlotCell::empty();

        // Empty cell: is_some is false, take returns None.
        assert!(!cell.is_some());
        assert!(cell.take().is_none());

        // After store: is_some is true, take returns the value.
        cell.store(Arc::new(FakeGuard(1)));
        assert!(cell.is_some());
        let taken = cell.take();
        assert_eq!(taken.expect("should be Some").0, 1);

        // After take: cell is empty again.
        assert!(cell.load().is_none());
        assert!(!cell.is_some());

        // Second take on now-empty cell returns None.
        assert!(cell.take().is_none());
    }
}
