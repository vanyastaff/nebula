//! Async-aware lazy initialization wrapper.
//!
//! [`Lazy<X>`] is a typed wrapper around [`tokio::sync::OnceCell<X>`] that exposes
//! a future-friendly `get_or_init` flow. Two paths are supported:
//!
//! 1. **Pre-populated** via [`Lazy::with_value`] — used when the framework eagerly resolves a slot
//! before passing it to action code (the common case for `Lazy<ResourceGuard<R>>` fields the
//! macro emits).
//! 2. **Deferred** via [`Lazy::new`] + [`Lazy::get_or_try_init`] — used when an action wants to
//! resolve a dependency only on the path that needs it. The initializer is provided per
//! `.get_or_try_init` call so callers can plug in the right async resolver (typed
//! `ResourceRef::resolve` or `CredentialRef::resolve` ).
//!
//! See ) for the
//! place this primitive occupies in the dependency-redesign cascade.
//!
//! ## Cancel safety
//!
//! `Lazy::get_or_try_init` defers to `tokio::sync::OnceCell::get_or_try_init`,
//! which is cancel-safe: if the initializer future is cancelled, the cell remains
//! uninitialized and the next caller can retry. Failed initializers also leave
//! the cell uninitialized — distinct from `std::sync::OnceLock` semantics, which
//! permanently absorb a panic. `Lazy<X>` mirrors the tokio policy.

use std::{fmt, future::Future};

use tokio::sync::OnceCell;

/// Async-aware lazy wrapper.
///
/// `Lazy<X>` is the canonical wrapper for "resolve this dependency on first use,
/// not at action construction time". The macro emits this around
/// `ResourceGuard<R>` / `CredentialGuard<C::Scheme>` slot fields whose attribute
/// declares lazy resolution; eager slots are resolved by the framework before
/// `execute()` runs.
///
/// # Examples
///
/// ```ignore
/// use nebula_core::sync::Lazy;
///
/// let lazy: Lazy<String> = Lazy::new();
///
/// let value = lazy
///.get_or_try_init(async {
/// Ok::<_, std::io::Error>(format!("computed-{}", 42))
/// })
///.await?;
///
/// assert_eq!(value, "computed-42");
/// ```
pub struct Lazy<X> {
    cell: OnceCell<X>,
}

impl<X> Lazy<X> {
    /// Creates an empty `Lazy<X>`. The first `get_or_try_init` call populates it.
    #[must_use]
    pub fn new() -> Self {
        Self {
            cell: OnceCell::new(),
        }
    }

    /// Creates a `Lazy<X>` already populated with `value`. Subsequent `get_or_try_init`
    /// calls return the stored value without invoking the initializer.
    ///
    /// Used by macro-emitted factory bodies for slots that the framework
    /// resolves eagerly but that authors still hold via `Lazy<X>` — e.g. when
    /// composing `Option<Lazy<X>>` for optional + lazy semantics where the
    /// framework chooses to pre-populate.
    pub fn with_value(value: X) -> Self {
        let cell = OnceCell::new_with(Some(value));
        Self { cell }
    }

    /// Returns a reference to the value if already initialized.
    pub fn get(&self) -> Option<&X> {
        self.cell.get()
    }

    /// Returns true if the cell has been populated.
    pub fn is_initialized(&self) -> bool {
        self.cell.initialized()
    }

    /// Initializes the cell with the supplied async fallible initializer if
    /// not already initialized; returns a reference to the resolved value.
    ///
    /// Cancel-safe: a cancelled initializer leaves the cell uninitialized;
    /// failed initializers also leave it uninitialized so the next caller
    /// can retry. See module-level "Cancel safety" note.
    ///
    /// # Errors
    ///
    /// Propagates whatever error type the initializer's `Result` carries.
    pub async fn get_or_try_init<F, E>(&self, init: F) -> Result<&X, E>
    where
        F: Future<Output = Result<X, E>>,
    {
        self.cell.get_or_try_init(|| init).await
    }
}

impl<X> Default for Lazy<X> {
    fn default() -> Self {
        Self::new()
    }
}

impl<X: fmt::Debug> fmt::Debug for Lazy<X> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.cell.get() {
            Some(v) => f.debug_tuple("Lazy").field(v).finish(),
            None => f.debug_tuple("Lazy").field(&"<uninit>").finish(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn new_starts_uninitialized() {
        let l: Lazy<u32> = Lazy::new();
        assert!(!l.is_initialized());
        assert!(l.get().is_none());
    }

    #[tokio::test]
    async fn with_value_is_initialized() {
        let l = Lazy::with_value(42_u32);
        assert!(l.is_initialized());
        assert_eq!(l.get(), Some(&42));
    }

    #[tokio::test]
    async fn get_or_try_init_runs_initializer_once() {
        let l: Lazy<String> = Lazy::new();
        let v = l
            .get_or_try_init(async { Ok::<_, &'static str>("hello".to_owned()) })
            .await
            .unwrap();
        assert_eq!(v, "hello");

        // Second call should return the cached value, NOT the new one
        let v2 = l
            .get_or_try_init(async { Ok::<_, &'static str>("ignored".to_owned()) })
            .await
            .unwrap();
        assert_eq!(v2, "hello");
    }

    #[tokio::test]
    async fn failed_initializer_leaves_cell_uninitialized() {
        let l: Lazy<u32> = Lazy::new();
        let err = l
            .get_or_try_init(async { Err::<u32, &'static str>("boom") })
            .await
            .unwrap_err();
        assert_eq!(err, "boom");
        assert!(!l.is_initialized());

        // Retry should succeed
        let v = l
            .get_or_try_init(async { Ok::<_, &'static str>(7) })
            .await
            .unwrap();
        assert_eq!(*v, 7);
    }

    #[test]
    fn debug_renders_uninit_state() {
        let l: Lazy<u32> = Lazy::new();
        let s = format!("{l:?}");
        assert!(s.contains("uninit"));
    }

    #[tokio::test]
    async fn debug_renders_init_state() {
        let l = Lazy::with_value(99_u32);
        let s = format!("{l:?}");
        assert!(s.contains("99"));
    }
}
