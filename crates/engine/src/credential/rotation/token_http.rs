//! ADR-0031: bounded HTTP client and body handling for OAuth2 **token** endpoints.
//!
//! Moved from `nebula-credential` per ADR-0031 incremental split: the engine
//! owns token refresh HTTP transport (ADR-0030).

use std::{sync::OnceLock, time::Duration};

use futures::StreamExt;
use thiserror::Error;

/// Upper bound in bytes for the token endpoint **response body** before JSON parse.
/// OAuth token JSON is small; rejecting larger payloads bounds memory and avoids
/// misinterpreting a huge response as a valid token document.
pub const OAUTH_TOKEN_HTTP_MAX_RESPONSE_BYTES: usize = 256 * 1024;

const OAUTH_TOKEN_HTTP_MAX_REDIRECTS: usize = 5;
const OAUTH_TOKEN_HTTP_TIMEOUT: Duration = Duration::from_secs(30);
const OAUTH_TOKEN_HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

static OAUTH_TOKEN_HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

/// Failures from reading or parsing a bounded 2xx token response body.
#[derive(Debug, Error)]
pub enum TokenHttpError {
    /// `Content-Length` exceeds the configured maximum.
    #[error("token response too large: Content-Length {claimed} (max {max} bytes)")]
    ContentLengthTooLarge {
        /// `Content-Length` value in bytes
        claimed: u64,
        /// Configured cap in bytes
        max: usize,
    },
    /// The response body exceeded the maximum before JSON parse.
    #[error("token response body exceeded {max} bytes")]
    BodyTooLarge {
        /// Configured cap in bytes
        max: usize,
    },
    /// Reading the streaming body failed.
    #[error("read token response body: {0}")]
    ReadChunk(#[source] reqwest::Error),
    /// JSON in the (already bounded) buffer is not valid.
    #[error("token response parse failed: {0}")]
    Json(#[source] serde_json::Error),
}

/// Returns a shared [`reqwest::Client`] with ADR-0031 policy for OAuth2 **token** calls
/// (one process-wide instance for connection pooling and TLS session reuse).
pub fn oauth_token_http_client() -> &'static reqwest::Client {
    OAUTH_TOKEN_HTTP_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(OAUTH_TOKEN_HTTP_CONNECT_TIMEOUT)
            .timeout(OAUTH_TOKEN_HTTP_TIMEOUT)
            .redirect(reqwest::redirect::Policy::limited(
                OAUTH_TOKEN_HTTP_MAX_REDIRECTS,
            ))
            .build()
            .expect("nebula: oauth token http client (ADR-0031 static policy) must build")
    })
}

/// Read a **successful** (2xx) token response body up to `max_bytes` and parse JSON.
pub async fn read_token_response_limited(
    response: reqwest::Response,
    max_bytes: usize,
) -> Result<serde_json::Value, TokenHttpError> {
    if let Some(claimed) = response.content_length()
        && claimed > u64::try_from(max_bytes).unwrap_or(u64::MAX)
    {
        return Err(TokenHttpError::ContentLengthTooLarge {
            claimed,
            max: max_bytes,
        });
    }

    let mut buf = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(TokenHttpError::ReadChunk)?;
        if buf.len().saturating_add(chunk.len()) > max_bytes {
            return Err(TokenHttpError::BodyTooLarge { max: max_bytes });
        }
        buf.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&buf).map_err(TokenHttpError::Json)
}
