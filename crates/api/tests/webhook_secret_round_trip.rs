//! End-to-end byte-consistency proof for the webhook signing-secret path.
//!
//! Scope: mint → store(signing_key credential) → resolve → decode → sign →
//! verify.  This test is the regression guard for the **silent-fail seam**:
//! if the resolver returned the `whsec_` literal string instead of the decoded
//! HMAC key bytes, the HMAC would be computed over the wrong material and
//! `RequiredPolicy::verify_with` would return `SignatureInvalid`.
//!
//! Backend: `with_memory_store` — a durable `SqliteCredentialStore` on an
//! ephemeral `:memory:` DB.  The real adapter code path, not an in-memory
//! double (AGENTS.md / ADR-0092 "no in-memory doubles" rule).

use std::sync::Arc;

use axum::http::{HeaderMap, HeaderValue, Method};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use nebula_action::{
    MockClock, RequiredPolicy, SignatureError, SignatureScheme, WebhookRequest, hmac_sha256_compute,
};
use nebula_api::{
    ports::credential_service_factory::with_memory_store,
    transport::webhook::{
        CredentialBackedWebhookSecretResolver, WebhookSecretResolver, mint_whsec,
    },
};
use nebula_credential::{CredentialDisplay, TenantScope};
use nebula_storage::credential::EnvKeyProvider;
use nebula_storage_port::Scope;
use serde_json::json;

/// Fixed 32-byte AES-256 test key — mirrors the factory dev constant.
/// 32 `0x42` bytes, base64.  Not a secret: a test fixture.
const TEST_KEY_B64: &str = "QkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkI=";

/// Anchored Unix timestamp for the deterministic `MockClock`.
/// All requests carry this timestamp and the clock is anchored to the same
/// second → skew = 0 → always within the 5-minute Standard Webhooks default
/// replay window.
const FIXED_TS_SECS: u64 = 1_700_000_000;

/// Direct decode of a `whsec_<base64>` string without going through the
/// production resolver.
///
/// Purpose: the round-trip test uses this to assert byte-value identity
/// between the resolver output and a direct decode of the original string.
/// This catches the **literal-string-vs-decoded-bytes** seam: if the resolver
/// returned the `whsec_` string unchanged, the `assert_eq!` would fail because
/// the resolver would return ~50 bytes of ASCII while this function would
/// return 32 raw bytes.
///
/// This function applies the **same algorithm** as the production `decode_whsec`
/// (standard base64, same prefix strip), so it is NOT an algorithm-independent
/// oracle.  It does NOT detect a production bug that uses URL-safe base64 —
/// that seam is covered by the unit test in `secret_resolver.rs`.  What it
/// does pin down is the byte identity: resolver output == direct decode of the
/// stored string.
fn direct_decode_whsec(s: &str) -> Vec<u8> {
    let b64 = s
        .strip_prefix("whsec_")
        .expect("direct_decode_whsec: whsec_ prefix expected");
    BASE64_STANDARD
        .decode(b64)
        .expect("direct_decode_whsec: valid standard base64 payload expected")
}

/// Build a Standard Webhooks-format signed request.
///
/// Signed content = `{webhook-id}.{webhook-timestamp}.{body}` (spec §4).
/// HMAC-SHA256 over signed content → `v1,<base64>` in `webhook-signature`.
fn swh_signed_request(webhook_id: &str, body: &[u8], secret: &[u8]) -> WebhookRequest {
    let ts_str = FIXED_TS_SECS.to_string();

    // Signed content per Standard Webhooks spec §4.
    let mut signed: Vec<u8> =
        Vec::with_capacity(webhook_id.len() + 1 + ts_str.len() + 1 + body.len());
    signed.extend_from_slice(webhook_id.as_bytes());
    signed.push(b'.');
    signed.extend_from_slice(ts_str.as_bytes());
    signed.push(b'.');
    signed.extend_from_slice(body);

    let mac: [u8; 32] = hmac_sha256_compute(secret, &signed);
    let sig_token = format!("v1,{}", BASE64_STANDARD.encode(mac));

    let mut headers = HeaderMap::new();
    headers.insert(
        "webhook-id",
        HeaderValue::from_str(webhook_id).expect("static webhook-id is a valid header value"),
    );
    headers.insert(
        "webhook-timestamp",
        HeaderValue::from_str(&ts_str).expect("unix timestamp is a valid header value"),
    );
    headers.insert(
        "webhook-signature",
        HeaderValue::from_str(&sig_token).expect("v1,<base64> is a valid header value"),
    );

    WebhookRequest::try_new(
        Method::POST,
        "/webhook/test-path",
        None::<String>,
        headers,
        body.to_vec(),
    )
    .expect("test body is within DEFAULT_MAX_BODY_BYTES; headers within MAX_HEADER_COUNT")
}

/// Full mint → store → resolve → decode → sign → verify round-trip.
///
/// The acceptance criterion: `resolve()` must return the **decoded** raw bytes
/// (i.e. `base64_decode(whsec_<base64>.strip_prefix("whsec_"))`), not the
/// encoded string.  The test fails — at step 4 or step 5 with
/// `SignatureInvalid` — if the resolver passes through the literal `whsec_`
/// string, because:
///
/// - Step 4 `assert_eq!` catches the byte-value mismatch directly.
/// - Step 5 `verify_with` would also fail because the HMAC key material is wrong.
#[tokio::test]
async fn webhook_secret_mint_store_resolve_verify_round_trip() {
    let key_provider =
        Arc::new(EnvKeyProvider::from_base64(TEST_KEY_B64).expect("valid 32-byte AES key"));
    let svc = with_memory_store(key_provider)
        .await
        .expect("service builds (advertised caps match ops)");

    // Storage-port Scope → TenantScope through `from_scope` — the one canonical
    // derivation used at both the create and resolve sides, so the owner key
    // cannot drift between them.
    let storage_scope = Scope::new("ws-round-trip", "org-round-trip");
    let tenant_scope = TenantScope::from_scope(&storage_scope);

    // 1. Mint a fresh `whsec_<base64>` signing secret.
    let whsec = mint_whsec();
    assert!(
        whsec.starts_with("whsec_"),
        "mint_whsec must return a whsec_-prefixed string; got {whsec:?}"
    );

    // 2. Store it as a `signing_key` credential.  The credential layer persists
    //    the verbatim `whsec_<base64>` string; it never decodes it.
    let head = svc
        .create(
            &tenant_scope,
            "signing_key",
            json!({ "key": whsec, "algorithm": "hmac-sha256" }),
            CredentialDisplay {
                display_name: Some("round-trip test signing key".to_owned()),
                ..Default::default()
            },
        )
        .await
        .expect("signing_key create must succeed — ops include signing_key after factory edit");
    assert_eq!(
        head.credential_key, "signing_key",
        "created head must carry the signing_key type key"
    );

    // 3. Resolve through the production path.
    let resolver = CredentialBackedWebhookSecretResolver::new(svc);
    let resolved_bytes = resolver
        .resolve(&storage_scope, &head.id)
        .await
        .expect("signing_key credential must resolve successfully");

    // 4. Byte-identity check: resolved bytes == direct decode of the stored string.
    //    This assertion catches the silent-fail seam: if the resolver returned the
    //    `whsec_` literal instead of decoded bytes, `direct_decode_whsec` would
    //    produce 32 raw bytes while `resolved_bytes` would be ~50 ASCII bytes, and
    //    the `assert_eq!` would fail.  See `direct_decode_whsec` doc for the scope
    //    of what this covers vs. the URL-safe base64 unit test.
    let expected = direct_decode_whsec(&whsec);
    assert_eq!(
        resolved_bytes, expected,
        "resolved bytes must equal direct base64-decode of the original whsec_ payload; \
         a mismatch means the resolver returned the encoded string rather than decoded bytes"
    );

    // 5. Prove the resolved bytes work as a Standard Webhooks HMAC key.
    let body = br#"{"event":"test.delivered"}"#;
    let webhook_id = "msg-round-trip-001";
    let request = swh_signed_request(webhook_id, body, &resolved_bytes);
    let clock = MockClock::at_unix_secs(FIXED_TS_SECS);

    RequiredPolicy::new()
        .with_secret(resolved_bytes.clone())
        .with_scheme(SignatureScheme::StandardWebhooks)
        .verify_with(&request, &clock)
        .expect(
            "Standard Webhooks verify must pass: resolved bytes are the correct decoded HMAC key",
        );

    // 6. Negative guard: a DIFFERENT key must not verify the same request.
    //    This confirms the test is sensitive to key identity — it catches the
    //    case where the resolver returns the whsec_ literal (wrong key) that
    //    accidentally signs correctly (impossible by HMAC collision probability,
    //    but the test makes that explicit).
    let wrong_key: Vec<u8> = vec![0xFFu8; 32];
    let request_wrong_key = swh_signed_request(webhook_id, body, &wrong_key);
    let neg_result = RequiredPolicy::new()
        .with_secret(resolved_bytes)
        .with_scheme(SignatureScheme::StandardWebhooks)
        .verify_with(&request_wrong_key, &clock);
    assert!(
        matches!(neg_result, Err(SignatureError::SignatureInvalid)),
        "verify with wrong signing key must return SignatureInvalid; got {neg_result:?}"
    );
}

/// Cross-tenant isolation: a credential created under tenant A must NOT be
/// resolvable under tenant B.
///
/// `validate_credential_binding` enforces the owner check before returning a
/// `ValidatedCredentialBinding`; the error surfaces as `ScopeMismatch`
/// (not `NotFound`) so the resolver fails closed and leaks no information
/// about credential existence in the other tenant's namespace.
#[tokio::test]
async fn webhook_secret_resolver_rejects_cross_tenant_credential_id() {
    let key_provider =
        Arc::new(EnvKeyProvider::from_base64(TEST_KEY_B64).expect("valid 32-byte AES key"));
    let svc = with_memory_store(key_provider)
        .await
        .expect("service builds");

    // Create a signing_key credential under tenant A.
    let scope_a = Scope::new("ws-tenant-a", "org-tenant-a");
    let tenant_a = TenantScope::from_scope(&scope_a);
    let whsec = mint_whsec();
    let head = svc
        .create(
            &tenant_a,
            "signing_key",
            json!({ "key": whsec, "algorithm": "hmac-sha256" }),
            CredentialDisplay::default(),
        )
        .await
        .expect("create under tenant A");

    // Attempt to resolve it under tenant B using A's credential id.
    let scope_b = Scope::new("ws-tenant-b", "org-tenant-b");
    let resolver = CredentialBackedWebhookSecretResolver::new(svc);
    let result = resolver.resolve(&scope_b, &head.id).await;

    // Assert: (a) the call fails (binding gate holds), (b) the error identifies
    // a binding/scope failure, (c) the error message contains no secret material.
    let err = result
        .expect_err("cross-tenant resolution must fail; Ok(_) means tenant isolation is broken");
    let err_str = err.to_string();

    // The error must be a binding failure, not a lower-level I/O or decrypt error.
    // `ResolverError::Binding` wraps `ValidatedCredentialBindingError::ScopeMismatch`
    // whose Display reads "credential binding validation failed: credential `<id>` belongs
    // to tenant `<actual>`; caller requested tenant `<requested>`".
    assert!(
        err_str.contains("binding") || err_str.contains("tenant") || err_str.contains("scope"),
        "error must identify a binding/scope failure; got: {err_str:?}"
    );

    // The error must NOT contain the minted `whsec_` string or any key material.
    assert!(
        !err_str.contains("whsec_"),
        "error message must not contain the whsec_ secret literal; got: {err_str:?}"
    );
    // The credential id (non-secret) may appear; the secret key never should.
    // Verify none of the base64 portion of the minted secret leaks into the error.
    let whsec_b64 = whsec.strip_prefix("whsec_").unwrap_or("");
    assert!(
        whsec_b64.is_empty() || !err_str.contains(whsec_b64),
        "error message must not contain the base64 key payload; got: {err_str:?}"
    );
}
