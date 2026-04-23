//! Credential guard — secure wrapper for credential access.
//!
//! [`CredentialGuard`] wraps an [`AuthScheme`](crate::AuthScheme) value with:
//! 1. **Transparent access** via `Deref<Target = S>`
//! 2. **Zeroize on drop** — secret material wiped from memory
//! 3. **Not Serialize** — prevents accidental inclusion in action output or state

use std::{fmt, ops::Deref, time::Instant};

use nebula_core::{Guard, TypedGuard};
use zeroize::Zeroize;

/// Secure wrapper for credential values returned by action contexts.
///
/// # Guarantees
///
/// - `Deref<Target = S>` — transparent access to the inner credential
/// - `Drop` calls `zeroize()` — secret material wiped from memory
/// - Does NOT implement `Serialize` — compile error if placed in output/state types
///
/// # Errors
///
/// `CredentialGuard` itself is infallible once constructed. It is typically
/// obtained via `ActionContext::credential::<S>()`, which returns an error
/// when the credential cannot be resolved or deserialized.
///
/// # Examples
///
/// ```rust,ignore
/// let cred = ctx.credential::<BearerSecret>().await?;
/// client.bearer_auth(cred.token.expose_secret());
/// // Dropped here — zeroized automatically
/// ```
#[must_use = "credential guards must be held for the duration of use"]
pub struct CredentialGuard<S: Zeroize> {
    inner: S,
    acquired_at: Instant,
}

impl<S: Zeroize> CredentialGuard<S> {
    /// Wrap a credential value in a guard.
    pub fn new(inner: S) -> Self {
        Self {
            inner,
            acquired_at: Instant::now(),
        }
    }
}

impl<S: Zeroize> Deref for CredentialGuard<S> {
    type Target = S;

    fn deref(&self) -> &S {
        &self.inner
    }
}

impl<S: Zeroize> Drop for CredentialGuard<S> {
    fn drop(&mut self) {
        self.inner.zeroize();
    }
}

impl<S: Zeroize + Clone> Clone for CredentialGuard<S> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            acquired_at: Instant::now(),
        }
    }
}

impl<S: Zeroize + Send + Sync + 'static> Guard for CredentialGuard<S> {
    fn guard_kind(&self) -> &'static str {
        "credential"
    }

    fn acquired_at(&self) -> Instant {
        self.acquired_at
    }
}

impl<S: Zeroize + Send + Sync + 'static> TypedGuard for CredentialGuard<S> {
    type Inner = S;

    fn as_inner(&self) -> &Self::Inner {
        &self.inner
    }
}

impl<S: Zeroize + Send + Sync + 'static> fmt::Debug for CredentialGuard<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        nebula_core::guard::debug_redacted(self, f)
    }
}

// NOTE: Intentionally NO Serialize/Deserialize impl — compile error if placed in output.

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq)]
    struct TestSecret {
        value: String,
    }

    impl Zeroize for TestSecret {
        fn zeroize(&mut self) {
            self.value.zeroize();
        }
    }

    #[test]
    fn deref_exposes_inner_value() {
        let guard = CredentialGuard::new(TestSecret {
            value: "secret-123".to_owned(),
        });
        assert_eq!(guard.value, "secret-123");
    }

    #[test]
    fn clone_preserves_value() {
        let guard = CredentialGuard::new(TestSecret {
            value: "secret-123".to_owned(),
        });
        let cloned = guard.clone();
        assert_eq!(*cloned, *guard);
    }

    #[test]
    fn debug_redacts_inner_value() {
        let guard = CredentialGuard::new(TestSecret {
            value: "secret-123".to_owned(),
        });
        let debug = format!("{guard:?}");
        assert!(debug.contains("Guard<credential>"));
        assert!(debug.contains("REDACTED"));
        assert!(!debug.contains("secret-123"));
    }

    #[test]
    fn drop_zeroizes_inner() {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };

        /// A secret type whose `Zeroize` impl sets a shared flag.
        struct ObservableSecret {
            zeroized: Arc<AtomicBool>,
        }

        impl Zeroize for ObservableSecret {
            fn zeroize(&mut self) {
                self.zeroized.store(true, Ordering::Release);
            }
        }

        let flag = Arc::new(AtomicBool::new(false));
        let guard = CredentialGuard::new(ObservableSecret {
            zeroized: Arc::clone(&flag),
        });

        assert!(!flag.load(Ordering::Acquire), "should not be zeroized yet");
        drop(guard);
        assert!(flag.load(Ordering::Acquire), "Drop must call zeroize()");
    }
}
