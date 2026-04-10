//! Integration tests for `nebula_action::webhook` signature helpers.
//!
//! Covers the full matrix:
//! - valid signatures (bare hex + `sha256=` prefix)
//! - tampered body / wrong secret (Invalid)
//! - missing / malformed / wrong-length headers
//! - header case-insensitive lookup
//! - empty-secret guard (`Validation`)
//! - `verify_tag_constant_time` for custom schemes (Stripe-style)
//!
//! None of the assertions rely on timing — the timing-safety guarantee
//! comes from delegating to `subtle::ConstantTimeEq` inside
//! `hmac::Mac::verify_slice`. We assert correctness here.

use nebula_action::ActionError;
use nebula_action::handler::IncomingEvent;
use nebula_action::webhook::{
    SignatureOutcome, hmac_sha256_compute, verify_hmac_sha256, verify_tag_constant_time,
};

fn sig_hex(secret: &[u8], body: &[u8]) -> String {
    hex::encode(hmac_sha256_compute(secret, body))
}

#[test]
fn valid_bare_hex_signature_accepted() {
    let body = br#"{"x":1}"#;
    let secret = b"whsec_test";
    let sig = sig_hex(secret, body);
    let event = IncomingEvent::new(body, &[("X-Signature", &sig)]);
    assert_eq!(
        verify_hmac_sha256(&event, secret, "X-Signature").unwrap(),
        SignatureOutcome::Valid
    );
}

#[test]
fn valid_sha256_prefixed_signature_accepted() {
    let body = br#"{"x":1}"#;
    let secret = b"gh-secret";
    let sig = format!("sha256={}", sig_hex(secret, body));
    let event = IncomingEvent::new(body, &[("X-Hub-Signature-256", &sig)]);
    assert_eq!(
        verify_hmac_sha256(&event, secret, "X-Hub-Signature-256").unwrap(),
        SignatureOutcome::Valid
    );
}

#[test]
fn wrong_secret_rejected() {
    let body = br#"{"x":1}"#;
    let sig = sig_hex(b"correct", body);
    let event = IncomingEvent::new(body, &[("X-Signature", &sig)]);
    assert_eq!(
        verify_hmac_sha256(&event, b"wrong", "X-Signature").unwrap(),
        SignatureOutcome::Invalid
    );
}

#[test]
fn tampered_body_rejected() {
    let sig = sig_hex(b"k", b"original");
    let event = IncomingEvent::new(b"tampered", &[("X-Signature", &sig)]);
    assert_eq!(
        verify_hmac_sha256(&event, b"k", "X-Signature").unwrap(),
        SignatureOutcome::Invalid
    );
}

#[test]
fn missing_header_returns_missing() {
    let event = IncomingEvent::new(b"body", &[]);
    assert_eq!(
        verify_hmac_sha256(&event, b"k", "X-Signature").unwrap(),
        SignatureOutcome::Missing
    );
}

#[test]
fn header_lookup_is_case_insensitive() {
    let body = b"payload";
    let sig = sig_hex(b"k", body);
    // Header stored under lowercase key but queried with mixed case.
    let event = IncomingEvent::new(body, &[("x-signature", &sig)]);
    assert!(
        verify_hmac_sha256(&event, b"k", "X-Signature")
            .unwrap()
            .is_valid()
    );
}

#[test]
fn invalid_hex_returns_invalid_not_error() {
    // Not a panic, not an error — just Invalid. The action author's
    // `is_valid()` check handles this alongside the wrong-digest case.
    let event = IncomingEvent::new(b"body", &[("X-Signature", "not-hex-zzz")]);
    assert_eq!(
        verify_hmac_sha256(&event, b"k", "X-Signature").unwrap(),
        SignatureOutcome::Invalid
    );
}

#[test]
fn wrong_length_digest_rejected_without_panic() {
    // Valid hex but not the 32 bytes HMAC-SHA256 produces.
    // `verify_slice` handles the length mismatch in constant time.
    let event = IncomingEvent::new(b"body", &[("X-Signature", "abcd")]);
    assert_eq!(
        verify_hmac_sha256(&event, b"k", "X-Signature").unwrap(),
        SignatureOutcome::Invalid
    );
}

#[test]
fn empty_secret_is_validation_error() {
    // An empty key accepts ANY input as valid — the only way to fail
    // closed is to refuse the operation entirely.
    let event = IncomingEvent::new(b"body", &[("X-Signature", "deadbeef")]);
    let err = verify_hmac_sha256(&event, b"", "X-Signature").unwrap_err();
    assert!(matches!(err, ActionError::Validation(_)));
}

#[test]
fn verify_tag_constant_time_length_mismatch() {
    // Different lengths must return false (no panic, no content branch).
    assert!(!verify_tag_constant_time(&[1, 2, 3], &[1, 2, 3, 4]));
    // Identical bytes match.
    assert!(verify_tag_constant_time(&[1, 2, 3], &[1, 2, 3]));
    // Same length, different content.
    assert!(!verify_tag_constant_time(&[1, 2, 3], &[1, 2, 4]));
    // Empty-empty matches.
    assert!(verify_tag_constant_time(&[], &[]));
}

#[test]
fn stripe_style_custom_scheme_roundtrip() {
    // Stripe signs "{timestamp}.{body}" with HMAC-SHA256 and carries
    // the tag in the "t=…,v1=…" header. We don't provide a helper for
    // the parsing — the escape hatch is to build the signed payload
    // and verify the tag yourself.
    let secret = b"whsec_stripe";
    let ts = "1700000000";
    let body = br#"{"event":"invoice.paid"}"#;
    let signed = format!("{ts}.{}", std::str::from_utf8(body).unwrap());
    let tag = hmac_sha256_compute(secret, signed.as_bytes());
    // Recomputing with the same secret must match; recomputing with a
    // tampered timestamp must not.
    let recomputed = hmac_sha256_compute(secret, signed.as_bytes());
    assert!(verify_tag_constant_time(&tag, &recomputed));

    let tampered = format!("1700000001.{}", std::str::from_utf8(body).unwrap());
    let tampered_tag = hmac_sha256_compute(secret, tampered.as_bytes());
    assert!(!verify_tag_constant_time(&tag, &tampered_tag));
}
