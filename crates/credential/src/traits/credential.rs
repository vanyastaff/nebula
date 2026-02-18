//! Core traits for credential flows and interactive authentication

use async_trait::async_trait;
use serde::{Serialize, de::DeserializeOwned};

use nebula_parameter::collection::ParameterCollection;
use nebula_parameter::values::ParameterValues;

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

/// Synchronous form-to-State protocol. No IO, no async.
///
/// Use for: API keys, Basic Auth, database credentials, header auth, and
/// other token-based credentials where initialization is pure form → State.
/// Protocols are purely static — no `&self`. They define a fixed schema
/// and default initialization logic that concrete [`CredentialType`]s can
/// inherit via `#[credential(extends = XyzProtocol)]`.
///
/// # Example
///
/// ```ignore
/// use nebula_credential::protocols::ApiKeyProtocol;
/// use nebula_macros::Credential;
///
/// #[derive(Credential)]
/// #[credential(key = "github-api", name = "GitHub API", extends = ApiKeyProtocol)]
/// pub struct GithubApi;
/// ```
pub trait StaticProtocol: Send + Sync + 'static {
    /// The state this protocol produces after initialization.
    type State: CredentialState;

    /// Parameters this protocol contributes.
    ///
    /// Merged first (before own params) by the macro.
    fn parameters() -> ParameterCollection
    where
        Self: Sized;

    /// Build state from flat parameter values.
    ///
    /// Called by the macro-generated `initialize()` when `extends` is set.
    /// `values` contains the full flat input (protocol fields + own fields).
    fn build_state(values: &ParameterValues) -> Result<Self::State, CredentialError>
    where
        Self: Sized;
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

/// Async multi-step protocol. Configurable per provider.
///
/// Use for: OAuth2, LDAP, SAML, Kerberos, mTLS.
/// Plugin implements `Config` type and uses macro attributes to wire it up.
#[allow(async_fn_in_trait)]
pub trait FlowProtocol: Send + Sync + 'static {
    /// Provider-specific configuration (endpoints, scopes, options)
    type Config: Send + Sync + 'static;

    /// State produced after successful flow completion
    type State: CredentialState;

    /// Parameters shown to user in UI (client_id, client_secret, etc.)
    fn parameters() -> ParameterCollection
    where
        Self: Sized;

    /// Execute the authentication flow
    async fn initialize(
        config: &Self::Config,
        values: &ParameterValues,
        ctx: &mut CredentialContext,
    ) -> Result<InitializeResult<Self::State>, CredentialError>
    where
        Self: Sized;

    /// Refresh an expired credential (default: no-op)
    async fn refresh(
        config: &Self::Config,
        state: &mut Self::State,
        ctx: &mut CredentialContext,
    ) -> Result<(), CredentialError>
    where
        Self: Sized,
    {
        let _ = (config, state, ctx);
        Ok(())
    }

    /// Revoke an active credential (default: no-op)
    async fn revoke(
        config: &Self::Config,
        state: &mut Self::State,
        ctx: &mut CredentialContext,
    ) -> Result<(), CredentialError>
    where
        Self: Sized,
    {
        let _ = (config, state, ctx);
        Ok(())
    }
}

/// Links a resource client to its required credential type at compile time.
///
/// The runtime retrieves the credential State automatically and calls
/// `authorize()` when creating or refreshing the resource instance.
pub trait CredentialResource {
    /// The credential type required by this resource
    type Credential: CredentialType;

    /// Apply credential state to authorize this resource's client.
    ///
    /// Called after the resource is created and whenever the credential
    /// is refreshed (e.g. OAuth2 token rotation).
    fn authorize(&mut self, state: &<Self::Credential as CredentialType>::State);
}
