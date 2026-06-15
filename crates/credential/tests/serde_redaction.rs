//! Serde-redaction test for `nebula-credential` — sibling to `redaction.rs`.
//!
//! `redaction.rs` covers the `Debug` / `tracing` leak path. THIS file covers
//! the **serde** path, which `redaction.rs` does not touch.
//!
//! # The gap this pins
//!
//! Every `Sensitive` scheme (`SecretToken`, `Certificate`, `OAuth2Token`,
//! `SigningKey`, `SharedKey`, …) routes its secret fields through
//! `#[serde(with = "crate::serde_secret")]` and `#[derive(Serialize)]`. The
//! `serde_secret` serializer cannot inspect its sink, so a plain
//! `serde_json::to_string(&scheme)` driven by a telemetry / logging /
//! API-response serializer is indistinguishable from the encrypted-at-rest
//! storage path.
//!
//! Contract under test: **serializing a `Sensitive` scheme to a default
//! (non-storage) sink redacts every secret field**; cleartext is emitted only
//! inside an explicit `serde_secret::expose_for_serialization` scope (covered
//! by the storage-contract half below).
//!
//! Because the gate lives at the field-level `serde_secret` helper, it covers
//! any scheme tier (`Sensitive` *and* `External`) by construction — the tier
//! does not matter, only the `#[serde(with = "serde_secret")]` attribute does.

use nebula_credential::{
    SecretString,
    scheme::{Certificate, OAuth2Token, SecretToken, SharedKey, SigningKey},
    serde_secret,
};

/// The sentinel the default secret serializer must emit in place of plaintext.
const SENTINEL: &str = "[REDACTED]";

// ---------------------------------------------------------------------
// Default sink: secrets must redact
// ---------------------------------------------------------------------

#[test]
fn secret_token_does_not_serialize_plaintext_to_default_sink() {
    let raw = "sk-serde-canary-secret-token-never-emitted";
    let token = SecretToken::new(SecretString::new(raw));

    let json = serde_json::to_string(&token).expect("SecretToken must serialize");

    assert!(
        !json.contains(raw),
        "secret-token plaintext leaked to a default serde sink:\n{json}"
    );
    assert!(
        json.contains(SENTINEL),
        "expected the {SENTINEL} sentinel in default-sink output:\n{json}"
    );
}

#[test]
fn certificate_does_not_serialize_private_key_or_passphrase_to_default_sink() {
    let key = "serde-canary-private-key-never-emitted";
    let pass = "serde-canary-passphrase-never-emitted";
    let cert = Certificate::new("PUBLIC-CERT-CHAIN", SecretString::new(key))
        .with_passphrase(SecretString::new(pass));

    let json = serde_json::to_string(&cert).expect("Certificate must serialize");

    assert!(
        !json.contains(key),
        "certificate private key leaked to a default serde sink:\n{json}"
    );
    assert!(
        !json.contains(pass),
        "certificate passphrase leaked to a default serde sink:\n{json}"
    );
    // The cert chain is public material and SHOULD still serialize.
    assert!(
        json.contains("PUBLIC-CERT-CHAIN"),
        "public cert chain should round-trip even on the default sink:\n{json}"
    );
    assert!(
        json.contains(SENTINEL),
        "expected the {SENTINEL} sentinel in default-sink output:\n{json}"
    );
}

#[test]
fn oauth2_token_does_not_serialize_access_token_to_default_sink() {
    let raw = "serde-canary-oauth2-access-token-never-emitted";
    let token = OAuth2Token::new(SecretString::new(raw)).with_scopes(vec!["read".into()]);

    let json = serde_json::to_string(&token).expect("OAuth2Token must serialize");

    assert!(
        !json.contains(raw),
        "oauth2 access token leaked to a default serde sink:\n{json}"
    );
    assert!(
        json.contains(SENTINEL),
        "expected the {SENTINEL} sentinel in default-sink output:\n{json}"
    );
}

#[test]
fn signing_key_does_not_serialize_plaintext_to_default_sink() {
    let raw = "serde-canary-signing-key-never-emitted";
    let key = SigningKey::new(SecretString::new(raw), "hmac-sha256");

    let json = serde_json::to_string(&key).expect("SigningKey must serialize");

    assert!(
        !json.contains(raw),
        "signing-key plaintext leaked to a default serde sink:\n{json}"
    );
    // Non-secret algorithm metadata should still serialize.
    assert!(
        json.contains("hmac-sha256"),
        "non-secret algorithm metadata should serialize:\n{json}"
    );
    assert!(
        json.contains(SENTINEL),
        "expected the {SENTINEL} sentinel in default-sink output:\n{json}"
    );
}

#[test]
fn shared_key_does_not_serialize_plaintext_to_default_sink() {
    let raw = "serde-canary-shared-key-never-emitted";
    let key = SharedKey::new(SecretString::new(raw));

    let json = serde_json::to_string(&key).expect("SharedKey must serialize");

    assert!(
        !json.contains(raw),
        "shared-key plaintext leaked to a default serde sink:\n{json}"
    );
    assert!(
        json.contains(SENTINEL),
        "expected the {SENTINEL} sentinel in default-sink output:\n{json}"
    );
}

// ---------------------------------------------------------------------
// Storage contract: cleartext only inside `expose_for_serialization`,
// and that scope is the *only* path that round-trips secrets.
// ---------------------------------------------------------------------

#[test]
fn expose_scope_is_the_only_cleartext_serialization_path() {
    let raw = "serde-canary-storage-roundtrip-token";
    let token = SecretToken::new(SecretString::new(raw));

    // Outside the scope: redacted.
    let default_json = serde_json::to_string(&token).expect("serialize");
    assert!(
        !default_json.contains(raw),
        "control: default sink must redact:\n{default_json}"
    );

    // Inside the scope: cleartext, and it round-trips (the encrypted-at-rest
    // storage path depends on this full-fidelity serialization).
    let sealed_json = serde_secret::expose_for_serialization(|| serde_json::to_string(&token))
        .expect("serialize");
    assert!(
        sealed_json.contains(raw),
        "storage scope must emit cleartext for at-rest persistence:\n{sealed_json}"
    );
    assert!(
        !sealed_json.contains(SENTINEL),
        "storage scope must NOT redact:\n{sealed_json}"
    );

    let recovered: SecretToken =
        serde_json::from_str(&sealed_json).expect("sealed json must round-trip");
    assert_eq!(recovered.token().expose_secret(), raw);
}

#[test]
fn expose_scope_ends_when_the_closure_returns() {
    let raw = "serde-canary-scope-does-not-leak-across-calls";
    let token = SecretToken::new(SecretString::new(raw));

    // Drive a sealed serialization, then immediately serialize again on the
    // SAME thread with no scope. The second call must redact — proving the
    // scope is strictly closure-scoped and does not leak forward.
    let _sealed = serde_secret::expose_for_serialization(|| serde_json::to_string(&token));

    let after = serde_json::to_string(&token).expect("serialize");
    assert!(
        !after.contains(raw),
        "scope leaked past the closure — a later default-sink serialize emitted cleartext:\n{after}"
    );
}

#[test]
fn deserializing_a_redacted_blob_is_rejected() {
    // Loud-failure half of the gate: a persist site that forgot the storage
    // scope writes the `[REDACTED]` sentinel in place of the secret. Reading
    // that blob back must error, not silently load "[REDACTED]" as the token.
    let redacted =
        serde_json::to_string(&SecretToken::new(SecretString::new("anything"))).expect("serialize");
    assert!(redacted.contains(SENTINEL), "control: default sink redacts");

    let result = serde_json::from_str::<SecretToken>(&redacted);
    assert!(
        result.is_err(),
        "deserializing a redacted blob must be rejected; got {result:?}"
    );
}
