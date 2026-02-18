//! Core traits for credential flows and interactive authentication

use async_trait::async_trait;
use serde::{Serialize, de::DeserializeOwned};

use crate::core::{
    CredentialContext, CredentialDescription, CredentialError, CredentialState,
    result::{InitializeResult, PartialState, UserInput},
};

/// Core credential trait — describes a concrete credential type.
///
/// Defines the schema (via `description()`) and initialization logic.
/// `refresh` and `revoke` are **not** here — implement [`Refreshable`] or
/// [`Revocable`] only when the credential actually supports those operations.
///
/// # Type Parameters
/// - `Input`: Parameters needed to initialize (matches `description().properties`)
/// - `State`: Persisted state produced after `initialize`
#[async_trait]
pub trait CredentialType: Send + Sync + 'static {
    /// Input type for initialization
    type Input: Serialize + DeserializeOwned + Send + Sync + 'static;

    /// Persisted state type
    type State: CredentialState;

    /// Static description: key, name, icon, parameter schema.
    ///
    /// Called once and cached — no `&self` needed.
    fn description() -> CredentialDescription
    where
        Self: Sized;

    /// Initialize credential from user input.
    ///
    /// Returns:
    /// - `Complete(state)` — for simple flows (API keys, static tokens)
    /// - `RequiresInteraction` / `Pending` — for interactive flows (OAuth2, SAML, 2FA)
    async fn initialize(
        &self,
        input: &Self::Input,
        ctx: &mut CredentialContext,
    ) -> Result<InitializeResult<Self::State>, CredentialError>;
}

/// Opt-in: credential supports token/secret refresh (OAuth2, JWT, etc.)
///
/// Implement only when the credential has a limited lifetime and can be
/// renewed without user interaction.
#[async_trait]
pub trait Refreshable: CredentialType {
    async fn refresh(
        &self,
        state: &mut Self::State,
        ctx: &mut CredentialContext,
    ) -> Result<(), CredentialError>;
}

/// Opt-in: credential supports explicit revocation (OAuth2 token revoke, etc.)
///
/// Implement only when the service provides a revocation endpoint or mechanism.
#[async_trait]
pub trait Revocable: CredentialType {
    async fn revoke(
        &self,
        state: &mut Self::State,
        ctx: &mut CredentialContext,
    ) -> Result<(), CredentialError>;
}

/// Trait for credentials that support interactive flows.
///
/// Implement for credentials requiring user interaction:
/// OAuth2 authorization code flow, SAML, device flow, 2FA, etc.
#[async_trait]
pub trait InteractiveCredential: CredentialType {
    /// Continue flow after user interaction.
    ///
    /// Called by the manager when user provides input for a pending flow.
    async fn continue_initialization(
        &self,
        partial_state: PartialState,
        user_input: UserInput,
        ctx: &mut CredentialContext,
    ) -> Result<InitializeResult<Self::State>, CredentialError>;
}
