use crate::core::{AccessToken, CredentialError};
use async_trait::async_trait;

/// Trait for creating authenticated clients from tokens
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