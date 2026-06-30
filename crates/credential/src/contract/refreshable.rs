//! `Refreshable` sub-trait — credentials with refreshable [`State`].
//!
//! Per Tech Spec §15.4 capability sub-trait split — closes
//! security-lead findings N1 + N3 + N5. The pre-§15.4 shape declared
//! refresh capability via `const REFRESHABLE: bool = false` plus a
//! defaulted [`refresh`] body that returned a "not supported" sentinel.
//! A plugin author setting `const REFRESHABLE = true` while forgetting
//! to override `refresh` produced a credential that *declared* refresh
//! capability but silently never refreshed — the engine read the
//! sentinel as a benign outcome, no error class fired, the credential
//! eventually expired in production with no alert. The sub-trait
//! variant in this module makes that mistake structurally impossible:
//! only credentials that explicitly `impl Refreshable` can route
//! through the engine's refresh dispatcher, and `refresh` has no
//! defaulted body (`E0046` if omitted). The runtime back-channel
//! variant has been removed from [`RefreshOutcome`]
//! to seal the silent-downgrade vector at the type level.
//!
//! Engine `RefreshDispatcher::for_credential<C>` binds
//! `where C: Refreshable`. A non-`Refreshable` credential cannot be
//! passed — `E0277` at the dispatch site. Probe 4
//! (`compile_fail_engine_dispatch_capability`) cements this guarantee.
//!
//! [`State`]: crate::CredentialState
//! [`refresh`]: Refreshable::refresh

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
/// ```
/// use nebula_credential::{
///     AuthPattern, Credential, CredentialContext, CredentialMetadata, Refreshable,
///     SecretString, scheme::SecretToken,
/// };
/// use nebula_credential::error::CredentialError;
/// use nebula_credential::resolve::{RefreshOutcome, RefreshPolicy, ResolveResult};
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
/// impl Refreshable for OAuth2Cred {
///     const REFRESH_POLICY: RefreshPolicy = RefreshPolicy::DEFAULT;
///
///     async fn refresh(
///         state: &mut SecretToken,
///         _ctx: &CredentialContext,
///     ) -> Result<RefreshOutcome, CredentialError> {
///         // Exchange the refresh token for a new access token, mutating in place.
///         *state = SecretToken::new(SecretString::new("new-access-token"));
///         Ok(RefreshOutcome::Refreshed)
///     }
/// }
///
/// // Refresh capability is encoded by trait membership — `where C: Refreshable`.
/// fn assert_refreshable<C: Refreshable>() {}
/// assert_refreshable::<OAuth2Cred>();
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
