//! Q4 DualAuth + §3.5 (i) compile-enforcement proof.
//!
//! `resolve_mtls_pair::<C1, C2>` requires `C1::Scheme = TlsIdentityScheme`
//! and `C2::Scheme = BearerScheme`. Using `BitbucketAppPassword` (Scheme
//! = BasicScheme) as C2 must FAIL TO COMPILE.
//!
//! This proves that a Resource-declared `AcceptedAuth = (TlsIdentity,
//! Bearer)` tuple compile-enforces both slots — an action matching only
//! one auth slot, or matching both with wrong schemes, is rejected.
//!
//! Expected: E0271 mismatch on `<BitbucketAppPassword as Credential>::Scheme`.

use credential_proto_builtin::{
    AppPasswordState, BitbucketAppPassword, MtlsClientCredential, MtlsIdentityState,
    resolve_mtls_pair,
};
use credential_proto::{CredentialKey, CredentialRegistry};

fn main() {
    let reg = CredentialRegistry::new();
    let tls_key = CredentialKey::new("tls_slot");
    let bearer_key = CredentialKey::new("bearer_slot");

    let tls_state = MtlsIdentityState {
        cert_pem: "…".into(),
        key_pem: "…".into(),
    };
    let bad_bearer_state = AppPasswordState {
        user: "u".into(),
        pass: "p".into(),
    };

    // MUST FAIL: BitbucketAppPassword::Scheme = BasicScheme, not BearerScheme.
    let _ = resolve_mtls_pair::<MtlsClientCredential, BitbucketAppPassword>(
        &reg,
        tls_key.as_str(),
        bearer_key.as_str(),
        &tls_state,
        &bad_bearer_state,
    );
}
