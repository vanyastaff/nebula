//! Typed credential handle returned by the resolver.
//!
//! Wraps a resolved [`AuthScheme`] in an [`Arc`] for cheap cloning
//! and tracks the credential ID it was resolved from.

use std::sync::Arc;

use nebula_core::AuthScheme;

/// Handle to a resolved credential with a specific [`AuthScheme`].
///
/// Obtained from [`CredentialResolver::resolve()`](crate::resolver::CredentialResolver::resolve).
/// The underlying scheme data is immutable and shared via [`Arc`].
///
/// # Examples
///
/// ```ignore
/// let handle: CredentialHandle<BearerToken> = resolver.resolve::<ApiKeyCredential>("my-cred").await?;
/// let token: &BearerToken = handle.snapshot();
/// request.header("Authorization", token.bearer_header());
/// ```
#[derive(Clone)]
pub struct CredentialHandle<S: AuthScheme> {
    scheme: Arc<S>,
    credential_id: String,
}

impl<S: AuthScheme> CredentialHandle<S> {
    /// Creates a new handle wrapping the given scheme.
    pub fn new(scheme: S, credential_id: impl Into<String>) -> Self {
        Self {
            scheme: Arc::new(scheme),
            credential_id: credential_id.into(),
        }
    }

    /// Returns a reference to the resolved auth scheme.
    pub fn snapshot(&self) -> &S {
        &self.scheme
    }

    /// Returns a cloned [`Arc`] to the scheme for sharing across tasks.
    pub fn shared(&self) -> Arc<S> {
        Arc::clone(&self.scheme)
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
            .field("scheme", &self.scheme)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheme::BearerToken;
    use crate::utils::SecretString;

    #[test]
    fn snapshot_returns_scheme_reference() {
        let token = BearerToken::new(SecretString::new("abc"));
        let handle = CredentialHandle::new(token, "cred-1");

        let snapshot = handle.snapshot();
        let value = snapshot.expose().expose_secret(|s| s.to_owned());
        assert_eq!(value, "abc");
    }

    #[test]
    fn credential_id_is_preserved() {
        let token = BearerToken::new(SecretString::new("x"));
        let handle = CredentialHandle::new(token, "my-cred-id");
        assert_eq!(handle.credential_id(), "my-cred-id");
    }

    #[test]
    fn clone_shares_same_arc() {
        let token = BearerToken::new(SecretString::new("shared"));
        let handle = CredentialHandle::new(token, "c1");
        let cloned = handle.clone();

        assert!(Arc::ptr_eq(&handle.shared(), &cloned.shared()));
    }

    #[test]
    fn debug_includes_credential_id() {
        let token = BearerToken::new(SecretString::new("secret"));
        let handle = CredentialHandle::new(token, "debug-test");
        let debug = format!("{handle:?}");
        assert!(debug.contains("debug-test"));
        // Token should be redacted
        assert!(debug.contains("REDACTED"));
    }
}
