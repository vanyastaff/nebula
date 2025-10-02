use crate::core::{AccessToken, CredentialError, CredentialState};
use crate::traits::Credential;
use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde::Serialize;

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

/// Trait for creating authenticated clients from credential state
///
/// This trait is more powerful than `ClientAuthenticator` as it has access
/// to the full credential state, not just the token. This is useful for
/// credentials that need multiple pieces of information (e.g., username + password).
///
/// # Type Parameters
/// * `C` - The credential type this authenticator works with
///
/// # Example
/// ```ignore
/// use nebula_credential::prelude::*;
///
/// struct PostgresAuthenticator;
///
/// #[async_trait]
/// impl<C> StatefulAuthenticator<C> for PostgresAuthenticator
/// where
///     C: Credential<State = PostgresState>,
/// {
///     type Target = PgConnectOptions;
///     type Output = PgPool;
///
///     async fn authenticate(
///         &self,
///         options: Self::Target,
///         state: &C::State,
///     ) -> Result<Self::Output, CredentialError> {
///         let pool = PgPoolOptions::new()
///             .connect_with(
///                 options
///                     .username(&state.username)
///                     .password(state.password.expose())
///             )
///             .await?;
///         Ok(pool)
///     }
/// }
/// ```
#[async_trait]
pub trait StatefulAuthenticator<C: Credential>: Send + Sync {
    /// Input type (what we start with)
    type Target;

    /// Output type (what we produce)
    type Output;

    /// Authenticate and create the client using full credential state
    ///
    /// # Arguments
    /// * `target` - The target object to authenticate (e.g., connection options)
    /// * `state` - Full credential state with all necessary information
    async fn authenticate(
        &self,
        target: Self::Target,
        state: &C::State,
    ) -> Result<Self::Output, CredentialError>;
}

/// Extension trait for easy use with stateful authenticators
#[async_trait]
pub trait AuthenticateWithState<C: Credential>: Sized {
    /// Create authenticated client using the stateful authenticator
    async fn authenticate_with_state<A>(
        self,
        authenticator: &A,
        state: &C::State,
    ) -> Result<A::Output, CredentialError>
    where
        A: StatefulAuthenticator<C, Target = Self>;
}

/// Implement for all types that can be targets
#[async_trait]
impl<T, C> AuthenticateWithState<C> for T
where
    T: Send,
    C: Credential,
{
    async fn authenticate_with_state<A>(
        self,
        authenticator: &A,
        state: &C::State,
    ) -> Result<A::Output, CredentialError>
    where
        A: StatefulAuthenticator<C, Target = Self>,
    {
        authenticator.authenticate(self, state).await
    }
}
