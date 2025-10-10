//! Core traits for credential flows and interactive authentication

use async_trait::async_trait;
use serde::{Serialize, de::DeserializeOwned};

use crate::core::{
    CredentialContext, CredentialError, CredentialMetadata, CredentialState,
    result::{InitializeResult, PartialState, UserInput},
};

/// Base credential trait
///
/// The `Credential` trait defines the interface for credential implementations.
/// It supports both simple (non-interactive) and complex (interactive) flows.
///
/// # Type Parameters
/// - `Input`: Parameters needed to initialize the credential (e.g., {`api_key`}, {username, password}, {`client_id`, `client_secret`})
/// - `State`: The persisted state of the credential (can contain sensitive information, tokens, etc.)
#[async_trait]
pub trait Credential: Send + Sync + 'static {
    /// Input type for initialization
    type Input: Serialize + DeserializeOwned + Send + Sync + 'static;

    /// State type for persistence
    type State: CredentialState;

    /// Get metadata about this credential
    fn metadata(&self) -> CredentialMetadata;

    /// Initialize credential from input
    ///
    /// Can return:
    /// - `Complete` for simple flows (API keys, static tokens)
    /// - `RequiresInteraction` or `Pending` for interactive flows (`OAuth2`, SAML, 2FA)
    async fn initialize(
        &self,
        input: &Self::Input,
        ctx: &mut CredentialContext,
    ) -> Result<InitializeResult<Self::State>, CredentialError>;

    /// Refresh the credential
    async fn refresh(
        &self,
        state: &mut Self::State,
        ctx: &mut CredentialContext,
    ) -> Result<(), CredentialError>;

    /// Revoke the credential (optional)
    async fn revoke(
        &self,
        state: &mut Self::State,
        ctx: &mut CredentialContext,
    ) -> Result<(), CredentialError>;
}

/// Trait for credentials that support interactive flows
///
/// Implement this trait for credentials that require user interaction
/// (`OAuth2` authorization code flow, SAML, device flow, 2FA, etc.)
#[async_trait]
pub trait InteractiveCredential: Credential {
    /// Continue flow after user interaction
    ///
    /// Called by the manager when user provides input for a pending flow.
    /// The `partial_state` contains intermediate data from the previous step.
    async fn continue_initialization(
        &self,
        partial_state: PartialState,
        user_input: UserInput,
        ctx: &mut CredentialContext,
    ) -> Result<InitializeResult<Self::State>, CredentialError>;
}
