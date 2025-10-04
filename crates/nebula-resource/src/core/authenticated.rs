//! Authenticated resource trait for credential-based resources

use crate::core::error::ResourceResult;
use async_trait::async_trait;

#[cfg(feature = "credentials")]
use crate::credentials::ResourceCredentialProvider;

/// Trait for resources that support authentication
///
/// This trait enables resources to create authenticated clients
/// using credentials managed by nebula-credential.
///
/// # Example
/// ```ignore
/// use nebula_resource::prelude::*;
///
/// impl AuthenticatedResource for HttpClient {
///     type Client = reqwest::Client;
///
///     async fn get_authenticated_client(
///         &self,
///         provider: &ResourceCredentialProvider,
///     ) -> ResourceResult<Self::Client> {
///         let token = provider.get_token().await?;
///         // Create authenticated client
///         Ok(client)
///     }
/// }
/// ```
#[cfg(feature = "credentials")]
#[async_trait]
pub trait AuthenticatedResource: Send + Sync {
    /// The authenticated client type this resource provides
    type Client: Send + Sync;

    /// Get an authenticated client using the credential provider
    ///
    /// # Arguments
    /// * `provider` - Credential provider for token management
    ///
    /// # Returns
    /// Authenticated client ready to use
    async fn get_authenticated_client(
        &self,
        provider: &ResourceCredentialProvider,
    ) -> ResourceResult<Self::Client>;
}
