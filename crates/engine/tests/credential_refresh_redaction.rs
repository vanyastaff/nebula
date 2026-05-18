#![cfg(all(feature = "rotation", feature = "test-util"))]

//! ADR-0030 §4 redaction CI gate. **One row per token_refresh code path.**
//!
//! Each test injects a secret-bearing IdP response through the bounded
//! response parser and asserts that the resulting error rendering (Display,
//! Debug, summary fields) does NOT contain the submitted secret and DOES
//! contain the `[REDACTED]` sentinel.
//!
//! Adding a new token_refresh code path = adding a new row here.
//!
//! # First row: SEC-13 — refresh_token echoed in error_description
//!
//! Source: `docs/tracking/credential-audit-2026-04-27.md` §XII Errata,
//! security hardening spec Stage 0.5.

use nebula_engine::credential::rotation::{
    TokenRefreshError, token_refresh::parse_oauth_token_response_for_tests,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

/// Drains a single HTTP/1.1 request until the header block ends.
/// Mirrors the helper in `token_refresh.rs` inline tests; duplicated here to
/// keep the integration test file standalone.
async fn drain_incoming_request(stream: &mut tokio::net::TcpStream) {
    let mut acc = Vec::new();
    let mut buf = [0u8; 1024];
    loop {
        let n = stream
            .read(&mut buf)
            .await
            .expect("read request from client");
        if n == 0 {
            break;
        }
        acc.extend_from_slice(&buf[..n]);
        if acc.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        if acc.len() > 64 * 1024 {
            return;
        }
    }
}

/// Spawn a one-shot mock token endpoint that returns the given JSON body
/// with the given HTTP status.
async fn spawn_idp_returning(status: u16, body: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept");
        drain_incoming_request(&mut stream).await;
        let n = body.len();
        let status_line = match status {
            400 => "400 Bad Request",
            401 => "401 Unauthorized",
            500 => "500 Internal Server Error",
            _ => "400 Bad Request",
        };
        let head = format!(
            "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nContent-Length: {n}\r\nConnection: close\r\n\r\n"
        );
        if stream.write_all(head.as_bytes()).await.is_err() {
            return;
        }
        let _ = stream.write_all(body.as_bytes()).await;
    });
    format!("http://127.0.0.1:{}/token", addr.port())
}

async fn parse_idp_error_response(status: u16, body: &'static str) -> TokenRefreshError {
    let token_url = spawn_idp_returning(status, body).await;
    let resp = reqwest::Client::new()
        .post(token_url)
        .send()
        .await
        .expect("token endpoint response");
    parse_oauth_token_response_for_tests(resp)
        .await
        .expect_err("non-2xx token response expected")
}

/// Asserts a string does NOT contain any plausible-looking
/// secret literal that the IdP echoed.
fn assert_no_secret_substring(haystack: &str, secret: &str, context: &str) {
    assert!(
        !haystack.contains(secret),
        "redaction gate violation: secret {secret:?} leaked into {context}: {haystack:?}"
    );
}

// ============================================================================
// Row 1: SEC-13 — refresh_token=<secret> in error_description on invalid_grant
// ============================================================================

#[tokio::test]
async fn sec13_refresh_token_in_error_description_is_redacted() {
    const SECRET: &str = "abc123-leaked-refresh-token-value";
    let err = parse_idp_error_response(
        400,
        // IdP echoes the submitted refresh_token inside error_description.
        // Real-world pattern observed on some buggy IdPs returning invalid_grant.
        // Note: literal must match SECRET above; static string for `'static` body.
        "{\"error\":\"invalid_grant\",\"error_description\":\"refresh_token=abc123-leaked-refresh-token-value expired\"}",
    )
    .await;

    let TokenRefreshError::TokenEndpoint { summary, status } = &err else {
        panic!("expected TokenEndpoint, got: {err:?}");
    };

    // Grep all renderings for the secret.
    let display = format!("{err}");
    let debug = format!("{err:?}");

    assert_no_secret_substring(summary, SECRET, "summary");
    assert_no_secret_substring(&display, SECRET, "Display");
    assert_no_secret_substring(&debug, SECRET, "Debug");

    // Structural diagnostics preserved.
    assert!(
        summary.contains("invalid_grant"),
        "structural error code preserved: {summary:?}"
    );
    assert!(
        summary.contains("[REDACTED]"),
        "redaction sentinel present: {summary:?}"
    );
    assert!(status.contains("400"), "status preserved: {status}");
}

// ============================================================================
// Row 2: defensive — access_token=<secret> in error_description
// ============================================================================

#[tokio::test]
async fn access_token_in_error_description_is_redacted() {
    const SECRET: &str = "xyz789-leaked-access-token-value";
    let err = parse_idp_error_response(
        400,
        "{\"error\":\"invalid_grant\",\"error_description\":\"access_token=xyz789-leaked-access-token-value revoked\"}",
    )
    .await;

    let TokenRefreshError::TokenEndpoint { summary, .. } = &err else {
        panic!("expected TokenEndpoint, got: {err:?}");
    };
    assert_no_secret_substring(summary, SECRET, "summary");
    assert_no_secret_substring(&format!("{err}"), SECRET, "Display");
    assert!(summary.contains("[REDACTED]"));
}

// ============================================================================
// Row 3: defensive — client_secret=<secret> in error_description
// ============================================================================

#[tokio::test]
async fn client_secret_in_error_description_is_redacted() {
    const SECRET: &str = "supersecretvalue123abc";
    let err = parse_idp_error_response(
        400,
        "{\"error\":\"invalid_client\",\"error_description\":\"client_secret=supersecretvalue123abc mismatch\"}",
    )
    .await;

    let TokenRefreshError::TokenEndpoint { summary, .. } = &err else {
        panic!("expected TokenEndpoint, got: {err:?}");
    };
    assert_no_secret_substring(summary, SECRET, "summary");
    assert!(summary.contains("[REDACTED]"));
}

// ============================================================================
// Row 4: case-insensitive match (RefreshToken vs refresh_token)
// ============================================================================

#[tokio::test]
async fn case_insensitive_key_match() {
    const SECRET: &str = "casevariantsecret456";
    let err = parse_idp_error_response(
        400,
        "{\"error\":\"invalid_grant\",\"error_description\":\"RefreshToken=casevariantsecret456 invalid\"}",
    )
    .await;

    let TokenRefreshError::TokenEndpoint { summary, .. } = &err else {
        panic!("expected TokenEndpoint, got: {err:?}");
    };
    assert_no_secret_substring(summary, SECRET, "summary");
}

// ============================================================================
// Row 5: regression — non-secret error_description passes through unchanged
// ============================================================================

#[tokio::test]
async fn non_secret_error_description_passes_through() {
    let err = parse_idp_error_response(
        400,
        "{\"error\":\"invalid_grant\",\"error_description\":\"the grant has expired\"}",
    )
    .await;

    let TokenRefreshError::TokenEndpoint { summary, .. } = &err else {
        panic!("expected TokenEndpoint, got: {err:?}");
    };
    assert!(
        summary.contains("the grant has expired"),
        "non-secret diagnostic preserved: {summary:?}"
    );
    assert!(
        !summary.contains("[REDACTED]"),
        "no spurious redaction on plain text: {summary:?}"
    );
}
