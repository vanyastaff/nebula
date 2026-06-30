//! `Revocable` sub-trait — credentials with provider-side revocation.
//!
//! Per Tech Spec §15.4 capability sub-trait split — closes
//! security-lead findings N1 + N3 + N5. The pre-§15.4 shape declared
//! revocation capability via `const REVOCABLE: bool = false` plus a
//! defaulted [`revoke`] body that returned `Ok(())` (no-op success). A
//! plugin author setting `const REVOCABLE = true` while forgetting to
//! override `revoke` produced a credential that *declared* revocation
//! capability but silently no-op'd at runtime — the engine treated
//! revocation as successful, the secret stayed live at the provider.
//! The sub-trait variant in this module makes that mistake structurally
//! impossible: only credentials that explicitly `impl Revocable` can
//! route through the engine's revoke dispatcher, and `revoke` has no
//! defaulted body (`E0046` if omitted).
//!
//! [`revoke`]: Revocable::revoke

use std::future::Future;

use crate::{Credential, CredentialContext, error::CredentialError};

/// Credentials that support explicit revocation at the issuing
/// provider (OAuth2 token revoke endpoint, AWS IAM access-key
/// deactivation, GitHub PAT revocation).
///
/// Revoke dispatch binds `where C: Revocable` — non-`Revocable`
/// credentials cannot reach the revoke path. The engine surfaces
/// revocation failures explicitly rather than silently no-op'ing them.
///
/// # Examples
///
/// ```
/// use nebula_credential::{
///     AuthPattern, Credential, CredentialContext, CredentialMetadata, Revocable,
///     SecretString, scheme::SecretToken,
/// };
/// use nebula_credential::error::CredentialError;
/// use nebula_credential::resolve::ResolveResult;
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
/// impl Revocable for OAuth2Cred {
///     async fn revoke(
///         state: &mut SecretToken,
///         _ctx: &CredentialContext,
///     ) -> Result<(), CredentialError> {
///         // POST to the provider's token revocation endpoint (RFC 7009), then
///         // zero the stored token so subsequent resolves see it as revoked.
///         *state = SecretToken::new(SecretString::new(""));
///         Ok(())
///     }
/// }
///
/// // Revoke capability is encoded by trait membership — `where C: Revocable`.
/// fn assert_revocable<C: Revocable>() {}
/// assert_revocable::<OAuth2Cred>();
/// ```
pub trait Revocable: Credential {
    /// Revoke this credential at the provider.
    ///
    /// Implementations should issue the provider-side revocation call
    /// (OAuth2 RFC 7009, IAM key deactivation, etc.) and mutate `state`
    /// to reflect the revoked status — typically zeroing the access
    /// token and clearing any refresh token. The framework persists the
    /// resulting state so subsequent resolves see the credential as
    /// revoked rather than stale.
    fn revoke(
        state: &mut Self::State,
        ctx: &CredentialContext,
    ) -> impl Future<Output = Result<(), CredentialError>> + Send
    where
        Self: Sized;
}
