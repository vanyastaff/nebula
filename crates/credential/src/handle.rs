//! Typed credential handle returned by the resolver.
//!
//! Wraps a resolved AuthScheme in an ArcSwap so the
//! RefreshCoordinator can hot-swap refreshed auth material without creating a new handle.

use std::sync::Arc;

use arc_swap::ArcSwap;
use nebula_core::AuthScheme;

/// Handle to a resolved credential with a specific AuthScheme.
///
/// Uses ArcSwap internally so the RefreshCoordinator can
/// hot-swap refreshed auth material. Callers use [`snapshot()`](Self::snapshot)
/// to get an immutable `Arc<S>` that remains valid even during concurrent refresh.
///
/// # Examples
///
/// ```ignore
/// let handle: CredentialHandle<SecretToken> = resolver.resolve::<ApiKeyCredential>("my-cred").await?;
/// let token: Arc<SecretToken> = handle.snapshot();
/// token.token().expose_secret(|t| request.header("Authorization", format!("Bearer {t}")));
/// ```
pub struct CredentialHandle<S: AuthScheme> {
    scheme: ArcSwap<S>,
    credential_id: String,
}

impl<S: AuthScheme> Clone for CredentialHandle<S> {
    fn clone(&self) -> Self {
        Self {
            scheme: ArcSwap::new(self.scheme.load_full()),
            credential_id: self.credential_id.clone(),
        }
    }
}

impl<S: AuthScheme> CredentialHandle<S> {
    /// Creates a new handle wrapping the given scheme.
    pub fn new(scheme: S, credential_id: impl Into<String>) -> Self {
        Self {
            scheme: ArcSwap::from_pointee(scheme),
            credential_id: credential_id.into(),
        }
    }

    /// Returns an immutable snapshot of the current auth material.
    ///
    /// The returned `Arc<S>` is valid even if refresh swaps the
    /// underlying value concurrently. Next `snapshot()` call returns
    /// the refreshed value.
    pub fn snapshot(&self) -> Arc<S> {
        self.scheme.load_full()
    }

    /// Swaps in refreshed auth material.
    ///
    /// Used by [`RefreshCoordinator`](crate::refresh::RefreshCoordinator)
    /// to hot-swap credentials after a successful refresh, without
    /// invalidating existing snapshots held by callers.
    // Reason: consumer (RefreshCoordinator) lands in task 1.5
    #[allow(dead_code)]
    pub(crate) fn replace(&self, next: S) {
        self.scheme.store(Arc::new(next));
    }

    /// Returns the credential ID this handle was resolved from.
    pub fn credential_id(&self) -> &str {
        &self.credential_id
    }
}

impl<S: AuthScheme + std::fmt::Debug> std::fmt::Debug for CredentialHandle<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CredentialHandle")
            .field("credential_id", &self.credential_id)
            .field("scheme", &*self.scheme.load_full())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheme::SecretToken;
    use nebula_core::SecretString;

    #[test]
    fn snapshot_returns_current_scheme() {
        let token = SecretToken::new(SecretString::new("abc"));
        let handle = CredentialHandle::new(token, "cred-1");

        let snapshot = handle.snapshot();
        let value = snapshot.token().expose_secret(|s| s.to_owned());
        assert_eq!(value, "abc");
    }

    #[test]
    fn credential_id_is_preserved() {
        let token = SecretToken::new(SecretString::new("x"));
        let handle = CredentialHandle::new(token, "my-cred-id");
        assert_eq!(handle.credential_id(), "my-cred-id");
    }

    #[test]
    fn clone_creates_independent_handle() {
        let token = SecretToken::new(SecretString::new("shared"));
        let handle = CredentialHandle::new(token, "c1");
        let cloned = handle.clone();

        // Both return the same underlying Arc
        assert!(Arc::ptr_eq(&handle.snapshot(), &cloned.snapshot()));

        // Replacing on one does not affect the other (independent ArcSwap)
        handle.replace(SecretToken::new(SecretString::new("updated")));
        let orig_val = handle.snapshot().token().expose_secret(|s| s.to_owned());
        let clone_val = cloned.snapshot().token().expose_secret(|s| s.to_owned());
        assert_eq!(orig_val, "updated");
        assert_eq!(clone_val, "shared");
    }

    #[test]
    fn replace_swaps_auth_material() {
        let token = SecretToken::new(SecretString::new("original"));
        let handle = CredentialHandle::new(token, "cred-1");

        let snap1 = handle.snapshot();
        assert_eq!(snap1.token().expose_secret(|s| s.to_owned()), "original");

        handle.replace(SecretToken::new(SecretString::new("refreshed")));

        let snap2 = handle.snapshot();
        assert_eq!(snap2.token().expose_secret(|s| s.to_owned()), "refreshed");

        // Old snapshot still valid
        assert_eq!(snap1.token().expose_secret(|s| s.to_owned()), "original");
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
