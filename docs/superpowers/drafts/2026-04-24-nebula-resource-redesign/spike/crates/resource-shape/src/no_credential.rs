//! `NoCredential` — opt-out for resources without an authenticated binding.
//!
//! Replaces today's `type Auth = ()` pattern. Per Strategy §4.1 and ADR-0036,
//! `type Credential = NoCredential;` is the idiomatic spelling for resources
//! that don't bind to a credential at all (Redis without auth, in-process
//! caches, KV stores backed by config-only material, etc.).
//!
//! # Why a full-fat `Credential` impl?
//!
//! Because `Resource::Credential: Credential` is the trait bound, the opt-out
//! must itself implement `Credential`. The methods are unreachable in
//! practice (`Manager` only calls them on registered credentials, and
//! `NoCredential` resources skip the reverse-index write path entirely per
//! ADR-0036 §Decision), but the compiler has to be able to typecheck the
//! bound. So we ship a structurally-honest impl with `Scheme = NoScheme`
//! whose `pattern() == AuthPattern::NoAuth`.
//!
//! # Where this lives in production
//!
//! Strategy and ADR-0036 don't specify whether `NoCredential` lives in
//! `nebula-credential` or `nebula-resource`. Spike defines it locally in
//! `resource-shape` to keep the spike self-contained and to flag the
//! decision for Tech Spec CP1 (see `NOTES.md` open questions).

use nebula_credential::{
    AuthPattern, AuthScheme, Credential, CredentialContext, CredentialError, CredentialMetadata,
    CredentialState, NoPendingState, ResolveResult, credential_key,
};
use serde::{Deserialize, Serialize};

/// Zero-sized scheme that asserts "no authentication".
///
/// Pattern is [`AuthPattern::NoAuth`]; impl details mirror the existing
/// `()` impl on `AuthScheme` but as a real named type so it composes with
/// the `Credential` trait's `type Scheme: AuthScheme` bound.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct NoScheme;

impl AuthScheme for NoScheme {
    fn pattern() -> AuthPattern {
        AuthPattern::NoAuth
    }
}

impl CredentialState for NoScheme {
    const KIND: &'static str = "no_scheme";
    const VERSION: u32 = 1;
}

/// `Credential` opt-out marker.
///
/// `type Credential = NoCredential;` declares that a `Resource` does not
/// bind to a credential. The reshaped `Manager` (see [`crate::manager`])
/// special-cases this at the type level: registering an `R` whose
/// `Credential = NoCredential` does not populate the credential reverse
/// index, and `on_credential_refreshed` / `on_credential_revoked` will
/// never reach this resource.
pub struct NoCredential;

impl Credential for NoCredential {
    type Input = ();
    type Scheme = NoScheme;
    type State = NoScheme;
    type Pending = NoPendingState;

    const KEY: &'static str = "no_credential";

    fn metadata() -> CredentialMetadata
    where
        Self: Sized,
    {
        CredentialMetadata::builder()
            .key(credential_key!("no_credential"))
            .name("No credential")
            .description("Opt-out marker for resources without an authenticated binding.")
            .schema(<() as nebula_schema::HasSchema>::schema())
            .pattern(AuthPattern::NoAuth)
            .build()
            .expect("static metadata fields are all set above")
    }

    fn project(_state: &NoScheme) -> NoScheme {
        NoScheme
    }

    async fn resolve(
        _values: &nebula_schema::FieldValues,
        _ctx: &CredentialContext,
    ) -> Result<ResolveResult<NoScheme, NoPendingState>, CredentialError> {
        // Returning `Complete(NoScheme)` keeps the trait obligations honest;
        // in practice the dispatcher never reaches this method for a
        // `NoCredential`-typed resource (see `Manager::register`).
        Ok(ResolveResult::Complete(NoScheme))
    }
}
