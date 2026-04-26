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
/// # Examples
///
/// ```ignore
/// use nebula_credential::{Credential, Testable};
/// use nebula_credential::resolve::TestResult;
///
/// struct OAuth2Cred;
///
/// // (impl Credential for OAuth2Cred elided)
///
/// impl Testable for OAuth2Cred {
///     async fn test(
///         scheme: &Self::Scheme,
///         ctx: &CredentialContext,
///     ) -> Result<TestResult, CredentialError> {
///         // ... probe provider health endpoint with this scheme ...
///     }
/// }
/// ```
pub trait Testable: Credential {
    /// Test that the credential actually works.
    ///
    /// Implementations should perform a lightweight authenticated call
    /// against the provider (token introspection, "whoami" endpoint)
    /// and return [`TestResult::Success`] on a 2xx response or
    /// [`TestResult::Failed { reason }`](TestResult::Failed) when the
    /// provider rejects the credential. Network or provider-internal
    /// errors that prevent determining validity surface as
    /// `Err(CredentialError)`.
    fn test(
        scheme: &Self::Scheme,
        ctx: &CredentialContext,
    ) -> impl Future<Output = Result<TestResult, CredentialError>> + Send
    where
        Self: Sized;
}
