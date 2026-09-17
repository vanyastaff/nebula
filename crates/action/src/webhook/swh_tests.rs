//! Unit tests for [`SignatureScheme::StandardWebhooks`] via
//! [`RequiredPolicy::verify_with`].
//!
//! All tests use a deterministic [`MockClock`] so replay-window
//! assertions are immune to wall-clock drift.

use base64::{Engine as _, engine::general_purpose::STANDARD as B64};

use super::*;

/// Fixed Unix-epoch anchor shared across tests.
const NOW_SECS: u64 = 1_700_000_000;

/// Test HMAC key (raw bytes — no whsec_ prefix).
const KEY: &[u8] = b"test-secret-key-for-standard-webhooks";

/// Build a [`RequiredPolicy`] configured for Standard Webhooks.
fn swh_policy() -> RequiredPolicy {
    RequiredPolicy::new()
        .with_secret(KEY)
        .with_scheme(SignatureScheme::StandardWebhooks)
}

/// Compute a valid Standard Webhooks signature token for the given
/// message-id, timestamp-string, and body.
///
/// Returns a `webhook-signature` header value of the form `v1,<base64>`.
fn sign(msg_id: &str, ts_str: &str, body: &[u8]) -> String {
    let prefix = format!("{msg_id}.{ts_str}.");
    let mut content = Vec::with_capacity(prefix.len() + body.len());
    content.extend_from_slice(prefix.as_bytes());
    content.extend_from_slice(body);
    let mac = hmac_sha256_compute(KEY, &content);
    format!("v1,{}", B64.encode(mac))
}

fn make_request(msg_id: &str, ts_str: &str, sig: &str, body: &[u8]) -> WebhookRequest {
    webhook_request_for_test(
        body,
        &[
            ("webhook-id", msg_id),
            ("webhook-timestamp", ts_str),
            ("webhook-signature", sig),
        ],
    )
    .expect("test request must be constructable")
}

// ── Happy path ────────────────────────────────────────────────────────────

/// Valid Standard Webhooks request → `Ok(())`.
#[test]
fn swh_valid_signature_passes() {
    let clock = MockClock::at_unix_secs(NOW_SECS);
    let ts = NOW_SECS.to_string();
    let body = b"{\"event\":\"test\"}";
    let sig = sign("msg_abc123", &ts, body);
    let req = make_request("msg_abc123", &ts, &sig, body);
    swh_policy()
        .verify_with(&req, &clock)
        .expect("valid SWH signature must pass");
}

// ── Tampering ─────────────────────────────────────────────────────────────

/// Tampered body → signature mismatch → `SignatureInvalid`.
///
/// Red-on-revert: removing the body from the signed content makes a
/// body-tamper undetectable.
#[test]
fn swh_tampered_body_is_invalid() {
    let clock = MockClock::at_unix_secs(NOW_SECS);
    let ts = NOW_SECS.to_string();
    let original_body = b"{\"event\":\"test\"}";
    let tampered_body = b"{\"event\":\"hacked\"}";
    let sig = sign("msg_abc123", &ts, original_body);
    // Request body differs from what was signed.
    let req = make_request("msg_abc123", &ts, &sig, tampered_body);
    let err = swh_policy()
        .verify_with(&req, &clock)
        .expect_err("tampered body must fail verification");
    assert!(
        matches!(err, SignatureError::SignatureInvalid),
        "expected SignatureInvalid, got {err:?}"
    );
}

/// Tampered timestamp → signature mismatch → `SignatureInvalid`.
///
/// The timestamp is part of the signed content, so changing it after
/// signing invalidates the MAC.
#[test]
fn swh_tampered_timestamp_is_invalid() {
    let clock = MockClock::at_unix_secs(NOW_SECS);
    let ts = NOW_SECS.to_string();
    let tampered_ts = (NOW_SECS - 10).to_string(); // different ts sent in header
    let body = b"{\"event\":\"test\"}";
    let sig = sign("msg_abc123", &ts, body);
    // Header has a different timestamp than what was signed.
    let req = make_request("msg_abc123", &tampered_ts, &sig, body);
    let err = swh_policy()
        .verify_with(&req, &clock)
        .expect_err("tampered timestamp must fail verification");
    assert!(
        matches!(err, SignatureError::SignatureInvalid),
        "expected SignatureInvalid, got {err:?}"
    );
}

// ── Replay window ─────────────────────────────────────────────────────────

/// Timestamp older than the replay window → `TimestampOutOfWindow`.
#[test]
fn swh_expired_timestamp_rejected() {
    // Advance clock 6 minutes past the request timestamp.
    let req_ts = NOW_SECS;
    let clock_now = NOW_SECS + 360; // 6 min later, outside 5-min window
    let clock = MockClock::at_unix_secs(clock_now);
    let ts = req_ts.to_string();
    let body = b"{}";
    let sig = sign("msg_exp", &ts, body);
    let req = make_request("msg_exp", &ts, &sig, body);
    let err = swh_policy()
        .verify_with(&req, &clock)
        .expect_err("expired timestamp must be rejected");
    assert!(
        matches!(err, SignatureError::TimestampOutOfWindow { .. }),
        "expected TimestampOutOfWindow, got {err:?}"
    );
}

/// Timestamp more than 60 s in the future → `TimestampOutOfWindow`.
///
/// The future-skew cap (`FUTURE_SKEW_SECS = 60`) applies independently of
/// the configured replay window, preventing tokens pre-dated in the future.
#[test]
fn swh_future_skew_beyond_cap_rejected() {
    let req_ts = NOW_SECS + 120; // 2 min ahead — exceeds the 60-s cap
    let clock_now = NOW_SECS;
    let clock = MockClock::at_unix_secs(clock_now);
    let ts = req_ts.to_string();
    let body = b"{}";
    let sig = sign("msg_future", &ts, body);
    let req = make_request("msg_future", &ts, &sig, body);
    let err = swh_policy()
        .verify_with(&req, &clock)
        .expect_err("far-future timestamp must be rejected");
    assert!(
        matches!(err, SignatureError::TimestampOutOfWindow { .. }),
        "expected TimestampOutOfWindow, got {err:?}"
    );
}

// ── Missing headers ───────────────────────────────────────────────────────

/// Missing `webhook-timestamp` → `TimestampMissing` (mandatory for SWH).
#[test]
fn swh_missing_timestamp_is_error() {
    let clock = MockClock::at_unix_secs(NOW_SECS);
    let ts = NOW_SECS.to_string();
    let body = b"{}";
    let sig = sign("msg_nots", &ts, body);
    // No webhook-timestamp header.
    let req = webhook_request_for_test(
        body,
        &[("webhook-id", "msg_nots"), ("webhook-signature", &sig)],
    )
    .expect("request build");
    let err = swh_policy()
        .verify_with(&req, &clock)
        .expect_err("missing timestamp must error");
    assert!(
        matches!(err, SignatureError::TimestampMissing),
        "expected TimestampMissing, got {err:?}"
    );
}

/// Missing `webhook-signature` → `SignatureMissing`.
#[test]
fn swh_missing_signature_header() {
    let clock = MockClock::at_unix_secs(NOW_SECS);
    let ts = NOW_SECS.to_string();
    let body = b"{}";
    // No webhook-signature header.
    let req = webhook_request_for_test(
        body,
        &[("webhook-id", "msg_nosig"), ("webhook-timestamp", &ts)],
    )
    .expect("request build");
    let err = swh_policy()
        .verify_with(&req, &clock)
        .expect_err("missing signature must error");
    assert!(
        matches!(err, SignatureError::SignatureMissing),
        "expected SignatureMissing, got {err:?}"
    );
}

// ── Multi-signature scenarios ─────────────────────────────────────────────

/// `v1,<bad> v1,<good>` → first candidate fails, second succeeds → `Valid`.
///
/// Proves the candidate loop does not short-circuit on a bad token.
#[test]
fn swh_multi_sig_bad_then_good_passes() {
    let clock = MockClock::at_unix_secs(NOW_SECS);
    let ts = NOW_SECS.to_string();
    let body = b"{}";
    let good = sign("msg_multi", &ts, body);
    let bad = "v1,AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
    let sig_header = format!("{bad} {good}");
    let req = make_request("msg_multi", &ts, &sig_header, body);
    swh_policy()
        .verify_with(&req, &clock)
        .expect("bad-then-good multi-sig must pass");
}

/// `v2,<x> v1,<good>` → non-v1 ignored, v1 accepted → `Valid`.
///
/// Proves algorithm-confusion guard: `v2` is never selected even if it
/// appears first.
#[test]
fn swh_v2_ignored_v1_accepted() {
    let clock = MockClock::at_unix_secs(NOW_SECS);
    let ts = NOW_SECS.to_string();
    let body = b"{}";
    let good = sign("msg_v2v1", &ts, body);
    let sig_header = format!("v2,AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA= {good}");
    let req = make_request("msg_v2v1", &ts, &sig_header, body);
    swh_policy()
        .verify_with(&req, &clock)
        .expect("v2 must be ignored; v1 must pass");
}

/// Only `v2,<x>` token present → no v1 candidate → `SignatureMissing`.
/// A `RequiredPolicy` with a custom `timestamp_header` switched to
/// `StandardWebhooks` must NOT apply the generic timestamp pre-check.
///
/// Before the fix, `verify_with` ran `validate_timestamp(custom_header, …)`
/// BEFORE dispatching to `verify_standard_webhooks`, so a valid SWH
/// request (which carries `webhook-timestamp`, not the custom header)
/// returned `TimestampMissing` for the absent custom header.
///
/// RED-on-revert: removing the `!matches!(self.scheme, StandardWebhooks)`
/// guard causes `validate_timestamp` to run for the custom header, which
/// is absent from the request → `TimestampMissing` → test fails.
#[test]
fn swh_stale_custom_timestamp_header_does_not_reject_valid_request() {
    let clock = MockClock::at_unix_secs(NOW_SECS);
    let ts = NOW_SECS.to_string();
    let body = b"{}";
    let msg_id = "msg_custom_ts";
    let sig = sign(msg_id, &ts, body);

    // A policy that previously had a custom timestamp header, then was
    // switched to StandardWebhooks. The stale `timestamp_header` is set.
    let policy = RequiredPolicy::new()
        .with_secret(KEY)
        .with_timestamp_header(HeaderName::from_static("x-my-custom-ts"))
        .with_scheme(SignatureScheme::StandardWebhooks);

    // Request carries the Standard Webhooks headers, NOT the custom ts header.
    let req = make_request(msg_id, &ts, &sig, body);

    // Must succeed: SWH validates webhook-timestamp; the custom header is irrelevant.
    policy
        .verify_with(&req, &clock)
        .expect("valid SWH request with stale custom timestamp_header must pass");
}

#[test]
fn swh_only_v2_no_v1_is_missing() {
    let clock = MockClock::at_unix_secs(NOW_SECS);
    let ts = NOW_SECS.to_string();
    let body = b"{}";
    let sig_header = "v2,AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
    let req = make_request("msg_v2only", &ts, sig_header, body);
    let err = swh_policy()
        .verify_with(&req, &clock)
        .expect_err("only-v2 must be SignatureMissing");
    assert!(
        matches!(err, SignatureError::SignatureMissing),
        "expected SignatureMissing, got {err:?}"
    );
}
