//! Stateful authenticator integration for nebula-resource
//!
//! This module provides integration with nebula-credential's StatefulAuthenticator,
//! enabling resources to access full credential state (not just tokens).

#[cfg(feature = "credentials")]
use async_trait::async_trait;

#[cfg(feature = "credentials")]
use nebula_credential::{
    authenticator::StatefulAuthenticator,
    core::{CredentialError, CredentialId},
    traits::{Credential, StateStore},
    CredentialManager,
};

use crate::core::error::{ResourceError, ResourceResult};
use std::sync::Arc;

/// Resource that can be authenticated with full credential state
///
/// This trait enables resources to receive the full credential state
/// (e.g., username, password, host, port, database) rather than just
/// an access token.
///
/// # Example
/// ```ignore
/// use nebula_resource::credentials::stateful::StatefulResourceAuthenticator;
/// use nebula_credential::authenticator::StatefulAuthenticator;
///
/// impl StatefulAuthenticator<DatabaseCredential> for DatabaseResource {
///     type Target = DatabaseConfig;
///     type Output = DatabaseConnection;
///
///     async fn authenticate(
///         &self,
///         config: DatabaseConfig,
///         state: &DatabaseState,
///     ) -> Result<DatabaseConnection> {
///         // Access all fields: username, password, host, port, database
///         let conn = connect(
///             &state.host,
///             state.port,
///             &state.username,
///             &state.password,
///         ).await?;
///         Ok(conn)
///     }
/// }
/// ```
#[cfg(feature = "credentials")]
#[async_trait]
pub trait StatefulResourceAuthenticator<C: Credential>: Send + Sync {
    /// The configuration/target type to authenticate
    type Target;
    /// The authenticated output type
    type Output;

    /// Authenticate using full credential state
    ///
    /// # Arguments
    /// * `target` - Configuration or target to authenticate
    /// * `state` - Full credential state with all fields
    ///
    /// # Returns
    /// Authenticated resource or connection
    async fn authenticate(
        &self,
        target: Self::Target,
        state: &C::State,
    ) -> ResourceResult<Self::Output>;
}

/// Wrapper to bridge nebula-credential StatefulAuthenticator to ResourceResult
#[cfg(feature = "credentials")]
pub struct ResourceAuthenticatorBridge<A, C>
where
    A: StatefulAuthenticator<C>,
    C: Credential,
{
    authenticator: A,
    _phantom: std::marker::PhantomData<C>,
}

#[cfg(feature = "credentials")]
impl<A, C> ResourceAuthenticatorBridge<A, C>
where
    A: StatefulAuthenticator<C>,
    C: Credential,
{
    /// Create a new bridge
    pub fn new(authenticator: A) -> Self {
        Self {
            authenticator,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Authenticate with resource-compatible error handling
    pub async fn authenticate(
        &self,
        target: A::Target,
        state: &C::State,
    ) -> ResourceResult<A::Output> {
        self.authenticator
            .authenticate(target, state)
            .await
            .map_err(|e| {
                ResourceError::internal(
                    "stateful_authenticator",
                    format!("Authentication failed: {}", e),
                )
            })
    }
}

/// Provider for stateful credential access
///
/// This allows resources to fetch the full credential state from the manager,
/// not just the access token.
#[cfg(feature = "credentials")]
pub struct StatefulCredentialProvider {
    manager: Arc<CredentialManager>,
    credential_id: CredentialId,
}

#[cfg(feature = "credentials")]
impl StatefulCredentialProvider {
    /// Create a new stateful provider
    pub fn new(manager: Arc<CredentialManager>, credential_id: CredentialId) -> Self {
        Self {
            manager,
            credential_id,
        }
    }

    /// Get the credential manager
    pub fn manager(&self) -> &Arc<CredentialManager> {
        &self.manager
    }

    /// Get the credential ID
    pub fn credential_id(&self) -> &CredentialId {
        &self.credential_id
    }

    /// Get state for a specific credential type
    ///
    /// This retrieves the full credential state, allowing access to all fields.
    ///
    /// # Example
    /// ```ignore
    /// let provider = StatefulCredentialProvider::new(manager, cred_id);
    /// let state: DatabaseState = provider.get_state::<DatabaseCredential>().await?;
    ///
    /// // Now you can access all fields
    /// println!("Host: {}", state.host);
    /// println!("Port: {}", state.port);
    /// println!("Username: {}", state.username);
    /// // password is SecureString, use state.password.expose()
    /// ```
    pub async fn get_state<C>(&self) -> ResourceResult<C::State>
    where
        C: Credential,
        C::State: Clone,
    {
        // This would need to be implemented on CredentialManager
        // For now, return error indicating this needs implementation
        Err(ResourceError::internal(
            "stateful_provider",
            "get_state not yet implemented - requires CredentialManager API extension",
        ))
    }
}

/// Extension trait to simplify stateful authentication
#[cfg(feature = "credentials")]
#[async_trait]
pub trait AuthenticateWithStateful<C: Credential>: Sized {
    /// Authenticate this target using a stateful authenticator
    ///
    /// # Example
    /// ```ignore
    /// let config = DatabaseConfig { /* ... */ };
    /// let connection = config
    ///     .authenticate_with_stateful(&authenticator, &state)
    ///     .await?;
    /// ```
    async fn authenticate_with_stateful<A>(
        self,
        authenticator: &A,
        state: &C::State,
    ) -> ResourceResult<A::Output>
    where
        A: StatefulResourceAuthenticator<C, Target = Self>;
}

#[cfg(feature = "credentials")]
#[async_trait]
impl<T, C> AuthenticateWithStateful<C> for T
where
    T: Send,
    C: Credential,
{
    async fn authenticate_with_stateful<A>(
        self,
        authenticator: &A,
        state: &C::State,
    ) -> ResourceResult<A::Output>
    where
        A: StatefulResourceAuthenticator<C, Target = Self>,
    {
        authenticator.authenticate(self, state).await
    }
}

#[cfg(test)]
#[cfg(feature = "credentials")]
mod tests {
    use super::*;

    #[test]
    fn test_compile_time_checks() {
        // Just ensure types compile correctly
    }
}
