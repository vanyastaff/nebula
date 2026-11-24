//! Unified credential trait (CP6).
//!
//! Per Tech Spec §15.4 capability sub-trait split — the previous shape
//! (CP4) carried five `const X: bool = false` capability flags plus
//! defaulted method bodies for `continue_resolve` / `refresh` / `revoke`
//! / `test` / `release`. A plugin author setting `const REFRESHABLE =
//! true` while forgetting to override `refresh()` produced a credential
//! that *declared* refresh capability but silently returned a
//! "not supported" outcome — the engine treated this as a benign
//! outcome and the credential never refreshed in production. The same
//! silent-downgrade vector existed for the four sister capabilities.
//!
//! CP5/CP6 splits the capability surface into per-capability sub-traits
//! ([`Interactive`], [`Refreshable`], [`Revocable`], [`Testable`],
//! [`Dynamic`]) — each carries the corresponding method without a
//! defaulted body, so a plugin that fails to implement the capability
//! method fails to compile (`E0046`). Engine dispatchers bind by
//! sub-trait (`where C: Refreshable`); a non-`Refreshable` credential
//! cannot reach the refresh dispatcher (`E0277`). Probes 3 + 4 cement
//! both guarantees. This closes security-lead findings N1 + N3 + N5
//! at the type level.
//!
//! [`Interactive`]: crate::Interactive
//! [`Refreshable`]: crate::Refreshable
//! [`Revocable`]: crate::Revocable
//! [`Testable`]: crate::Testable
//! [`Dynamic`]: crate::Dynamic

use std::future::Future;

use nebula_component::{Component, kind};

use super::CredentialState;
use crate::{AuthScheme, CredentialContext, error::CredentialError, resolve::StaticResolveResult};

/// Unified trait for all credential types.
///
/// A credential is a [`Component`] of kind [`kind::Credential`]: its key,
/// its setup-form type (`Config`) and its catalog metadata are declared on
/// the `Component` impl. This trait adds what only a credential has.
///
/// # Integration credentials (Plane B)
///
/// Implementations of this trait are **integration credentials** —
/// secrets and auth material for **external** systems (SaaS APIs,
/// webhooks, databases), not for logging into Nebula's own API or
/// control plane. That host-facing authentication (**Plane A**) stays
/// out of this trait; see auth plane separation ().
///
/// # Associated types
///
/// - **`Scheme`** — consumer-facing auth material ([`AuthScheme`]).
/// - **`State`** — what gets encrypted and stored ([`CredentialState`]). May include refresh
///   internals not exposed to consumers.
///
/// The `Pending` associated type lives on [`Interactive`](crate::Interactive);
/// non-interactive credentials do not declare one.
///
/// # Capability sub-traits
///
/// Capabilities live in dedicated sub-traits — `impl
/// Refreshable for MyCred` declares refresh capability and forces
/// `refresh()` to be implemented (no default). Compile-time membership
/// (`where C: Refreshable`) replaces runtime const-bool checks.
///
/// | Capability | Sub-trait |
/// |------------|-----------|
/// | Interactive multi-step resolve | [`Interactive`](crate::Interactive) |
/// | Token refresh | [`Refreshable`](crate::Refreshable) |
/// | Provider-side revocation | [`Revocable`](crate::Revocable) |
/// | Live health probe | [`Testable`](crate::Testable) |
/// | Per-execution ephemeral lease | [`Dynamic`](crate::Dynamic) |
///
/// # Examples
///
/// ```
/// use nebula_credential::{
///     Component, Credential, CredentialContext, Metadata, SecretString, kind,
///     scheme::SecretToken,
/// };
/// use nebula_credential::error::CredentialError;
/// use nebula_credential::resolve::StaticResolveResult;
/// use serde::Deserialize;
///
/// struct SlackBotToken;
///
/// #[derive(Deserialize, nebula_schema::Schema)]
/// struct SlackBotConfig {
///     #[property(input(secret, required))]
///     bot_token: SecretString,
/// }
///
/// impl Component for SlackBotToken {
///     type Kind = kind::Credential;
///     type Config = SlackBotConfig;
///
///     const KEY: &'static str = "slack_bot_token";
///
///     fn metadata() -> Metadata {
///         Metadata::new(nebula_credential::metadata_name!("Slack Bot Token"),
///             "Slack bot OAuth token",
///         )
///     }
/// }
///
/// impl Credential for SlackBotToken {
///     // `State` == `Scheme` for static credentials: `SecretToken` is both an
///     // `AuthScheme` and (via `identity_state!`) a `CredentialState`.
///     type Scheme = SecretToken;
///     type State = SecretToken;
///
///     fn project(state: &SecretToken) -> SecretToken { state.clone() }
///
///     async fn resolve(
///         config: &SlackBotConfig,
///         _ctx: &CredentialContext,
///     ) -> Result<StaticResolveResult<SecretToken>, CredentialError> {
///         Ok(StaticResolveResult::Complete(SecretToken::new(config.bot_token.clone())))
///     }
/// }
///
/// assert_eq!(nebula_credential::key_of::<SlackBotToken>().as_str(), "slack_bot_token");
/// // `project` is a pure, synchronous extraction of auth material from state.
/// let state = SecretToken::new(SecretString::new("xoxb-123"));
/// let scheme = SlackBotToken::project(&state);
/// assert_eq!(scheme.token().expose_secret(), "xoxb-123");
/// ```
pub trait Credential: Component<Kind = kind::Credential> {
    /// What this credential produces — the consumer-facing auth material.
    type Scheme: AuthScheme;

    /// What gets stored — may include refresh internals not exposed to
    /// resources. For static credentials: same type as `Scheme`
    /// (use [`identity_state!`](crate::identity_state) macro).
    type State: CredentialState;

    /// Project the runtime [`Scheme`] from stored [`State`]. Synchronous,
    /// pure.
    ///
    /// [`Scheme`]: Credential::Scheme
    /// [`State`]: Credential::State
    fn project(state: &Self::State) -> Self::Scheme;

    /// Build initial [`Self::State`] from the typed config decoded once by the
    /// runtime after canonical schema validation. Returns
    /// [`StaticResolveResult<State>`](StaticResolveResult).
    ///
    /// Callers cannot provide or retain schema proof values. The runtime
    /// consumes literal authored data, completes data-only validation, and
    /// performs the trusted typed decode before invoking this method.
    ///
    /// Untyped test doubles may use `serde_json::Value` as `Config`, whose
    /// schema is gradual `Any`, not an empty record. Prefer a concrete config
    /// type for real credentials.
    ///
    /// The result can complete or request a retry. It cannot carry pending
    /// state; interactive credentials use [`Interactive::begin`](crate::Interactive::begin).
    fn resolve(
        config: &Self::Config,
        ctx: &CredentialContext,
    ) -> impl Future<Output = Result<StaticResolveResult<Self::State>, CredentialError>> + Send;
}
