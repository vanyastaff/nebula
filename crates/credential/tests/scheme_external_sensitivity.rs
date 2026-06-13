//! Positive coverage for the `#[auth_scheme(external)]` derive arm (§15.5).
//!
//! An external scheme holds only an opaque handle to an out-of-process signer
//! (HSM / KMS / FIDO) — no in-process secret bytes, so **no `ZeroizeOnDrop` is
//! required** (unlike `sensitive`). The derive must emit `impl ExternalScheme`
//! plus the base `AuthScheme` impl, and the field audit must accept a plain
//! handle field (here `key_id: String`) while still rejecting `SecretString`
//! (covered by the `scheme_sensitivity_external_with_secret` compile-fail probe).

use nebula_credential::{AuthPattern, AuthScheme, ExternalScheme, scheme::SigningKeyFamily};

#[derive(AuthScheme)]
#[auth_scheme(pattern = RequestSigning, family = SigningKeyFamily, external)]
struct KmsSigningKey {
    // An opaque handle (key id / ARN), not a secret — accepted as plain `String`
    // even though it is named like a key (the secret-name lint is `sensitive`-only;
    // external rejects only `SecretString`/`SecretBytes`). No `ZeroizeOnDrop`.
    key_id: String,
}

fn requires_external<T: ExternalScheme>() {}
fn requires_scheme<T: AuthScheme>() {}

#[test]
fn external_derive_emits_external_scheme_without_zeroize() {
    // Trait membership: the derive emitted both impls.
    requires_external::<KmsSigningKey>();
    requires_scheme::<KmsSigningKey>();

    assert_eq!(
        <KmsSigningKey as AuthScheme>::pattern(),
        AuthPattern::RequestSigning,
    );

    // Constructing and dropping the type requires no `ZeroizeOnDrop` bound —
    // an external scheme holds no secret bytes to wipe.
    let key = KmsSigningKey {
        key_id: "kms://key-2026".to_owned(),
    };
    assert_eq!(key.key_id, "kms://key-2026");
}
