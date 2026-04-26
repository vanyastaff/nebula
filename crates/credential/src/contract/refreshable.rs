//! `Refreshable` sub-trait — credentials with refreshable [`State`].
//!
//! Per Tech Spec §15.4 capability sub-trait split — closes
//! security-lead findings N1 + N3 + N5. The pre-§15.4 shape declared
//! refresh capability via `const REFRESHABLE: bool = false` plus a
//! defaulted [`refresh`] body that returned `Ok(RefreshOutcome::NotSupported)`.
//! A plugin author setting `const REFRESHABLE = true` while forgetting
//! to override `refresh` produced a credential that *declared* refresh
//! capability but silently never refreshed — the engine read
//! `NotSupported` as a benign outcome, no error class fired, the
//! credential eventually expired in production with no alert. The
//! sub-trait variant in this module makes that mistake structurally
//! impossible: only credentials that explicitly `impl Refreshable` can
//! route through the engine's refresh dispatcher, and `refresh` has no
//! defaulted body (`E0046` if omitted).
//!
//! Engine [`RefreshDispatcher::for_credential<C>`] binds
//! `where C: Refreshable`. A non-`Refreshable` credential cannot be
//! passed — `E0277` at the dispatch site. Probe 4
//! (`compile_fail_engine_dispatch_capability`) cements this guarantee.
//!
//! [`State`]: crate::CredentialState
//! [`refresh`]: Refreshable::refresh
//! [`RefreshDispatcher::for_credential<C>`]: nebula_engine::credential::rotation::RefreshDispatcher

use std::future::Future;

use crate::{
    Credential, CredentialContext,
    error::CredentialError,
    resolve::{RefreshOutcome, RefreshPolicy},
};

/// Credentials that support refreshing their stored [`State`] without
/// requiring full re-authentication (OAuth2 refresh token, dynamic AWS
/// session token rotation, expiring API keys with rotation hook).
///
/// Refresh dispatch goes through the engine's
/// `RefreshDispatcher::for_credential::<C>()` which binds
/// `where C: Refreshable`. Non-`Refreshable` credentials cannot reach
/// the refresh path — the silent-downgrade vector from the const-bool
/// shape is structurally absent.
///
/// # Examples
///
/// ```ignore
/// use nebula_credential::{Credential, Refreshable};
/// use nebula_credential::resolve::{RefreshOutcome, RefreshPolicy};
///
/// struct OAuth2Cred;
///
/// // (impl Credential for OAuth2Cred elided)
///
/// impl Refreshable for OAuth2Cred {
///     const REFRESH_POLICY: RefreshPolicy = RefreshPolicy::DEFAULT;
///
///     async fn refresh(
///         state: &mut OAuth2State,
///         ctx: &CredentialContext<'_>,
///     ) -> Result<RefreshOutcome, CredentialError> {
///         // ... exchange refresh token for new access token ...
///     }
/// }
/// ```
///
/// [`State`]: crate::CredentialState
pub trait Refreshable: Credential {
    /// Refresh timing policy — controls early refresh, retry backoff,
    /// and jitter. Default: [`RefreshPolicy::DEFAULT`] (5 min early
    /// refresh, 5 s minimum retry backoff, 30 s jitter window).
    const REFRESH_POLICY: RefreshPolicy = RefreshPolicy::DEFAULT;

    /// Refresh the credential's stored state.
    ///
    /// The engine drives this method when the credential enters its
    /// early-refresh window or when downstream consumers detect a
    /// credential failure indicating expiry. Implementations should
    /// mutate `state` in place (e.g., replace the access token while
    /// preserving the refresh token) and return [`RefreshOutcome`].
    ///
    /// Return [`RefreshOutcome::ReauthRequired`] when the refresh path
    /// fails irrecoverably (refresh token revoked, scope changed) — the
    /// engine surfaces this as an explicit re-auth signal rather than
    /// silently swallowing the failure.
    fn refresh(
        state: &mut Self::State,
        ctx: &CredentialContext,
    ) -> impl Future<Output = Result<RefreshOutcome, CredentialError>> + Send
    where
        Self: Sized;
}
