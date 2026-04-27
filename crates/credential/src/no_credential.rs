//! `NoCredential` — idiomatic opt-out for resources without an authenticated binding.
//!
//! Per ADR-0036, `Resource` impls that don't need credential material write
//! `type Credential = NoCredential;`. The associated `Scheme = ()` already
//! implements `AuthScheme` (with `pattern() = AuthPattern::NoAuth`) and
//! `PublicScheme` in `nebula_core::auth`, so no secret material flows.
//!
//! This is the credential-side mirror of the previous `type Auth = ();` pattern
//! retired in the П1 trait reshape.

use nebula_schema::FieldValues;
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::{
    AuthPattern, Credential, CredentialContext, CredentialError, CredentialMetadata,
    CredentialState, ResolveResult,
};

/// State for [`NoCredential`]. Carries no data — it is the type-level marker
/// the credential subsystem hands resources that don't bind any auth material.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct NoCredentialState;

impl Zeroize for NoCredentialState {
    fn zeroize(&mut self) {
        // No sensitive data to zeroize — this is the no-auth marker.
    }
}

impl ZeroizeOnDrop for NoCredentialState {}

impl CredentialState for NoCredentialState {
    const KIND: &'static str = "no_credential";
    const VERSION: u32 = 1;
}

/// Opt-out [`Credential`] for resources without an authenticated binding.
///
/// Replaces the legacy `type Auth = ();` pattern from before the П1 trait
/// reshape. Use as `type Credential = NoCredential;` on any `Resource` impl
/// that does not need credential material in `create()`.
///
/// # Not registered with `CredentialRegistry`
///
/// `NoCredential` is a type-level marker for `Resource` impls that don't bind
/// authenticated material — it never enters
/// [`CredentialRegistry`](crate::CredentialRegistry) (no UI catalog entry, no
/// `register()` call, no capability-report impls). The five `IsInteractive` /
/// `IsRefreshable` / `IsRevocable` / `IsTestable` / `IsDynamic` impls present
/// on registered built-ins (see [`ApiKeyCredential`](crate::ApiKeyCredential))
/// are intentionally absent here.
///
/// # Examples
///
/// <!-- TODO(П1 Task 9): un-ignore once nebula-resource trait reshape lands -->
/// ```ignore
/// use nebula_credential::NoCredential;
/// use nebula_resource::Resource;
///
/// struct PingResource;
///
/// impl Resource for PingResource {
///     type Config = ();
///     type Runtime = ();
///     type Lease = ();
///     type Error = std::io::Error;
///     type Credential = NoCredential;
///     // create() receives `&()` as `scheme` — no secrets.
/// }
/// ```
#[derive(Clone, Copy, Debug, Default)]
pub struct NoCredential;

impl Credential for NoCredential {
    type Input = ();
    /// `()` — already implements `AuthScheme` with `AuthPattern::NoAuth` and
    /// `PublicScheme` in `nebula_core::auth`.
    type Scheme = ();
    type State = NoCredentialState;

    const KEY: &'static str = "no_credential";

    fn metadata() -> CredentialMetadata {
        CredentialMetadata::builder()
            .key(nebula_core::credential_key!("no_credential"))
            .name("No credential")
            .description("Opt-out marker for resources without an authenticated binding.")
            .schema(Self::schema())
            .pattern(AuthPattern::NoAuth)
            .build()
            .expect("NoCredential metadata is statically valid")
    }

    fn project(_state: &Self::State) {}

    async fn resolve(
        _values: &FieldValues,
        _ctx: &CredentialContext,
    ) -> Result<ResolveResult<Self::State, ()>, CredentialError> {
        Ok(ResolveResult::Complete(NoCredentialState))
    }
}
