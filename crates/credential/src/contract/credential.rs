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

use nebula_schema::FieldValues;

use super::CredentialState;
use crate::{
    AuthScheme, CredentialContext, CredentialMetadata, error::CredentialError,
    resolve::ResolveResult,
};

/// Unified trait for all credential types.
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
/// - **`Properties`** — typed shape of the setup-form fields
///   ([`HasSchema`](nebula_schema::HasSchema)). Mirrors `Action::Input` and `Resource::Config` —
///   the schema-bearing companion type for credentials per Phase 5 of the M6 dependency redesign.
///   Replaces the previous `Input` associated type.
/// - **`Scheme`** — consumer-facing auth material ([`AuthScheme`]).
/// - **`State`** — what gets encrypted and stored ([`CredentialState`]). May include refresh
///   internals not exposed to consumers.
///
/// The pre-§15.4 `Pending` associated type now lives on
/// [`Interactive`](crate::Interactive); non-interactive credentials no
/// longer declare a companion type.
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
///     AuthPattern, Credential, CredentialContext, CredentialMetadata, SecretString,
///     scheme::SecretToken,
/// };
/// use nebula_credential::error::CredentialError;
/// use nebula_credential::resolve::ResolveResult;
/// use nebula_core::credential_key;
/// use nebula_schema::{FieldValues, ValidSchema};
///
/// struct SlackBotToken;
///
/// impl Credential for SlackBotToken {
///     // `State` == `Scheme` for static credentials: `SecretToken` is both an
///     // `AuthScheme` and (via `identity_state!`) a `CredentialState`.
///     type Properties = FieldValues;
///     type Scheme = SecretToken;
///     type State = SecretToken;
///
///     const KEY: &'static str = "slack_bot_token";
///
///     fn metadata() -> CredentialMetadata {
///         CredentialMetadata::new(
///             credential_key!("slack_bot_token"),
///             "Slack Bot Token",
///             "Slack bot OAuth token",
///             ValidSchema::empty(),
///             AuthPattern::SecretToken,
///         )
///     }
///
///     fn project(state: &SecretToken) -> SecretToken { state.clone() }
///
///     async fn resolve(
///         values: &FieldValues,
///         _ctx: &CredentialContext,
///     ) -> Result<ResolveResult<SecretToken, ()>, CredentialError> {
///         let token = values.get_string_by_str("bot_token").unwrap_or_default();
///         Ok(ResolveResult::Complete(SecretToken::new(SecretString::new(token.to_owned()))))
///     }
/// }
///
/// assert_eq!(SlackBotToken::KEY, "slack_bot_token");
/// // `project` is a pure, synchronous extraction of auth material from state.
/// let state = SecretToken::new(SecretString::new("xoxb-123"));
/// let scheme = SlackBotToken::project(&state);
/// assert_eq!(scheme.token().expose_secret(), "xoxb-123");
/// ```
pub trait Credential: Send + Sync + 'static {
    /// Typed shape of the credential setup-form fields.
    ///
    /// Mirrors `Action::Input` / `Resource::Config` — the canonical
    /// schema-bearing companion struct. Per Phase 5 of the M6 redesign
    /// the schema lives on this type rather than being baked into
    /// [`CredentialMetadata`]: read it via
    /// [`nebula_schema::schema_of::<Self::Properties>()`](nebula_schema::schema_of)
    /// (there is no per-trait schema method — schema-of properties).
    ///
    /// Use [`FieldValues`] for legacy credentials that do not yet
    /// declare a typed properties struct (the blanket
    /// [`HasSchema`](nebula_schema::HasSchema) impl returns an empty
    /// schema).
    type Properties: nebula_schema::HasSchema + Send + Sync + 'static;

    /// What this credential produces — the consumer-facing auth material.
    type Scheme: AuthScheme;

    /// What gets stored — may include refresh internals not exposed to
    /// resources. For static credentials: same type as `Scheme`
    /// (use [`identity_state!`](crate::identity_state) macro).
    type State: CredentialState;

    /// Stable key for this credential type (e.g., `"github_oauth2"`).
    const KEY: &'static str;

    /// Integration-catalog metadata: key, name, icon, documentation
    /// URL, parameters.
    fn metadata() -> CredentialMetadata
    where
        Self: Sized;

    /// Project the runtime [`Scheme`] from stored [`State`]. Synchronous,
    /// pure. `where Self: Sized` excludes this from any object-safe
    /// vtable — dispatch goes through downcast at full type knowledge.
    ///
    /// [`Scheme`]: Credential::Scheme
    /// [`State`]: Credential::State
    fn project(state: &Self::State) -> Self::Scheme
    where
        Self: Sized;

    /// Build initial [`State`] from user [`Properties`] (carried as
    /// [`FieldValues`]). Returns `ResolveResult<State, ()>`.
    ///
    /// # Allowed return shapes
    ///
    /// - [`Complete(state)`](ResolveResult::Complete) — credential resolved synchronously (the
    ///   common case for non-interactive credentials such as API keys).
    /// - [`Retry { after }`](ResolveResult::Retry) — caller polls again after the delay (rare; some
    ///   long-running provider calls).
    ///
    /// # Forbidden: `Pending(())`
    ///
    /// The base resolve **must not** return
    /// [`Pending`](ResolveResult::Pending). Per Tech Spec §15.4 the
    /// degenerate `state: ()` carried here cannot deserialize into a
    /// credential's typed [`Interactive::Pending`] later in
    /// [`Interactive::continue_resolve`]. The framework executor
    /// (`nebula-engine` `execute_resolve`) rejects `Pending` from the
    /// base resolve with `ExecutorError::BaseResolvePending`.
    ///
    /// Interactive credentials kick off through credential-specific
    /// helpers (e.g. `OAuth2Credential::initiate_authorization_code`)
    /// that construct the typed `Self::Pending` directly and persist it
    /// via [`PendingStateStore::put`](crate::PendingStateStore::put);
    /// `execute_continue::<C: Interactive>` then loads that typed
    /// pending and threads it through
    /// [`Interactive::continue_resolve`].
    ///
    /// [`Interactive::Pending`]: crate::Interactive::Pending
    /// [`Interactive::continue_resolve`]: crate::Interactive::continue_resolve
    /// [`Properties`]: Credential::Properties
    /// [`State`]: Credential::State
    fn resolve(
        values: &FieldValues,
        ctx: &CredentialContext,
    ) -> impl Future<Output = Result<ResolveResult<Self::State, ()>, CredentialError>> + Send
    where
        Self: Sized;
}
