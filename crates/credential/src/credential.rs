//! Unified credential trait (v2).
//!
//! The [`Credential`] trait replaces six v1 traits (`CredentialType`,
//! `FlowProtocol`, `InteractiveCredential`, `Refreshable`, `Revocable`,
//! `StaticProtocol`) with a single trait that covers the full lifecycle:
//!
//! - **Resolve** user input into stored state (single-step or interactive).
//! - **Project** consumer-facing auth material from stored state.
//! - **Refresh** expiring tokens.
//! - **Test** that a credential actually works.
//! - **Revoke** a credential.
//!
//! Capability flags (`INTERACTIVE`, `REFRESHABLE`, etc.) are associated
//! consts -- zero-cost, compile-time, no allocation.

use std::future::Future;

use nebula_core::AuthScheme;
use nebula_parameter::ParameterCollection;
use nebula_parameter::values::ParameterValues;

use crate::context::CredentialContext;
use crate::description::CredentialDescription;
use crate::error::CredentialError;
use crate::pending::PendingState;
use crate::resolve::{RefreshOutcome, RefreshPolicy, ResolveResult, TestResult, UserInput};
use crate::state::CredentialState;

/// Unified trait for all credential types.
///
/// One trait replaces six v1 traits. Three associated types pin the
/// generic parameters at the impl site; five associated consts declare
/// capabilities at compile time.
///
/// # Associated types
///
/// - **`Scheme`** -- consumer-facing auth material ([`AuthScheme`]).
/// - **`State`** -- what gets encrypted and stored
///   ([`CredentialState`]). May include refresh internals not exposed
///   to consumers.
/// - **`Pending`** -- typed ephemeral state for interactive flows
///   ([`PendingState`]). Non-interactive credentials use
///   [`NoPendingState`](crate::pending::NoPendingState).
///
/// # Capability consts
///
/// | Const | Default | Meaning |
/// |---|---|---|
/// | `INTERACTIVE` | `false` | Supports multi-step resolve |
/// | `REFRESHABLE` | `false` | Supports token refresh |
/// | `REVOCABLE` | `false` | Supports explicit revocation |
/// | `TESTABLE` | `false` | Supports live testing |
/// | `REFRESH_POLICY` | 5 min early / 5 s backoff / 30 s jitter | Refresh timing |
///
/// # Methods
///
/// Only [`resolve`](Credential::resolve) requires a real implementation.
/// All other methods have sensible defaults.
///
/// # Examples
///
/// ```ignore
/// use nebula_credential::{
///     Credential, NoPendingState, identity_state,
///     scheme::SecretToken,
///     resolve::StaticResolveResult,
/// };
///
/// struct SlackBotToken;
///
/// identity_state!(SecretToken, "secret_token", 1);
///
/// impl Credential for SlackBotToken {
///     type Scheme = SecretToken;
///     type State = SecretToken;
///     type Pending = NoPendingState;
///
///     const KEY: &'static str = "slack_bot_token";
///
///     fn description() -> CredentialDescription { /* ... */ }
///     fn parameters() -> ParameterCollection { /* ... */ }
///     fn project(state: &SecretToken) -> SecretToken { state.clone() }
///
///     fn resolve(
///         values: &ParameterValues,
///         _ctx: &CredentialContext,
///     ) -> impl Future<Output = Result<StaticResolveResult<SecretToken>, CredentialError>> + Send {
///         async move {
///             let token = values.get_string("bot_token").unwrap_or_default();
///             Ok(StaticResolveResult::Complete(SecretToken::new(SecretString::new(token))))
///         }
///     }
/// }
/// ```
pub trait Credential: Send + Sync + 'static {
    /// What this credential produces -- the consumer-facing auth material.
    type Scheme: AuthScheme;

    /// What gets stored -- may include refresh internals not exposed to
    /// resources. For static credentials: same type as `Scheme`
    /// (use [`identity_state!`](crate::identity_state) macro).
    type State: CredentialState;

    /// Typed pending state for interactive flows.
    ///
    /// Non-interactive credentials: use
    /// [`NoPendingState`](crate::pending::NoPendingState).
    /// No default -- associated type defaults are unstable on stable Rust.
    /// The `#[derive(Credential)]` macro fills `NoPendingState` automatically.
    type Pending: PendingState;

    /// Stable key for this credential type (e.g., `"github_oauth2"`).
    const KEY: &'static str;

    /// Whether this credential requires multi-step interactive resolution.
    const INTERACTIVE: bool = false;

    /// Whether this credential supports token refresh.
    const REFRESHABLE: bool = false;

    /// Whether this credential supports explicit revocation.
    const REVOCABLE: bool = false;

    /// Whether this credential supports live testing.
    const TESTABLE: bool = false;

    /// Refresh timing policy -- controls early refresh, retry backoff,
    /// and jitter.
    const REFRESH_POLICY: RefreshPolicy = RefreshPolicy::DEFAULT;

    /// Human-readable metadata: name, icon, documentation URL.
    fn description() -> CredentialDescription
    where
        Self: Sized;

    /// Parameter schema for the setup form.
    fn parameters() -> ParameterCollection
    where
        Self: Sized;

    /// Extract consumer-facing auth material from stored state.
    fn project(state: &Self::State) -> Self::Scheme
    where
        Self: Sized;

    /// Resolve user input into credential state.
    ///
    /// **Framework handles `PendingState` storage.** Credential returns
    /// raw `Pending { state, interaction }` -- framework encrypts, stores,
    /// generates `PendingToken`, and manages the lifecycle. Credential
    /// author never calls `store_pending()` or `consume_pending()`.
    ///
    /// For non-interactive credentials: use
    /// [`StaticResolveResult<S>`](crate::resolve::StaticResolveResult).
    fn resolve(
        values: &ParameterValues,
        ctx: &CredentialContext,
    ) -> impl Future<Output = Result<ResolveResult<Self::State, Self::Pending>, CredentialError>> + Send
    where
        Self: Sized;

    /// Continue interactive resolve after user completes interaction.
    ///
    /// Framework loads and consumes `PendingState` before calling this.
    /// The `pending` parameter is the typed state returned by `resolve()`.
    ///
    /// Default: returns [`CredentialError::NotInteractive`].
    fn continue_resolve(
        _pending: &Self::Pending,
        _input: &UserInput,
        _ctx: &CredentialContext,
    ) -> impl Future<Output = Result<ResolveResult<Self::State, Self::Pending>, CredentialError>> + Send
    where
        Self: Sized,
    {
        async { Err(CredentialError::NotInteractive) }
    }

    /// Test that the credential actually works.
    ///
    /// Default: returns [`TestResult::Untestable`].
    fn test(
        _scheme: &Self::Scheme,
        _ctx: &CredentialContext,
    ) -> impl Future<Output = Result<TestResult, CredentialError>> + Send
    where
        Self: Sized,
    {
        async { Ok(TestResult::Untestable) }
    }

    /// Refresh expiring auth material.
    ///
    /// Default: returns [`RefreshOutcome::NotSupported`].
    fn refresh(
        _state: &mut Self::State,
        _ctx: &CredentialContext,
    ) -> impl Future<Output = Result<RefreshOutcome, CredentialError>> + Send
    where
        Self: Sized,
    {
        async { Ok(RefreshOutcome::NotSupported) }
    }

    /// Revoke credential at the provider.
    ///
    /// Default: no-op (succeeds silently).
    fn revoke(
        _state: &mut Self::State,
        _ctx: &CredentialContext,
    ) -> impl Future<Output = Result<(), CredentialError>> + Send
    where
        Self: Sized,
    {
        async { Ok(()) }
    }
}
