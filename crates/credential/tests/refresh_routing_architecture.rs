//! Architecture ratchet for type-directed credential refresh.
//!
//! Refresh dispatch must remain generic over `Refreshable`: credential keys
//! are registry identity, not a runtime type discriminator, and generic state
//! must never be reinterpreted through a cleartext `serde_json::Value`.

#[test]
fn resolver_has_no_key_routing_or_value_round_trip() {
    let source = include_str!("../src/runtime/resolver.rs");
    let production = source
        .split_once("\n#[cfg(test)]\nmod refresh_revoke_race")
        .map_or(source, |(production, _tests)| production);

    assert!(
        !production.contains("C::KEY"),
        "resolver refresh must dispatch through the Refreshable type, not C::KEY"
    );
    assert!(
        !production.contains("OAuth2Credential"),
        "generic resolver must not name a concrete credential implementation"
    );
    assert!(
        !production.contains("OAuth2State"),
        "generic resolver must not reinterpret state as a concrete credential state"
    );
    assert!(
        !production.contains("serde_json::to_value"),
        "generic resolver must not create a cleartext Value round-trip"
    );
    assert!(
        !production.contains("serde_json::from_value"),
        "generic resolver must not decode a cleartext Value round-trip"
    );
    assert!(
        !production.contains("timeout("),
        "resolver must not cancel the owned provider/persistence critical section"
    );
}

#[test]
fn refresh_transport_is_not_a_public_context_capability() {
    let source = include_str!("../src/context.rs");

    assert!(
        !source.contains("pub fn refresh_transport("),
        "CredentialContext must not expose the runtime transport publicly"
    );
    assert!(
        !source.contains("pub fn with_refresh_transport("),
        "public callers must not be able to stamp a runtime transport"
    );
    assert!(
        !source.contains("pub fn refresh_transport(mut self"),
        "CredentialContextBuilder must not gain a public transport method"
    );
}
