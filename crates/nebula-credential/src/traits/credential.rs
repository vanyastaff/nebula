use crate::core::{
    AccessToken, CredentialContext, CredentialError, CredentialMetadata, CredentialState,
};
use async_trait::async_trait;
use serde::{de::DeserializeOwned, Serialize};

/// Main trait for credential implementations
#[async_trait]
pub trait Credential: Send + Sync + 'static {
    /// Input type for initialization
    type Input: Serialize + DeserializeOwned + Send + Sync;

    /// State type for persistence
    type State: CredentialState;

    /// Type name constant
    const TYPE_NAME: &'static str = Self::State::KIND;

    /// Get metadata about this credential
    fn metadata(&self) -> CredentialMetadata;

    /// Initialize credential from input
    async fn initialize(
        &self,
        input: &Self::Input,
        ctx: &mut CredentialContext,
    ) -> Result<(Self::State, Option<AccessToken>), CredentialError>;

    /// Refresh the credential
    async fn refresh(
        &self,
        state: &mut Self::State,
        ctx: &mut CredentialContext,
    ) -> Result<AccessToken, CredentialError> {
        Err(CredentialError::refresh_not_supported(Self::TYPE_NAME.to_string()))
    }

    /// Revoke the credential (optional)
    async fn revoke(
        &self,
        state: &mut Self::State,
        ctx: &mut CredentialContext,
    ) -> Result<(), CredentialError> {
        Ok(())
    }

    /// Validate the credential state
    async fn validate(
        &self,
        state: &Self::State,
        ctx: &CredentialContext,
    ) -> Result<bool, CredentialError> {
        Ok(true)
    }
}
