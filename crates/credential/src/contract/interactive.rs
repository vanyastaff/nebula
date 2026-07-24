//! `Interactive` sub-trait — credentials with multi-step resolve flows.
//!
//! Per Tech Spec §15.4 capability sub-trait split — closes
//! security-lead findings N1 + N3 + N5 by eliminating the
//! silent-downgrade vector at the type level.
//!
//! In the pre-§15.4 shape `Credential` carried a `const INTERACTIVE: bool
//! = false` flag plus a defaulted [`continue_resolve`] body that returned
//! `Err(CredentialError::NotInteractive)`. A plugin author setting
//! `const INTERACTIVE = true` while forgetting to override
//! `continue_resolve` produced a credential that *declared* interactive
//! capability but silently rejected callbacks at runtime — the engine
//! treated the failure as an ordinary error, no diagnostic surfaced the
//! authoring mistake. The sub-trait variant in this module makes the
//! mistake structurally impossible: only credentials that explicitly
//! `impl Interactive` can route through interactive dispatch, and the
//! [`Interactive::Pending`] associated type plus [`Interactive::continue_resolve`] are
//! both required (no defaulted bodies).
//!
//! The `Pending` associated type lives here, *not* on the base
//! [`Credential`] trait — non-interactive credentials need no `Pending`
//! companion type. The base [`Credential::resolve`] therefore returns
//! `ResolveResult<Self::State, ()>`; interactive credentials
//! continue through [`Interactive::continue_resolve`] returning
//! `ResolveResult<Self::State, Self::Pending>` with the typed pending
//! state.
//!
//! [`continue_resolve`]: Interactive::continue_resolve
//! [`Credential`]: crate::Credential
//! [`Credential::resolve`]: crate::Credential::resolve

use std::future::Future;

use crate::{
    Credential, CredentialContext, PendingState,
    error::CredentialError,
    resolve::{ResolveResult, UserInput},
};

/// Credentials that require multi-step interactive resolution
/// (OAuth2 authorize→callback, device code flow, multi-step chain).
///
/// Static credentials (API keys, basic auth) do **not** implement this
/// trait. The base [`Credential::resolve`] returns
/// `ResolveResult<Self::State, ()>` — interactive variants go through
/// [`Interactive::continue_resolve`] with a typed [`Self::Pending`]
/// companion. The framework persists the typed `Pending` via
/// [`PendingStateStore`](crate::pending_store::PendingStateStore)
/// (encrypted, TTL-bounded, single-use) and surfaces it on subsequent
/// continuation calls.
///
/// # Examples
///
/// ```
/// use nebula_credential::{
///     AuthPattern, Credential, CredentialContext, CredentialMetadata, Interactive,
///     OAuth2Pending, SecretString, scheme::SecretToken,
/// };
/// use nebula_credential::error::CredentialError;
/// use nebula_credential::resolve::{ResolveResult, UserInput};
/// use nebula_core::credential_key;
/// use nebula_schema::{FieldValues, ValidSchema};
///
/// struct OAuth2Cred;
///
/// # impl Credential for OAuth2Cred {
/// #     type Properties = FieldValues;
/// #     type Scheme = SecretToken;
/// #     type State = SecretToken;
/// #     const KEY: &'static str = "oauth2_cred";
/// #     fn metadata() -> CredentialMetadata {
/// #         CredentialMetadata::new(
/// #             credential_key!("oauth2_cred"), "OAuth2", "demo",
/// #             ValidSchema::empty(), AuthPattern::SecretToken,
/// #         )
/// #     }
/// #     fn project(state: &SecretToken) -> SecretToken { state.clone() }
/// #     async fn resolve(
/// #         _values: &FieldValues,
/// #         _ctx: &CredentialContext,
/// #     ) -> Result<ResolveResult<SecretToken, ()>, CredentialError> {
/// #         Ok(ResolveResult::Complete(SecretToken::new(SecretString::new(""))))
/// #     }
/// # }
/// impl Interactive for OAuth2Cred {
///     // Typed pending state retained with TTL/single-use semantics between
///     // `resolve()` and `continue_resolve()`. The current first-party adapter
///     // is ephemeral memory; a durable adapter must encrypt at rest.
///     type Pending = OAuth2Pending;
///
///     async fn continue_resolve(
///         pending: &OAuth2Pending,
///         input: &UserInput,
///         _ctx: &CredentialContext,
///     ) -> Result<ResolveResult<SecretToken, OAuth2Pending>, CredentialError> {
///         // Validate the callback against the stored anti-CSRF state, then
///         // exchange the authorization code for an access token.
///         let _ = (pending, input);
///         Ok(ResolveResult::Complete(SecretToken::new(SecretString::new("access-token"))))
///     }
/// }
///
/// // Interactive capability is encoded by trait membership.
/// fn assert_interactive<C: Interactive>() {}
/// assert_interactive::<OAuth2Cred>();
/// ```
///
/// [`Credential::resolve`]: crate::Credential::resolve
pub trait Interactive: Credential {
    /// Typed pending state for interactive flows.
    ///
    /// Held in encrypted storage between `resolve()` and
    /// `continue_resolve()`; carries flow-specific data (PKCE verifier,
    /// device code, anti-CSRF state) that must not leak into URLs or
    /// callback parameters.
    type Pending: PendingState;

    /// Continue interactive resolve after the user completes
    /// interaction.
    ///
    /// The framework loads and consumes the typed
    /// [`PendingState`] before calling this method —
    /// credential authors never call `store_pending()` or
    /// `consume_pending()` directly.
    ///
    /// Returns `Complete(state)` when the flow finishes, `Pending { state,
    /// interaction }` to continue with another interactive step (typed
    /// pending state is re-stored), or `Retry { after }` to ask the
    /// framework to poll again after the given delay (device code flow).
    fn continue_resolve(
        pending: &Self::Pending,
        input: &UserInput,
        ctx: &CredentialContext,
    ) -> impl Future<Output = Result<ResolveResult<Self::State, Self::Pending>, CredentialError>> + Send
    where
        Self: Sized;
}
