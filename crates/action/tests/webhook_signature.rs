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

use std::time::{Duration, UNIX_EPOCH};

use nebula_action::{
    ActionError, SignatureOutcome, hmac_sha256_compute, verify_hmac_sha256,
    verify_hmac_sha256_base64, verify_hmac_sha256_with_timestamp, verify_tag_constant_time,
    webhook::webhook_request_for_test,
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

// ── H3: multi-valued signature header rejection ──────────────────────────

#[test]
fn verify_hmac_sha256_rejects_multi_valued_header() {
    let body = br#"{"x":1}"#;
    let secret = b"k";
    let valid_sig = sig_hex(secret, body);
    // Build a request with TWO X-Signature headers — one valid, one
    // garbage. An attacker upstream via proxy chain can produce this.
    // `headers.get()` returns the first, which in some proxy layouts
    // is the injected one. Strict single-value rejection is the only
    // safe answer.
    let req = webhook_request_for_test(
        body,
        &[("X-Signature", &valid_sig), ("X-Signature", "deadbeef")],
    )
    .unwrap();
    assert_eq!(
        verify_hmac_sha256(&req, secret, "X-Signature").unwrap(),
        SignatureOutcome::Invalid,
        "multi-valued sig header must be rejected even if one value is valid",
    );
}

// ── H4: base64-HMAC helper (Shopify / Square) ────────────────────────────

#[test]
fn verify_hmac_sha256_base64_accepts_valid_signature() {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    let body = br#"{"order_id":42}"#;
    let secret = b"shopify_secret";
    let tag = hmac_sha256_compute(secret, body);
    let sig = STANDARD.encode(tag);
    let req = webhook_request_for_test(body, &[("X-Shopify-Hmac-Sha256", &sig)]).unwrap();
    assert_eq!(
        verify_hmac_sha256_base64(&req, secret, "X-Shopify-Hmac-Sha256").unwrap(),
        SignatureOutcome::Valid
    );
}

#[test]
fn verify_hmac_sha256_base64_rejects_invalid_base64() {
    let req = webhook_request_for_test(
        b"body",
        &[("X-Shopify-Hmac-Sha256", "not-valid-base64!!!!")],
    )
    .unwrap();
    assert_eq!(
        verify_hmac_sha256_base64(&req, b"k", "X-Shopify-Hmac-Sha256").unwrap(),
        SignatureOutcome::Invalid
    );
}

#[test]
fn verify_hmac_sha256_base64_rejects_wrong_secret() {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    let body = b"hello";
    let tag = hmac_sha256_compute(b"wrong", body);
    let sig = STANDARD.encode(tag);
    let req = webhook_request_for_test(body, &[("X-Shopify-Hmac-Sha256", &sig)]).unwrap();
    assert_eq!(
        verify_hmac_sha256_base64(&req, b"right", "X-Shopify-Hmac-Sha256").unwrap(),
        SignatureOutcome::Invalid
    );
}

// ── H2: verify_hmac_sha256_with_timestamp (Stripe / Slack) ───────────────

/// Build a `WebhookRequest` whose `received_at` is pinned to a specific
/// epoch time, so replay-window tests are deterministic.
fn req_at(body: &[u8], headers: &[(&str, &str)], epoch_secs: u64) -> nebula_action::WebhookRequest {
    let req = webhook_request_for_test(body, headers).unwrap();
    req.with_received_at(UNIX_EPOCH + Duration::from_secs(epoch_secs))
}

#[test]
fn verify_hmac_sha256_with_timestamp_stripe_scheme() {
    let secret = b"whsec_stripe";
    let ts = 1_700_000_000u64;
    let body = br#"{"event":"invoice.paid"}"#;
    // Stripe canonical form: "{ts}.{body}"
    let canonical = format!("{ts}.{}", std::str::from_utf8(body).unwrap());
    let sig = hex::encode(hmac_sha256_compute(secret, canonical.as_bytes()));

    let req = req_at(
        body,
        &[("Stripe-Ts", &ts.to_string()), ("Stripe-Signature", &sig)],
        ts,
    );

    let result = verify_hmac_sha256_with_timestamp(
        &req,
        secret,
        "Stripe-Signature",
        "Stripe-Ts",
        Duration::from_mins(5),
        |t, b| format!("{t}.{}", std::str::from_utf8(b).unwrap()).into_bytes(),
    )
    .unwrap();
    assert_eq!(result, SignatureOutcome::Valid);
}

#[test]
fn verify_hmac_sha256_with_timestamp_slack_scheme() {
    let secret = b"slack_signing_secret";
    let ts = 1_700_000_000u64;
    let body = br"payload=%7B%22type%22%3A%22event%22%7D";
    // Slack canonical form: "v0:{ts}:{body}"
    let canonical = format!("v0:{ts}:{}", std::str::from_utf8(body).unwrap());
    let sig = hex::encode(hmac_sha256_compute(secret, canonical.as_bytes()));

    let req = req_at(
        body,
        &[
            ("X-Slack-Request-Timestamp", &ts.to_string()),
            ("X-Slack-Signature", &sig),
        ],
        ts,
    );

    let result = verify_hmac_sha256_with_timestamp(
        &req,
        secret,
        "X-Slack-Signature",
        "X-Slack-Request-Timestamp",
        Duration::from_mins(5),
        |t, b| format!("v0:{t}:{}", std::str::from_utf8(b).unwrap()).into_bytes(),
    )
    .unwrap();
    assert_eq!(result, SignatureOutcome::Valid);
}

#[test]
fn verify_hmac_sha256_with_timestamp_rejects_old_timestamp() {
    let secret = b"k";
    let body = b"body";
    // ts is 10 minutes before received_at, tolerance 5 minutes
    let ts = 1_700_000_000u64;
    let received = ts + 600;
    let canonical = format!("{ts}.{}", std::str::from_utf8(body).unwrap());
    let sig = hex::encode(hmac_sha256_compute(secret, canonical.as_bytes()));

    let req = req_at(body, &[("Ts", &ts.to_string()), ("Sig", &sig)], received);

    let result = verify_hmac_sha256_with_timestamp(
        &req,
        secret,
        "Sig",
        "Ts",
        Duration::from_mins(5), // 5 min tolerance
        |t, b| format!("{t}.{}", std::str::from_utf8(b).unwrap()).into_bytes(),
    )
    .unwrap();
    // Even though the HMAC is valid, the stale timestamp forces Invalid.
    assert_eq!(result, SignatureOutcome::Invalid);
}

#[test]
fn verify_hmac_sha256_with_timestamp_rejects_future_timestamp() {
    let secret = b"k";
    let body = b"body";
    // ts is 5 minutes ahead of received_at — far beyond the 60 s
    // forward skew allowance.
    let received = 1_700_000_000u64;
    let ts = received + 300;
    let canonical = format!("{ts}.{}", std::str::from_utf8(body).unwrap());
    let sig = hex::encode(hmac_sha256_compute(secret, canonical.as_bytes()));

    let req = req_at(body, &[("Ts", &ts.to_string()), ("Sig", &sig)], received);

    let result = verify_hmac_sha256_with_timestamp(
        &req,
        secret,
        "Sig",
        "Ts",
        Duration::from_hours(1), // huge tolerance for past; forward is capped
        |t, b| format!("{t}.{}", std::str::from_utf8(b).unwrap()).into_bytes(),
    )
    .unwrap();
    assert_eq!(
        result,
        SignatureOutcome::Invalid,
        "future timestamp beyond 60s forward skew must be rejected"
    );
}

#[test]
fn verify_hmac_sha256_with_timestamp_rejects_non_numeric_ts() {
    let req = req_at(
        b"body",
        &[("Ts", "not-a-number"), ("Sig", "deadbeef")],
        1_700_000_000,
    );
    let result = verify_hmac_sha256_with_timestamp(
        &req,
        b"k",
        "Sig",
        "Ts",
        Duration::from_mins(5),
        |t, b| format!("{t}.{}", std::str::from_utf8(b).unwrap()).into_bytes(),
    )
    .unwrap();
    assert_eq!(result, SignatureOutcome::Invalid);
}

#[test]
fn verify_hmac_sha256_with_timestamp_empty_secret_is_error() {
    let req = req_at(
        b"body",
        &[("Ts", "1700000000"), ("Sig", "deadbeef")],
        1_700_000_000,
    );
    let err = verify_hmac_sha256_with_timestamp(
        &req,
        b"",
        "Sig",
        "Ts",
        Duration::from_mins(5),
        |t, b| format!("{t}.{}", std::str::from_utf8(b).unwrap()).into_bytes(),
    )
    .unwrap_err();
    assert!(matches!(err, ActionError::Validation { .. }));
}

#[test]
fn verify_hmac_sha256_with_timestamp_rejects_multi_valued_ts() {
    let req = req_at(
        b"body",
        &[
            ("Ts", "1700000000"),
            ("Ts", "1700000001"),
            ("Sig", "deadbeef"),
        ],
        1_700_000_000,
    );
    let result = verify_hmac_sha256_with_timestamp(
        &req,
        b"k",
        "Sig",
        "Ts",
        Duration::from_mins(5),
        |t, b| format!("{t}.{}", std::str::from_utf8(b).unwrap()).into_bytes(),
    )
    .unwrap();
    assert_eq!(result, SignatureOutcome::Invalid);
}

// ── H9: timing-invariant hex decode ──────────────────────────────────────
//
// Behavioural test (not timing-based — timing assertions are flaky in
// CI): with the new implementation, an invalid-hex signature still
// runs the full MAC computation internally. We can't observe this
// from a behavioural test directly, but we can assert that both
// invalid-hex and valid-hex-wrong-secret produce the same outcome
// (`Invalid`) and don't error — i.e. no path short-circuits early.

#[test]
fn verify_hmac_sha256_invalid_hex_returns_invalid_like_wrong_signature() {
    let body = b"payload";
    let invalid_hex_req = webhook_request_for_test(body, &[("X-Signature", "not-hex")]).unwrap();
    let wrong_sig_req =
        webhook_request_for_test(body, &[("X-Signature", &"ab".repeat(32))]).unwrap();

    assert_eq!(
        verify_hmac_sha256(&invalid_hex_req, b"k", "X-Signature").unwrap(),
        SignatureOutcome::Invalid
    );
    assert_eq!(
        verify_hmac_sha256(&wrong_sig_req, b"k", "X-Signature").unwrap(),
        SignatureOutcome::Invalid
    );
}
