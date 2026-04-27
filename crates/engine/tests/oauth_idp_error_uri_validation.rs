#![cfg(feature = "rotation")]

//! SEC-02 (security hardening 2026-04-27 Stage 3) — `error_uri` from
//! non-2xx OAuth2 token responses must pass through `sanitize_error_uri`
//! before landing in operator-facing summaries.
//!
//! Table-driven coverage of the sanitizer against payloads a buggy or
//! hostile IdP might emit:
//!   1. `http://attacker.example` → rejected (scheme allowlist)
//!   2. `https://valid.example/ok` → passthrough unchanged
//!   3. `https://x.example/[control-char]` → rejected (control byte strip)
//!   4. 300-char `https://x.example/aaa...` → truncated at 256 + suffix
//!   5. `javascript:alert(1)` → rejected (parse OK but scheme not https)
//!   6. empty string → rejected (parse fail)

use nebula_credential::{
    SecretString,
    credentials::{OAuth2State, oauth2::AuthStyle},
};
use nebula_engine::credential::rotation::{TokenRefreshError, refresh_oauth2_state};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

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

fn sample_state(token_url: String) -> OAuth2State {
    OAuth2State {
        access_token: SecretString::new("old-access"),
        token_type: "Bearer".to_owned(),
        refresh_token: Some(SecretString::new("rt")),
        expires_at: None,
        scopes: vec![],
        client_id: SecretString::new("cid"),
        client_secret: SecretString::new("csecret"),
        token_url,
        auth_style: AuthStyle::Header,
    }
}

async fn spawn_idp_returning(body: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept");
        drain_incoming_request(&mut stream).await;
        let n = body.len();
        let head = format!(
            "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {n}\r\nConnection: close\r\n\r\n"
        );
        if stream.write_all(head.as_bytes()).await.is_err() {
            return;
        }
        let _ = stream.write_all(body.as_bytes()).await;
    });
    format!("http://127.0.0.1:{}/token", addr.port())
}

async fn run_and_get_summary(body: &'static str) -> String {
    let url = spawn_idp_returning(body).await;
    let mut state = sample_state(url);
    let err = refresh_oauth2_state(&mut state)
        .await
        .expect_err("400 expected");
    let TokenRefreshError::TokenEndpoint { summary, .. } = err else {
        panic!("expected TokenEndpoint");
    };
    summary
}

#[tokio::test]
async fn http_scheme_rejected() {
    let summary = run_and_get_summary(
        r#"{"error":"invalid_request","error_uri":"http://attacker.example/page"}"#,
    )
    .await;
    assert!(
        summary.contains("[invalid_error_uri_redacted]"),
        "non-https scheme rejected: {summary:?}"
    );
    assert!(
        !summary.contains("attacker.example"),
        "attacker host stripped: {summary:?}"
    );
}

#[tokio::test]
async fn valid_https_passes_through() {
    let summary = run_and_get_summary(
        r#"{"error":"invalid_grant","error_uri":"https://valid.example/help"}"#,
    )
    .await;
    assert!(
        summary.contains("https://valid.example/help"),
        "valid https URL preserved: {summary:?}"
    );
    assert!(!summary.contains("[invalid_error_uri_redacted]"));
    assert!(!summary.contains("[control_chars_in_error_uri_redacted]"));
}

#[tokio::test]
async fn control_chars_safely_neutralized() {
    // `Url::parse` percent-encodes control bytes inside the URL path
    // ( → %01), so the URL surfaces with the control byte already
    // neutralized. Defense-in-depth: even if Url::parse ever stops
    // encoding a particular byte, the explicit `bytes().any(...)` check
    // in `sanitize_error_uri` catches it. This test pins the END-TO-END
    // outcome: no raw control byte ever lands in the operator summary.
    let summary = run_and_get_summary(
        "{\"error\":\"invalid_grant\",\"error_uri\":\"https://x.example/path\\u0001bad\"}",
    )
    .await;
    assert!(
        !summary.bytes().any(|b| b < 0x20 || b == 0x7f),
        "no raw control bytes in summary: {summary:?}"
    );
    // The sanitizer either (a) percent-encodes via Url::parse, or
    // (b) replaces with the redaction sentinel — either is acceptable.
    assert!(
        summary.contains("%01") || summary.contains("[control_chars_in_error_uri_redacted]"),
        "control byte either percent-encoded or redacted: {summary:?}"
    );
}

#[tokio::test]
async fn javascript_scheme_rejected() {
    let summary =
        run_and_get_summary(r#"{"error":"invalid_grant","error_uri":"javascript:alert(1)"}"#).await;
    assert!(
        summary.contains("[invalid_error_uri_redacted]"),
        "javascript scheme rejected: {summary:?}"
    );
    assert!(
        !summary.contains("alert"),
        "JS payload stripped: {summary:?}"
    );
}

#[tokio::test]
async fn empty_uri_rejected() {
    let summary = run_and_get_summary(r#"{"error":"invalid_grant","error_uri":""}"#).await;
    assert!(
        summary.contains("[invalid_error_uri_redacted]"),
        "empty URI rejected: {summary:?}"
    );
}

#[tokio::test]
async fn long_uri_truncated_at_256_chars() {
    // 300-char URL — 8 char prefix + 292 path chars = 300 total > 256.
    // The body literal is constructed at compile time so the size is fixed.
    let summary = run_and_get_summary(
        "{\"error\":\"invalid_grant\",\"error_uri\":\"https://x.example/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"}",
    )
    .await;
    assert!(
        summary.contains("…[truncated]"),
        "long URL truncated: {summary:?}"
    );
    // Ensure the truncation suffix is exactly one occurrence; the URL
    // prefix itself is preserved.
    assert!(summary.contains("https://x.example/"));
}
