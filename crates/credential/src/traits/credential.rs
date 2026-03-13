//! Core traits for credential flows and interactive authentication

use async_trait::async_trait;
use serde::{Serialize, de::DeserializeOwned};

use nebula_parameter::schema::Schema;
use nebula_parameter::values::FieldValues;

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

    /// Stable protocol key (D-015) derived from the static description.
    ///
    /// By default, this uses `description().key` and validates it as a `CredentialKey`.
    /// Macro-generated credential types can rely on this without overriding.
    fn credential_key() -> nebula_core::CredentialKey
    where
        Self: Sized,
    {
        nebula_core::CredentialKey::new(Self::description().key.clone())
            .expect("invalid credential key in CredentialType::description()")
    }
}

/// Declares how the resource pool reacts when this resource's credential rotates.
///
/// Choose based on where authentication state lives in the client:
/// - Token in a header/field you can swap → `HotSwap`
/// - Password baked into a connection at connect-time → `DrainAndRecreate`
/// - Session-level auth (SSH, LDAP bind) → `Reconnect`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RotationStrategy {
    /// Call `authorize()` on all live instances. In-flight requests complete
    /// with old credential; new requests get the new credential immediately.
    /// Good for: HTTP bearer tokens, API keys in headers, gRPC metadata.
    #[default]
    HotSwap,

    /// Gracefully drain the pool (in-flight complete), then recreate all
    /// instances with the new credential. New instances call `authorize()` after creation.
    /// Good for: database connections, Redis AUTH, any connection-level auth.
    DrainAndRecreate,

    /// Immediately close all instances. Next acquire triggers fresh creation.
    /// Good for: SSH sessions, LDAP binds, any session-level auth.
    Reconnect,
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
    fn parameters() -> Schema
    where
        Self: Sized;

    /// Build state from flat parameter values.
    ///
    /// Called by the macro-generated `initialize()` when `extends` is set.
    /// `values` contains the full flat input (protocol fields + own fields).
    fn build_state(values: &FieldValues) -> Result<Self::State, CredentialError>
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
    fn parameters() -> Schema
    where
        Self: Sized;

    /// Execute the authentication flow
    async fn initialize(
        config: &Self::Config,
        values: &FieldValues,
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

    /// How the resource pool handles credential rotation.
    /// Override only if `HotSwap` is not correct for this resource.
    fn rotation_strategy() -> RotationStrategy
    where
        Self: Sized,
    {
        RotationStrategy::HotSwap
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestApiKey;
    struct TestDbCred;

    #[async_trait]
    impl CredentialType for TestApiKey {
        type Input = ();
        type State = crate::protocols::ApiKeyState;

        fn description() -> CredentialDescription
        where
            Self: Sized,
        {
            CredentialDescription::builder()
                .key("test_api_key")
                .name("Test API Key")
                .description("Test API key credential")
                .properties(nebula_parameter::schema::Schema::new())
                .build()
                .unwrap()
        }

        async fn initialize(
            &self,
            _input: &Self::Input,
            _ctx: &mut CredentialContext,
        ) -> Result<InitializeResult<Self::State>, CredentialError> {
            unreachable!()
        }
    }

    #[async_trait]
    impl CredentialType for TestDbCred {
        type Input = ();
        type State = crate::protocols::DatabaseState;

        fn description() -> CredentialDescription
        where
            Self: Sized,
        {
            CredentialDescription::builder()
                .key("test_db_cred")
                .name("Test DB Cred")
                .description("Test database credential")
                .properties(nebula_parameter::schema::Schema::new())
                .build()
                .unwrap()
        }

        async fn initialize(
            &self,
            _input: &Self::Input,
            _ctx: &mut CredentialContext,
        ) -> Result<InitializeResult<Self::State>, CredentialError> {
            unreachable!()
        }
    }

    struct MyHttpClient;

    impl CredentialResource for MyHttpClient {
        type Credential = TestApiKey;

        fn authorize(&mut self, _: &<Self::Credential as CredentialType>::State) {}
    }

    struct MyDbPool;

    impl CredentialResource for MyDbPool {
        type Credential = TestDbCred;

        fn authorize(&mut self, _: &<Self::Credential as CredentialType>::State) {}

        fn rotation_strategy() -> RotationStrategy
        where
            Self: Sized,
        {
            RotationStrategy::DrainAndRecreate
        }
    }

    #[test]
    fn default_rotation_strategy_is_hotswap() {
        assert!(matches!(
            MyHttpClient::rotation_strategy(),
            RotationStrategy::HotSwap
        ));
    }

    #[test]
    fn db_resource_declares_drain_and_recreate() {
        assert!(matches!(
            MyDbPool::rotation_strategy(),
            RotationStrategy::DrainAndRecreate
        ));
    }
}
