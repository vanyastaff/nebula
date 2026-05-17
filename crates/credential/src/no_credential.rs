//! `NoCredential` — legacy no-auth credential marker.
//!
//! Per ADR-0044 (supersedes ADR-0036), the `Resource::Credential`
//! associated type was removed in favor of typed `#[credential(...)]`
//! slot fields on resource structs. Resources that do not bind any
//! credential simply declare zero `#[credential]` fields — `NoCredential`
//! is no longer used as an opt-out marker on the resource side and is
//! no longer re-exported from `nebula-resource`.
//!
//! The type itself is retained inside `nebula-credential` because its
//! `Scheme = ()` projection (in `nebula_core::auth`) is referenced by
//! credential-subsystem internals (capability sub-traits, registry
//! diagnostics, etc.). It can be removed when those internal callers
//! migrate.

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

/// Legacy no-auth credential type — retained for credential-subsystem
/// internal use (registry diagnostics, capability sub-traits).
///
/// Per ADR-0044 the `Resource::Credential` associated type was removed;
/// `NoCredential` is no longer used as an opt-out marker. Resources that
/// don't need credential material simply declare zero `#[credential]`
/// slot fields.
///
/// # Not registered with `CredentialRegistry`
///
/// `NoCredential` never enters
/// [`CredentialRegistry`](crate::CredentialRegistry) (no UI catalog entry, no
/// `register()` call, no capability-report impls). The five `IsInteractive` /
/// `IsRefreshable` / `IsRevocable` / `IsTestable` / `IsDynamic` impls present
/// on registered built-ins (see [`ApiKeyCredential`](crate::ApiKeyCredential))
/// are intentionally absent here.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoCredential;

impl Credential for NoCredential {
    type Properties = ();
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
            .schema(nebula_schema::schema_of::<Self::Properties>())
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
