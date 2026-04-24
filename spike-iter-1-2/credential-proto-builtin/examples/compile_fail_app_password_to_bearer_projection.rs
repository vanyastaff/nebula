//! §3.5 mechanism (i) validation — where-clause cross-check.
//!
//! `resolve_as_bearer::<C>` requires `C: Credential<Scheme = BearerScheme>`.
//! `BitbucketAppPassword` has `Scheme = BasicScheme`. Therefore calling
//! `resolve_as_bearer::<BitbucketAppPassword>` must FAIL TO COMPILE.
//!
//! This is the downstream manifestation of §3.3 semantic rejection at the
//! resolve layer: the engine cannot even TRY to project AppPassword as
//! Bearer because the generic bound rules it out.
//!
//! Expected: E0271 (type equality mismatch) or E0277 on the Scheme=Bearer
//! projection bound.

use credential_proto::{CredentialKey, CredentialRegistry};
use credential_proto_builtin::{AppPasswordState, BearerScheme, BitbucketAppPassword};

// Redeclare the resolve_as_bearer shape in-example (the real one lives in
// tests/, not lib/). Same where-clause.
fn resolve_as_bearer<C>(
    _reg: &CredentialRegistry,
    _key: &str,
    _state: &C::State,
) -> Option<BearerScheme>
where
    C: credential_proto::Credential<Scheme = BearerScheme>,
{
    None
}

fn main() {
    let reg = CredentialRegistry::new();
    let key = CredentialKey::new("forge");
    let state = AppPasswordState { user: "u".into(), pass: "p".into() };

    // MUST FAIL: BitbucketAppPassword::Scheme = BasicScheme ≠ BearerScheme.
    let _ = resolve_as_bearer::<BitbucketAppPassword>(&reg, key.as_str(), &state);
}
