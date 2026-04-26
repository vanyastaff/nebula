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
//! [`Self::Pending`] associated type plus [`Self::continue_resolve`] are
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
/// ```ignore
/// use nebula_credential::{Credential, Interactive};
///
/// struct OAuth2Cred;
///
/// // (impl Credential for OAuth2Cred elided)
///
/// impl Interactive for OAuth2Cred {
///     type Pending = OAuth2Pending;
///
///     async fn continue_resolve(
///         pending: &OAuth2Pending,
///         input: &UserInput,
///         ctx: &CredentialContext<'_>,
///     ) -> Result<ResolveResult<OAuth2State, OAuth2Pending>, CredentialError> {
///         // ... validate callback, exchange code for token ...
///     }
/// }
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
    /// [`PendingState`](crate::PendingState) before calling this method —
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
