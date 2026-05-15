//! Per-slot runtime storage for a resolved credential.
//!
//! A resource declares `#[credential]` slots; the engine resolves each into a
//! `CredentialGuard<C>` and stores it here before `Resource::create`. On
//! rotation the engine swaps a fresh guard in without `&mut` on the resource
//! (D2 of the finalization spec). Lock-free via `arc-swap`.

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
}
