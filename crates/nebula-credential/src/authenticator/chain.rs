use super::ClientAuthenticator;
use crate::core::{AccessToken, CredentialError};
use async_trait::async_trait;

/// Chain multiple authenticators together
pub struct ChainAuthenticator<A, B> {
    /// First authenticator
    pub first: A,
    /// Second authenticator
    pub second: B,
}

impl<A, B> ChainAuthenticator<A, B> {
    /// Create new chain
    pub fn new(first: A, second: B) -> Self {
        Self { first, second }
    }
}

#[async_trait]
impl<A, B> ClientAuthenticator for ChainAuthenticator<A, B>
where
    A: ClientAuthenticator + Send + Sync,
    B: ClientAuthenticator<Target = A::Output> + Send + Sync,
    A::Target: Send,
    A::Output: Send,
    B::Output: Send,
{
    type Target = A::Target;
    type Output = B::Output;

    async fn authenticate(
        &self,
        target: Self::Target,
        token: &AccessToken,
    ) -> Result<Self::Output, CredentialError> {
        let intermediate = self.first.authenticate(target, token).await?;
        self.second.authenticate(intermediate, token).await
    }
}