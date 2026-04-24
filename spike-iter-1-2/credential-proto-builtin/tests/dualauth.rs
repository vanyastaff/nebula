//! Q4 DualAuth end-to-end — MtlsHttpResource consumes both TlsIdentity +
//! Bearer credentials, projects both, "uses" them.

use credential_proto::{CredentialKey, CredentialRegistry};
use credential_proto_builtin::{
    BitbucketOAuth2, MtlsClientCredential, MtlsIdentityState, OAuth2State,
    resolve_mtls_pair,
};

#[test]
fn dualauth_resolves_both_slots() {
    let mut reg = CredentialRegistry::new();
    let tls_key = CredentialKey::new("tls");
    let bearer_key = CredentialKey::new("bearer");
    reg.insert(tls_key.clone(), MtlsClientCredential);
    reg.insert(bearer_key.clone(), BitbucketOAuth2);

    let tls_state = MtlsIdentityState {
        cert_pem: "-----BEGIN CERT-----".into(),
        key_pem: "-----BEGIN PRIVATE KEY-----".into(),
    };
    let bearer_state = OAuth2State {
        access_token: "oauth_tok".into(),
        refresh_token: "oauth_refresh".into(),
    };

    let pair =
        resolve_mtls_pair::<MtlsClientCredential, BitbucketOAuth2>(
            &reg,
            tls_key.as_str(),
            bearer_key.as_str(),
            &tls_state,
            &bearer_state,
        )
        .expect("both slots resolve");

    assert!(pair.0.cert_pem.starts_with("-----BEGIN CERT-----"));
    assert_eq!(pair.1.token, "oauth_tok");
}
