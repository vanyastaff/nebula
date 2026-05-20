//! Pins the public surface of `nebula-credential`. Failures here mean
//! either a public symbol was added without intent or one was removed
//! and we need to update this list.

#[allow(unused_imports)]
use nebula_credential::{
    AuthScheme, CredentialContext, CredentialError, CredentialId, CredentialMetadata,
    CredentialRecord, CredentialRegistry, CredentialSnapshot, CredentialState, CredentialStore,
    Dynamic, Interactive, PendingStateStore, Refreshable, Revocable, ScopeResolver, SecretString,
    Testable,
};

#[test]
fn public_contract_surface_stable() {
    // Existence-only check — compiles iff every named symbol is `pub` at root.
    let _ = std::any::TypeId::of::<CredentialError>();
}
