#![cfg(feature = "rotation")]

//! SEC-01 (security hardening 2026-04-27 Stage 3) — bounded reader on the
//! OAuth2 token endpoint error path.
//!
//! Previously `parse_token_response` called `resp.text().await` unbounded
//! when the IdP returned non-2xx. A compromised / MITM IdP could push
//! hundreds of MB of error body before the 30s transport timeout, causing
//! memory pressure on the engine. The fix uses
//! `read_token_response_text_limited` with the 256 KiB cap.
//!
//! This test injects a 1 MiB error body and asserts the engine fails fast
//! with a sized error rather than running into the 30s timeout or OOM.

use std::time::{Duration, Instant};

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

#[tokio::test]
async fn oversized_error_body_returns_bounded_failure_fast() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");

    // 1 MiB of `a`s as the error body — well above the 256 KiB cap.
    const BODY_LEN: usize = 1024 * 1024;
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept");
        drain_incoming_request(&mut stream).await;
        let head = format!(
            "HTTP/1.1 400 Bad Request\r\nContent-Type: text/plain\r\nContent-Length: {BODY_LEN}\r\nConnection: close\r\n\r\n"
        );
        if stream.write_all(head.as_bytes()).await.is_err() {
            return;
        }
        // Stream chunks rather than allocating 1 MiB upfront on the test side.
        let chunk = vec![b'a'; 4096];
        for _ in 0..(BODY_LEN / 4096) {
            if stream.write_all(&chunk).await.is_err() {
                return;
            }
        }
    });

    let mut state = sample_state(format!("http://127.0.0.1:{}/token", addr.port()));
    let started = Instant::now();
    let err = refresh_oauth2_state(&mut state)
        .await
        .expect_err("400 expected");
    let elapsed = started.elapsed();

    // The bounded reader fails fast (well under the 30s transport timeout).
    assert!(
        elapsed < Duration::from_secs(10),
        "bounded reader should fail fast, elapsed: {elapsed:?}"
    );

    // Maps to TokenEndpoint with the bounded-read failure embedded in summary.
    let TokenRefreshError::TokenEndpoint { summary, status } = &err else {
        panic!("expected TokenEndpoint, got: {err:?}");
    };
    assert!(status.contains("400"), "status preserved: {status}");
    assert!(
        summary.contains("failed to read token endpoint error body"),
        "summary captures bounded-read failure: {summary:?}"
    );
}

#[tokio::test]
async fn oversized_content_length_rejected_before_read() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");

    // Advertise a 10 MiB Content-Length but never send any bytes — the
    // bounded reader should reject on the Content-Length check before
    // touching the network read loop.
    const CLAIMED: u64 = 10 * 1024 * 1024;
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept");
        drain_incoming_request(&mut stream).await;
        let head = format!(
            "HTTP/1.1 400 Bad Request\r\nContent-Type: text/plain\r\nContent-Length: {CLAIMED}\r\nConnection: close\r\n\r\n"
        );
        if stream.write_all(head.as_bytes()).await.is_err() {
            return;
        }
        // Send only one byte — Content-Length lies, but the reader's
        // pre-flight check on Content-Length should already have rejected.
        let _ = stream.write_all(b"x").await;
    });

    let mut state = sample_state(format!("http://127.0.0.1:{}/token", addr.port()));
    let err = refresh_oauth2_state(&mut state)
        .await
        .expect_err("400 expected");
    let TokenRefreshError::TokenEndpoint { summary, .. } = &err else {
        panic!("expected TokenEndpoint, got: {err:?}");
    };
    assert!(
        summary.contains("failed to read token endpoint error body"),
        "Content-Length cap surfaces as bounded-read failure: {summary:?}"
    );
}
