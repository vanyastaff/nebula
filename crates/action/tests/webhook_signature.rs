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

use nebula_action::{
    ActionError, SignatureOutcome, hmac_sha256_compute, verify_hmac_sha256,
    verify_tag_constant_time, webhook::webhook_request_for_test,
};

fn sig_hex(secret: &[u8], body: &[u8]) -> String {
    hex::encode(hmac_sha256_compute(secret, body))
}

#[test]
fn valid_bare_hex_signature_accepted() {
    let body = br#"{"x":1}"#;
    let secret = b"whsec_test";
    let sig = sig_hex(secret, body);
    let req = webhook_request_for_test(body, &[("X-Signature", &sig)]).unwrap();
    assert_eq!(
        verify_hmac_sha256(&req, secret, "X-Signature").unwrap(),
        SignatureOutcome::Valid
    );
}

#[test]
fn valid_sha256_prefixed_signature_accepted() {
    let body = br#"{"x":1}"#;
    let secret = b"gh-secret";
    let sig = format!("sha256={}", sig_hex(secret, body));
    let req = webhook_request_for_test(body, &[("X-Hub-Signature-256", &sig)]).unwrap();
    assert_eq!(
        verify_hmac_sha256(&req, secret, "X-Hub-Signature-256").unwrap(),
        SignatureOutcome::Valid
    );
}

#[test]
fn wrong_secret_rejected() {
    let body = br#"{"x":1}"#;
    let sig = sig_hex(b"correct", body);
    let req = webhook_request_for_test(body, &[("X-Signature", &sig)]).unwrap();
    assert_eq!(
        verify_hmac_sha256(&req, b"wrong", "X-Signature").unwrap(),
        SignatureOutcome::Invalid
    );
}

#[test]
fn tampered_body_rejected() {
    let sig = sig_hex(b"k", b"original");
    let req = webhook_request_for_test(b"tampered", &[("X-Signature", &sig)]).unwrap();
    assert_eq!(
        verify_hmac_sha256(&req, b"k", "X-Signature").unwrap(),
        SignatureOutcome::Invalid
    );
}

#[test]
fn missing_header_returns_missing() {
    let req = webhook_request_for_test(b"body", &[]).unwrap();
    assert_eq!(
        verify_hmac_sha256(&req, b"k", "X-Signature").unwrap(),
        SignatureOutcome::Missing
    );
}

#[test]
fn header_lookup_is_case_insensitive() {
    let body = b"payload";
    let sig = sig_hex(b"k", body);
    let req = webhook_request_for_test(body, &[("x-signature", &sig)]).unwrap();
    assert!(
        verify_hmac_sha256(&req, b"k", "X-Signature")
            .unwrap()
            .is_valid()
    );
}

#[test]
fn invalid_hex_returns_invalid_not_error() {
    let req = webhook_request_for_test(b"body", &[("X-Signature", "not-hex-zzz")]).unwrap();
    assert_eq!(
        verify_hmac_sha256(&req, b"k", "X-Signature").unwrap(),
        SignatureOutcome::Invalid
    );
}

#[test]
fn wrong_length_digest_rejected_without_panic() {
    let req = webhook_request_for_test(b"body", &[("X-Signature", "abcd")]).unwrap();
    assert_eq!(
        verify_hmac_sha256(&req, b"k", "X-Signature").unwrap(),
        SignatureOutcome::Invalid
    );
}

#[test]
fn empty_secret_is_validation_error() {
    let req = webhook_request_for_test(b"body", &[("X-Signature", "deadbeef")]).unwrap();
    let err = verify_hmac_sha256(&req, b"", "X-Signature").unwrap_err();
    assert!(matches!(err, ActionError::Validation { .. }));
}

#[test]
fn verify_tag_constant_time_length_mismatch() {
    assert!(!verify_tag_constant_time(&[1, 2, 3], &[1, 2, 3, 4]));
    assert!(verify_tag_constant_time(&[1, 2, 3], &[1, 2, 3]));
    assert!(!verify_tag_constant_time(&[1, 2, 3], &[1, 2, 4]));
    assert!(verify_tag_constant_time(&[], &[]));
}

#[test]
fn stripe_style_custom_scheme_roundtrip() {
    let secret = b"whsec_stripe";
    let ts = "1700000000";
    let body = br#"{"event":"invoice.paid"}"#;
    let signed = format!("{ts}.{}", std::str::from_utf8(body).unwrap());
    let tag = hmac_sha256_compute(secret, signed.as_bytes());

    let recomputed = hmac_sha256_compute(secret, signed.as_bytes());
    assert!(verify_tag_constant_time(&tag, &recomputed));

    let tampered = format!("1700000001.{}", std::str::from_utf8(body).unwrap());
    let tampered_tag = hmac_sha256_compute(secret, tampered.as_bytes());
    assert!(!verify_tag_constant_time(&tag, &tampered_tag));
}
