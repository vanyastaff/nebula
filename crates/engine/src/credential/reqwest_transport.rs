//! Reqwest-backed [`RefreshTransport`] implementation (ADR-0092).
//!
//! This is the **only** module in `nebula-engine` that links reqwest. It is a
//! dumb pipe: it performs the HTTP POST exactly as composed by the credential
//! crate and returns raw `(status, bounded-body)`. It does NO url validation,
//! NO body parsing, NO secret interpretation.
//!
//! # Security responsibilities (NOT this module's)
//!
//! | Concern | Owner |
//! |---------|-------|
//! | SSRF endpoint validation | `nebula-credential` (`validate_token_endpoint`, runs before `post_token`) |
//! | OAuth2 form composition + `AuthStyle` placement | `nebula-credential` |
//! | Response status + body parse + SEC-02 redaction | `nebula-credential` |
//! | SEC-01 body cap policy value | `nebula-credential` (`OAUTH_TOKEN_HTTP_MAX_RESPONSE_BYTES`) |
//! | SEC-01 body cap mechanical enforcement | **this module** (reads at most `req.max_response_bytes`) |
//!
//! The connect-time private-IP blocking (DNS-rebind defence-in-depth) is a
//! SHOULD per the transport module doc. The current implementation relies on
//! the credential-side pre-call string check; a hardened variant with a custom
//! DNS resolver is tracked separately.

use std::{future::Future, sync::OnceLock, time::Duration};

use futures::StreamExt as _;
use nebula_credential::runtime::{
    RefreshTransport, RefreshTransportError, TokenPostRequest, TokenPostResponse,
};

const OAUTH_TOKEN_HTTP_MAX_REDIRECTS: usize = 5;
const OAUTH_TOKEN_HTTP_TIMEOUT: Duration = Duration::from_secs(30);
const OAUTH_TOKEN_HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

static OAUTH_TOKEN_HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

/// Returns the process-wide OAuth2 token HTTP client (connection-pooled,
/// TLS-session-reuse).
///
/// # Panics
///
/// Panics if `reqwest::Client::builder().build()` fails. This cannot occur
/// with the default TLS backend (rustls) on supported platforms; a panic here
/// means the TLS stack is broken at process startup and the process cannot
/// operate.
fn oauth_token_http_client() -> &'static reqwest::Client {
    OAUTH_TOKEN_HTTP_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(OAUTH_TOKEN_HTTP_CONNECT_TIMEOUT)
            .timeout(OAUTH_TOKEN_HTTP_TIMEOUT)
            .redirect(reqwest::redirect::Policy::limited(
                OAUTH_TOKEN_HTTP_MAX_REDIRECTS,
            ))
            .build()
            .expect("nebula: OAuth token HTTP client must build — TLS stack unavailable")
    })
}

/// Reqwest-backed [`RefreshTransport`].
///
/// Stateless: all configuration is baked into the process-wide
/// `reqwest::Client`. Construct via [`ReqwestRefreshTransport::default`].
#[derive(Debug, Default)]
pub struct ReqwestRefreshTransport;

impl RefreshTransport for ReqwestRefreshTransport {
    fn post_token<'a>(
        &'a self,
        req: TokenPostRequest,
    ) -> std::pin::Pin<
        Box<dyn Future<Output = Result<TokenPostResponse, RefreshTransportError>> + Send + 'a>,
    > {
        Box::pin(async move {
            let client = oauth_token_http_client();

            // Build form pairs: expose SecretString values only long enough for
            // reqwest to copy them into its (non-zeroizing) serialized body.
            // The borrows drop at the end of this block; the request builder
            // owns the serialized bytes from here on.
            let form_pairs: Vec<(&str, &str)> = req
                .form
                .iter()
                .map(|(k, v)| (k.as_str(), v.expose_secret()))
                .collect();

            let mut builder = client.post(&req.url).form(&form_pairs);

            if let Some((user, pass)) = &req.basic_auth {
                builder = builder.basic_auth(user, Some(pass.expose_secret()));
            }

            let response = builder
                .send()
                .await
                .map_err(|e| RefreshTransportError::Send(e.to_string()))?;

            let status = response.status().as_u16();
            let body = read_bounded(response, req.max_response_bytes)
                .await
                .map_err(|e| RefreshTransportError::ReadBody(e.to_string()))?;

            Ok(TokenPostResponse { status, body })
        })
    }
}

/// Read a response body up to `max_bytes`, returning the bytes collected.
///
/// Enforces [`TokenPostRequest::max_response_bytes`] (SEC-01). On
/// `Content-Length` pre-flight rejection or stream truncation the function
/// returns an `Err` — the credential side maps this to a refresh error.
async fn read_bounded(
    response: reqwest::Response,
    max_bytes: usize,
) -> Result<Vec<u8>, ReadBoundedError> {
    // Pre-flight Content-Length check (fast reject before reading stream).
    if let Some(claimed) = response.content_length() {
        let max = u64::try_from(max_bytes).unwrap_or(u64::MAX);
        if claimed > max {
            return Err(ReadBoundedError::ContentLengthTooLarge {
                claimed,
                max: max_bytes,
            });
        }
    }

    let mut buf = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(ReadBoundedError::Read)?;
        if buf.len().saturating_add(chunk.len()) > max_bytes {
            return Err(ReadBoundedError::BodyTooLarge { max: max_bytes });
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(buf)
}

#[derive(Debug, thiserror::Error)]
enum ReadBoundedError {
    #[error("token response too large: Content-Length {claimed} (max {max} bytes)")]
    ContentLengthTooLarge { claimed: u64, max: usize },
    #[error("token response body exceeded {max} bytes")]
    BodyTooLarge { max: usize },
    #[error("read token response body: {0}")]
    Read(#[source] reqwest::Error),
}
