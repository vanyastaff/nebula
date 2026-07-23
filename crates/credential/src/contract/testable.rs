//! `Testable` sub-trait — credentials with provider-side health probe.
//!
//! Per Tech Spec §15.4 capability sub-trait split — closes
//! security-lead findings N1 + N3 + N5. The pre-§15.4 shape declared
//! testability via `const TESTABLE: bool = false` plus a defaulted
//! [`test`] body that returned `Ok(None)`. A plugin author setting
//! `const TESTABLE = true` while forgetting to override `test` produced
//! a credential that *declared* testability but silently returned the
//! "type does not support testing" sentinel at runtime — UI showed "no
//! probe available" instead of the correct status. The sub-trait
//! variant in this module makes that mistake structurally impossible:
//! only credentials that explicitly `impl Testable` can route through
//! the engine's test dispatcher, `test` has no defaulted body
//! (`E0046` if omitted), and the return type is
//! `Result<TestResult, _>` — the `Option` carve-out from the const-bool
//! shape is removed because the type-level membership in `Testable`
//! already encodes "this credential supports testing."
//!
//! [`test`]: Testable::test

use std::future::Future;

use crate::{Credential, CredentialContext, error::CredentialError, resolve::TestResult};

/// Credentials that support a live health probe against the issuing
/// provider (OAuth2 introspect, AWS STS GetCallerIdentity, GitHub user
/// API).
///
/// Test dispatch binds `where C: Testable` — non-`Testable` credentials
/// cannot reach the test path. The signature returns
/// `Result<TestResult, CredentialError>` — there is no `Ok(None)`
/// "not testable" carve-out because membership in `Testable` already
/// guarantees the credential supports probing.
///
/// A provider adapter must classify a rejection with the payload-free,
/// extensible [`TestFailureCode`](crate::TestFailureCode) enum and discard raw
/// provider text before returning. Provider messages are untrusted and may
/// echo credentials; they must not cross this contract into logs or API
/// responses.
///
/// # Examples
///
/// ```
/// use nebula_credential::{
///     AuthPattern, Credential, CredentialContext, CredentialMetadata, Testable,
///     SecretString, scheme::SecretToken,
/// };
/// use nebula_credential::error::CredentialError;
/// use nebula_credential::resolve::{ResolveResult, TestResult};
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
/// impl Testable for OAuth2Cred {
///     async fn test(
///         scheme: &SecretToken,
///         _ctx: &CredentialContext,
///     ) -> Result<TestResult, CredentialError> {
///         // Probe a lightweight authenticated "whoami" endpoint with `scheme`.
///         let _ = scheme;
///         Ok(TestResult::Success)
///     }
/// }
///
/// // Test capability is encoded by trait membership — `where C: Testable`.
/// fn assert_testable<C: Testable>() {}
/// assert_testable::<OAuth2Cred>();
/// ```
pub trait Testable: Credential {
    /// Test that the credential actually works.
    ///
    /// Implementations should perform a lightweight authenticated call
    /// against the provider (token introspection, "whoami" endpoint)
    /// and return [`TestResult::Success`] on a 2xx response or
    /// [`TestResult::Failed`] with a payload-free,
    /// extensible [`TestFailureCode`](crate::TestFailureCode) when the provider
    /// definitively rejects the credential. Network, transport, and
    /// provider-internal errors that prevent determining validity surface as
    /// `Err(CredentialError)`; they are not negative probe outcomes.
    fn test(
        scheme: &Self::Scheme,
        ctx: &CredentialContext,
    ) -> impl Future<Output = Result<TestResult, CredentialError>> + Send
    where
        Self: Sized;
}
