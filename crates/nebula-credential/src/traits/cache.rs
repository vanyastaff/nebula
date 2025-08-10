use crate::core::{AccessToken, CredentialError};
use async_trait::async_trait;
use std::time::Duration;

/// Trait for token caching
#[async_trait]
pub trait TokenCache: Send + Sync {
    /// Get token from cache
    async fn get(&self, key: &str) -> Result<Option<AccessToken>, CredentialError>;

    /// Put token with TTL
    async fn put(
        &self,
        key: &str,
        token: &AccessToken,
        ttl: Duration,
    ) -> Result<(), CredentialError>;

    /// Delete token from cache
    async fn del(&self, key: &str) -> Result<(), CredentialError>;

    /// Clear all cached tokens
    async fn clear(&self) -> Result<(), CredentialError> {
        Ok(())
    }

    /// Check cache health
    async fn health_check(&self) -> Result<(), CredentialError> {
        Ok(())
    }
}