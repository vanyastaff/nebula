use crate::core::{AccessToken, CredentialError};
use async_trait::async_trait;

/// Trait for creating authenticated clients from tokens
///
/// This trait allows creating authenticated clients from access tokens.
/// It's designed to be composable and work with any client type.
///
/// # Example
/// ```ignore
/// use nebula_credential::prelude::*;
///
/// struct MyAuthenticator;
///
/// #[async_trait]
/// impl ClientAuthenticator for MyAuthenticator {
///     type Target = reqwest::RequestBuilder;
///     type Output = reqwest::RequestBuilder;
///
///     async fn authenticate(
///         &self,
///         request: Self::Target,
///         token: &AccessToken,
///     ) -> Result<Self::Output, CredentialError> {
///         let auth_header = format!("Bearer {}", token.token.expose());
///         Ok(request.header("Authorization", auth_header))
///     }
/// }
/// ```
#[async_trait]
pub trait ClientAuthenticator: Send + Sync {
    /// Input type (what we start with)
    type Target;

    /// Output type (what we produce)
    type Output;

    /// Authenticate and create the client
    async fn authenticate(
        &self,
        target: Self::Target,
        token: &AccessToken,
    ) -> Result<Self::Output, CredentialError>;
}

/// Extension trait for easy use with credential manager
///
/// This trait provides a convenient `.authenticate_with()` method
/// that can be called on any type that can be authenticated.
///
/// # Example
/// ```ignore
/// use nebula_credential::prelude::*;
///
/// let token = manager.get_token(&cred_id).await?;
/// let authenticator = HttpBearer;
///
/// // Instead of: authenticator.authenticate(request, &token).await
/// let request = client.get("https://api.example.com")
///     .authenticate_with(&authenticator, &token)
///     .await?;
/// ```
#[async_trait]
pub trait AuthenticateWith: Sized {
    /// Create authenticated client using the authenticator
    async fn authenticate_with<A>(
        self,
        authenticator: &A,
        token: &AccessToken,
    ) -> Result<A::Output, CredentialError>
    where
        A: ClientAuthenticator<Target = Self>;
}

/// Implement for all types that can be targets
#[async_trait]
impl<T> AuthenticateWith for T
where
    T: Send,
{
    async fn authenticate_with<A>(
        self,
        authenticator: &A,
        token: &AccessToken,
    ) -> Result<A::Output, CredentialError>
    where
        A: ClientAuthenticator<Target = Self>,
    {
        authenticator.authenticate(self, token).await
    }
}
