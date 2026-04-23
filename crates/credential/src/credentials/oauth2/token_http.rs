//! ADR-0031: bounded HTTP client and body handling for OAuth2 **token** endpoints
//! (code exchange, refresh, client credentials, device code poll to `token_url`).
//!
//! The same `reqwest` policy and max-body guard must apply everywhere token JSON is
//! accepted as credentials (see product canon, ADR-0031). Shared by
//! `nebula-credential`’s `flow`, `nebula-api`’s callback exchange, and
//! `nebula-engine` refresh.

use std::time::Duration;

use futures::StreamExt;

/// Upper bound in bytes for the token endpoint **response body** before JSON parse.
/// OAuth token JSON is small; rejecting larger payloads bounds memory and avoids
/// misinterpreting a huge response as a valid token document.
pub const OAUTH_TOKEN_HTTP_MAX_RESPONSE_BYTES: usize = 256 * 1024;

const OAUTH_TOKEN_HTTP_MAX_REDIRECTS: usize = 5;
const OAUTH_TOKEN_HTTP_TIMEOUT: Duration = Duration::from_secs(30);
const OAUTH_TOKEN_HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Builds a [`reqwest::Client`] with ADR-0031 policy for OAuth2 **token** calls.
/// Callers should not apply more permissive redirect/timeout/body policy on this client.
pub fn oauth_token_http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(OAUTH_TOKEN_HTTP_CONNECT_TIMEOUT)
        .timeout(OAUTH_TOKEN_HTTP_TIMEOUT)
        .redirect(reqwest::redirect::Policy::limited(
            OAUTH_TOKEN_HTTP_MAX_REDIRECTS,
        ))
        .build()
        .map_err(|e| format!("oauth token http client build failed: {e}"))
}

/// Read a **successful** (2xx) token response body up to `max_bytes` and parse JSON.
pub async fn read_token_response_limited(
    response: reqwest::Response,
    max_bytes: usize,
) -> Result<serde_json::Value, String> {
    if let Some(claimed) = response.content_length()
        && claimed > u64::try_from(max_bytes).unwrap_or(u64::MAX)
    {
        return Err(format!(
            "token response too large: Content-Length {claimed} (max {max_bytes} bytes)"
        ));
    }

    let mut buf = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("read token response body: {e}"))?;
        if buf.len().saturating_add(chunk.len()) > max_bytes {
            return Err(format!("token response body exceeded {max_bytes} bytes"));
        }
        buf.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&buf).map_err(|e| format!("token response parse failed: {e}"))
}

#[cfg(test)]
mod tests {
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    use super::*;

    /// Drain a single HTTP/1.1 request until the header block ends.
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

    #[tokio::test]
    async fn read_succeeds_for_small_response() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local addr");
        let body = br#"{"access_token":"t","token_type":"Bearer"}"#;
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            drain_incoming_request(&mut stream).await;
            let n = body.len();
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {n}\r\nConnection: close\r\n\r\n"
            );
            let _ = stream.write_all(head.as_bytes()).await;
            let _ = stream.write_all(body).await;
        });

        let client = oauth_token_http_client().expect("client");
        let response = client
            .get(format!("http://{addr}/"))
            .send()
            .await
            .expect("send");
        let val = read_token_response_limited(response, OAUTH_TOKEN_HTTP_MAX_RESPONSE_BYTES)
            .await
            .expect("body should parse");
        assert_eq!(val["access_token"], "t");
    }

    #[tokio::test]
    async fn read_rejects_oversized_content_length() {
        let max = OAUTH_TOKEN_HTTP_MAX_RESPONSE_BYTES;
        let body_len = max + 1;

        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local addr");

        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            drain_incoming_request(&mut stream).await;
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {body_len}\r\nConnection: close\r\n\r\n"
            );
            let _ = stream.write_all(head.as_bytes()).await;
        });

        let client = oauth_token_http_client().expect("client");
        let response = client
            .get(format!("http://{addr}/"))
            .send()
            .await
            .expect("send");
        let err = read_token_response_limited(response, max)
            .await
            .expect_err("oversized Content-Length should fail");
        let lower = err.to_lowercase();
        assert!(
            lower.contains("too large") || lower.contains("exceeded") || lower.contains("exceeds"),
            "expected size gate error, got: {err}"
        );
    }

    #[tokio::test]
    async fn read_rejects_oversized_streaming_body_without_content_length() {
        let max = OAUTH_TOKEN_HTTP_MAX_RESPONSE_BYTES;
        let one_chunk = max + 1;

        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local addr");

        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            drain_incoming_request(&mut stream).await;
            const HEAD: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
Transfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
            if stream.write_all(HEAD).await.is_err() {
                return;
            }
            let size_line = format!("{one_chunk:x}\r\n");
            if stream.write_all(size_line.as_bytes()).await.is_err() {
                return;
            }
            if stream.write_all(&vec![b'x'; one_chunk]).await.is_err() {
                return;
            }
            let _ = stream.write_all(b"\r\n0\r\n\r\n").await;
        });

        let client = oauth_token_http_client().expect("client");
        let response = client
            .get(format!("http://{addr}/"))
            .send()
            .await
            .expect("send");
        let err = read_token_response_limited(response, max)
            .await
            .expect_err("streaming body over max should fail");
        let lower = err.to_lowercase();
        assert!(
            lower.contains("exceeded"),
            "expected streaming cap (exceeded) error, got: {err}"
        );
    }
}
