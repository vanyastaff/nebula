//! Unified credential trait (CP6).
//!
//! Per Tech Spec §15.4 capability sub-trait split — the previous shape
//! (CP4) carried five `const X: bool = false` capability flags plus
//! defaulted method bodies for `continue_resolve` / `refresh` / `revoke`
//! / `test` / `release`. A plugin author setting `const REFRESHABLE =
//! true` while forgetting to override `refresh()` produced a credential
//! that *declared* refresh capability but silently returned
//! `RefreshOutcome::NotSupported` — the engine treated this as a benign
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

use nebula_schema::{FieldValues, ValidSchema};

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
/// out of this trait; see ADR-0033 (`docs/adr/0033-integration-credentials-plane-b.md`).
///
/// # Associated types
///
/// - **`Input`** — typed shape of the setup-form fields ([`HasSchema`](nebula_schema::HasSchema)).
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
/// ```ignore
/// use nebula_credential::{
///     Credential, identity_state,
///     scheme::SecretToken,
///     resolve::ResolveResult,
/// };
///
/// struct SlackBotToken;
///
/// identity_state!(SecretToken, "secret_token", 1);
///
/// impl Credential for SlackBotToken {
///     type Input = SlackBotInput;
///     type Scheme = SecretToken;
///     type State = SecretToken;
///
///     const KEY: &'static str = "slack_bot_token";
///
///     fn metadata() -> CredentialMetadata { /* ... */ }
///     fn project(state: &SecretToken) -> SecretToken { state.clone() }
///
///     async fn resolve(
///         values: &FieldValues,
///         _ctx: &CredentialContext<'_>,
///     ) -> Result<ResolveResult<SecretToken, ()>, CredentialError> {
///         let token = values.get_string_by_str("bot_token").unwrap_or_default();
///         Ok(ResolveResult::Complete(SecretToken::new(SecretString::new(token.to_owned()))))
///     }
/// }
/// ```
pub trait Credential: Send + Sync + 'static {
    /// Typed shape of the setup-form fields.
    ///
    /// The canonical [`schema()`](Credential::schema) is
    /// auto-derived via `<Self::Input as HasSchema>::schema()`. Use
    /// [`FieldValues`] for legacy credentials that do not yet declare a
    /// typed input (the blanket [`HasSchema`](nebula_schema::HasSchema)
    /// impl returns an empty schema).
    type Input: nebula_schema::HasSchema + Send + Sync + 'static;

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

    /// Returns the schema for credential input parameters.
    ///
    /// The default implementation derives the schema from
    /// [`Self::Input`], which must implement
    /// [`HasSchema`](nebula_schema::HasSchema). Override only if the
    /// form layout must differ from the `Input` struct (rare).
    fn schema() -> ValidSchema
    where
        Self: Sized,
    {
        <Self::Input as nebula_schema::HasSchema>::schema()
    }

    /// Project the runtime [`Scheme`] from stored [`State`]. Synchronous,
    /// pure. `where Self: Sized` excludes this from any object-safe
    /// vtable — dispatch goes through downcast at full type knowledge.
    ///
    /// [`Scheme`]: Credential::Scheme
    /// [`State`]: Credential::State
    fn project(state: &Self::State) -> Self::Scheme
    where
        Self: Sized;

    /// Build initial [`State`] from user [`Input`]. Returns
    /// `ResolveResult<State, ()>` — interactive credentials carry typed
    /// pending state on [`Interactive::continue_resolve`] rather than
    /// here; non-interactive credentials always return
    /// [`Complete`](ResolveResult::Complete).
    ///
    /// **Framework handles `PendingState` storage.** When an
    /// implementation returns
    /// [`Pending`](ResolveResult::Pending) — typically only the kickoff
    /// for an interactive flow — the framework encrypts, stores, and
    /// generates a [`PendingToken`](crate::PendingToken). Credential
    /// authors never call `store_pending()` or `consume_pending()`.
    ///
    /// [`Interactive::continue_resolve`]: crate::Interactive::continue_resolve
    /// [`Input`]: Credential::Input
    /// [`State`]: Credential::State
    fn resolve(
        values: &FieldValues,
        ctx: &CredentialContext,
    ) -> impl Future<Output = Result<ResolveResult<Self::State, ()>, CredentialError>> + Send
    where
        Self: Sized;
}
