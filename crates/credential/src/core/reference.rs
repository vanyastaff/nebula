//! Credential references and provider traits.
//!
//! Provides type-safe references to credentials and a common provider interface
//! that decouples credential acquisition from the concrete Manager implementation.

use std::any::TypeId;
use std::fmt;
use std::future::Future;

use crate::core::{CredentialContext, CredentialError, SecretString};

/// Type-safe reference to a credential.
///
/// Wraps a `TypeId` to identify a credential type. Used to request credentials
/// from providers with compile-time and runtime type safety.
///
/// # Example
/// ```rust
/// use nebula_credential::CredentialRef;
/// use std::any::TypeId;
///
/// struct GithubToken;
///
/// let cred_ref = CredentialRef::of::<GithubToken>();
/// assert_eq!(cred_ref.type_id(), TypeId::of::<GithubToken>());
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct CredentialRef(TypeId);

impl CredentialRef {
    /// Create a credential reference from a type.
    ///
    /// # Example
    /// ```rust
    /// use nebula_credential::CredentialRef;
    ///
    /// struct ApiToken;
    /// let cred_ref = CredentialRef::of::<ApiToken>();
    /// ```
    pub const fn of<T: 'static>() -> Self {
        Self(TypeId::of::<T>())
    }

    /// Get the underlying type ID.
    pub const fn type_id(self) -> TypeId {
        self.0
    }
}

impl fmt::Debug for CredentialRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("CredentialRef").field(&self.0).finish()
    }
}

impl fmt::Display for CredentialRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CredentialRef({:?})", self.0)
    }
}

impl<T: 'static> From<std::marker::PhantomData<T>> for CredentialRef {
    fn from(_: std::marker::PhantomData<T>) -> Self {
        Self::of::<T>()
    }
}

/// Provider trait for acquiring credentials.
///
/// Decouples credential acquisition from the concrete [`CredentialManager`](crate::manager::CredentialManager)
/// implementation. Supports both type-based and string-based credential acquisition.
///
/// # Example Implementation
///
/// ```rust,ignore
/// use nebula_credential::{CredentialProvider, CredentialRef, CredentialContext, SecretString};
///
/// struct MyProvider {
///     manager: Arc<CredentialManager>,
/// }
///
/// impl CredentialProvider for MyProvider {
///     // Type-safe acquisition by credential type
///     async fn credential<C: CredentialType>(
///         &self,
///         ctx: &CredentialContext,
///     ) -> Result<SecretString, CredentialError> {
///         let type_id = TypeId::of::<C>();
///         self.manager.get_by_type(type_id, ctx).await
///     }
///
///     // Dynamic acquisition by string ID
///     async fn get(
///         &self,
///         id: &str,
///         ctx: &CredentialContext,
///     ) -> Result<SecretString, CredentialError> {
///         self.manager.get(id, ctx).await
///     }
/// }
/// ```
pub trait CredentialProvider: Send + Sync {
    /// Acquire a credential by type.
    ///
    /// Type-safe method that uses the credential type to identify and retrieve
    /// the credential value.
    ///
    /// # Example
    /// ```rust,ignore
    /// // Define a credential type
    /// struct GithubToken;
    ///
    /// // Acquire it
    /// let token = provider.credential::<GithubToken>(&ctx).await?;
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`CredentialError`] if the credential doesn't exist or acquisition fails.
    fn credential<C: Send + 'static>(
        &self,
        ctx: &CredentialContext,
    ) -> impl Future<Output = Result<SecretString, CredentialError>> + Send;

    /// Acquire a credential by string ID (type-erased).
    ///
    /// Returns a credential that is identified by its string ID.
    /// Use this when the credential type is not known at compile time.
    ///
    /// # Example
    /// ```rust,ignore
    /// let token = provider.get("github_token", &ctx).await?;
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`CredentialError`] if the credential doesn't exist or acquisition fails.
    fn get(
        &self,
        id: &str,
        ctx: &CredentialContext,
    ) -> impl Future<Output = Result<SecretString, CredentialError>> + Send;

    /// Check if a credential exists by type.
    fn has_credential<C: Send + 'static>(
        &self,
        ctx: &CredentialContext,
    ) -> impl Future<Output = bool> + Send {
        async move { self.credential::<C>(ctx).await.is_ok() }
    }

    /// Check if a credential exists by ID.
    fn has(&self, id: &str, ctx: &CredentialContext) -> impl Future<Output = bool> + Send {
        async move { self.get(id, ctx).await.is_ok() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct GithubToken;
    struct SlackToken;

    #[test]
    fn test_credential_ref_creation() {
        let github_ref = CredentialRef::of::<GithubToken>();
        assert_eq!(github_ref.type_id(), TypeId::of::<GithubToken>());
    }

    #[test]
    fn test_credential_ref_equality() {
        let ref1 = CredentialRef::of::<GithubToken>();
        let ref2 = CredentialRef::of::<GithubToken>();
        let ref3 = CredentialRef::of::<SlackToken>();

        // Same type
        assert_eq!(ref1, ref2);
        // Different type
        assert_ne!(ref1, ref3);
    }

    #[test]
    fn test_credential_ref_type_safety() {
        let github_ref = CredentialRef::of::<GithubToken>();
        let slack_ref = CredentialRef::of::<SlackToken>();

        // Different TypeIds for different types
        assert_ne!(github_ref.type_id(), slack_ref.type_id());
    }

    #[test]
    fn test_credential_ref_const() {
        // Can be used in const contexts
        const GITHUB_REF: CredentialRef = CredentialRef::of::<GithubToken>();
        assert_eq!(GITHUB_REF.type_id(), TypeId::of::<GithubToken>());
    }

    #[test]
    fn test_credential_ref_clone_copy() {
        let ref1 = CredentialRef::of::<GithubToken>();
        let ref2 = ref1; // Copy
        let ref3 = ref1.clone(); // Clone

        assert_eq!(ref1, ref2);
        assert_eq!(ref1, ref3);
    }
}
