//! Typed credential handle returned by the resolver.
//!
//! Wraps a resolved AuthScheme in an `ArcSwap` so the refresh coordinator can
//! hot-swap refreshed auth material without creating a new handle.

use std::sync::Arc;

use arc_swap::ArcSwap;

use crate::AuthScheme;

struct Inner<S: AuthScheme> {
    scheme: ArcSwap<S>,
    credential_id: String,
}

/// Handle to a resolved credential with a specific AuthScheme.
///
/// Uses `ArcSwap` internally so [`CredentialResolver`](crate::runtime::CredentialResolver)
/// can hot-swap refreshed auth material via [`replace`](Self::replace).
/// [`Clone`](Self::clone) shares the same live scheme cell — a refresh updates
/// every clone's next [`snapshot`](Self::snapshot), while prior snapshots stay valid.
///
/// # Examples
///
/// ```ignore
/// let handle: CredentialHandle<SecretToken> = resolver.resolve::<ApiKeyCredential>("my-cred").await?;
/// let token: Arc<SecretToken> = handle.snapshot();
/// token.token().expose_secret();
/// ```
pub struct CredentialHandle<S: AuthScheme> {
    inner: Arc<Inner<S>>,
}

impl<S: AuthScheme> Clone for CredentialHandle<S> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<S: AuthScheme> CredentialHandle<S> {
    /// Creates a new handle wrapping the given scheme.
    pub fn new(scheme: S, credential_id: impl Into<String>) -> Self {
        Self {
            inner: Arc::new(Inner {
                scheme: ArcSwap::from_pointee(scheme),
                credential_id: credential_id.into(),
            }),
        }
    }

    /// Returns an immutable snapshot of the current auth material.
    ///
    /// The returned `Arc<S>` is valid even if refresh swaps the
    /// underlying value concurrently. The next [`snapshot`](Self::snapshot)
    /// call returns the refreshed value.
    pub fn snapshot(&self) -> Arc<S> {
        self.inner.scheme.load_full()
    }

    /// Swaps in refreshed auth material.
    ///
    /// Used by [`CredentialResolver`](crate::runtime::CredentialResolver) after
    /// a successful refresh so callers holding this handle (or a [`Clone`] of
    /// it) observe the new scheme on their next [`snapshot`](Self::snapshot).
    pub(crate) fn replace(&self, next: S) {
        self.inner.scheme.store(Arc::new(next));
    }

    /// Returns the credential ID this handle was resolved from.
    pub fn credential_id(&self) -> &str {
        &self.inner.credential_id
    }
}

impl<S: AuthScheme + std::fmt::Debug> std::fmt::Debug for CredentialHandle<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CredentialHandle")
            .field("credential_id", &self.inner.credential_id)
            .field("scheme", &*self.inner.scheme.load_full())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SecretString, scheme::SecretToken};

    #[test]
    fn snapshot_returns_current_scheme() {
        let token = SecretToken::new(SecretString::new("abc"));
        let handle = CredentialHandle::new(token, "cred-1");

        let snapshot = handle.snapshot();
        let value = snapshot.token().expose_secret().to_owned();
        assert_eq!(value, "abc");
    }

    #[test]
    fn credential_id_is_preserved() {
        let token = SecretToken::new(SecretString::new("x"));
        let handle = CredentialHandle::new(token, "my-cred-id");
        assert_eq!(handle.credential_id(), "my-cred-id");
    }

    #[test]
    fn clone_shares_live_scheme() {
        let token = SecretToken::new(SecretString::new("shared"));
        let handle = CredentialHandle::new(token, "c1");
        let cloned = handle.clone();

        assert!(Arc::ptr_eq(&handle.snapshot(), &cloned.snapshot()));

        handle.replace(SecretToken::new(SecretString::new("updated")));
        let orig_val = handle.snapshot().token().expose_secret().to_owned();
        let clone_val = cloned.snapshot().token().expose_secret().to_owned();
        assert_eq!(orig_val, "updated");
        assert_eq!(clone_val, "updated");
    }

    #[test]
    fn replace_swaps_auth_material() {
        let token = SecretToken::new(SecretString::new("original"));
        let handle = CredentialHandle::new(token, "cred-1");

        let snap1 = handle.snapshot();
        assert_eq!(snap1.token().expose_secret(), "original");

        handle.replace(SecretToken::new(SecretString::new("refreshed")));

        let snap2 = handle.snapshot();
        assert_eq!(snap2.token().expose_secret(), "refreshed");

        // Old snapshot still valid
        assert_eq!(snap1.token().expose_secret(), "original");
    }

    #[test]
    fn debug_includes_credential_id() {
        let token = SecretToken::new(SecretString::new("secret"));
        let handle = CredentialHandle::new(token, "debug-test");
        let debug = format!("{handle:?}");
        assert!(debug.contains("debug-test"));
        // Token should be redacted
        assert!(debug.contains("REDACTED"));
    }
}
