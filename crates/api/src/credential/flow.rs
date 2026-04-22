//! OAuth HTTP flow helpers for the API layer.

use std::time::Duration;

use futures::StreamExt;
use nebula_credential::credentials::oauth2::AuthStyle;
use serde::Deserialize;
use url::Url;

// ─── ADR-0031: bounded HTTP client for OAuth2 token exchange ─────────────────
//
// Policy names (timeouts, redirect cap, max body) and shared builder live here so
// token exchange and future refresh work share the same `reqwest` shape. Pointers:
// - Product canon: integration credential / OAuth and ADR-0031

/// Upper bound in bytes for the token endpoint **response body** before JSON parse.
/// OAuth token JSON is small; rejecting larger payloads bounds memory and avoids
/// misinterpreting a huge response as a valid token document.
pub const OAUTH_TOKEN_HTTP_MAX_RESPONSE_BYTES: usize = 256 * 1024;

const OAUTH_TOKEN_HTTP_MAX_REDIRECTS: usize = 5;
const OAUTH_TOKEN_HTTP_TIMEOUT: Duration = Duration::from_secs(30);
const OAUTH_TOKEN_HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Builds a [`reqwest::Client`] with ADR-0031 policy for the OAuth2 **token** endpoint
/// (authorization code exchange, and refresh when wired through the same stack).
/// Callers should not layer extra permissive policy on this client.
pub fn oauth_token_http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(OAUTH_TOKEN_HTTP_CONNECT_TIMEOUT)
        .timeout(OAUTH_TOKEN_HTTP_TIMEOUT)
        .redirect(reqwest::redirect::Policy::limited(
            OAUTH_TOKEN_HTTP_MAX_REDIRECTS,
        ))
        .build()
        .map_err(|e| format!("oauth client build failed: {e}"))
}

async fn read_token_response_limited(
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

/// Request parameters for authorization URI construction.
#[derive(Debug, Clone, Deserialize)]
pub struct AuthorizationUriRequest {
    /// OAuth authorization endpoint URL.
    pub auth_url: String,
    /// OAuth token endpoint URL (persisted for callback exchange).
    pub token_url: String,
    /// OAuth client identifier.
    pub client_id: String,
    /// OAuth client secret (persisted for callback exchange).
    pub client_secret: String,
    /// Redirect URI registered with provider.
    pub redirect_uri: String,
    /// Space-separated scopes.
    pub scopes: Option<String>,
    /// Client auth style for token endpoint.
    pub auth_style: Option<AuthStyle>,
}

/// Build OAuth2 Authorization Code URI with mandatory PKCE S256 parameters.
pub fn build_authorization_uri(
    req: &AuthorizationUriRequest,
    state: &str,
    code_challenge: &str,
) -> Result<Url, url::ParseError> {
    let mut url = Url::parse(&req.auth_url)?;
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("response_type", "code");
        q.append_pair("client_id", &req.client_id);
        q.append_pair("redirect_uri", &req.redirect_uri);
        q.append_pair("state", state);
        q.append_pair("code_challenge", code_challenge);
        q.append_pair("code_challenge_method", "S256");
        if let Some(scopes) = req.scopes.as_deref()
            && !scopes.trim().is_empty()
        {
            q.append_pair("scope", scopes);
        }
    }
    Ok(url)
}

/// Token endpoint exchange request.
#[derive(Debug, Clone, Deserialize)]
pub struct TokenExchangeRequest {
    /// OAuth token endpoint URL.
    pub token_url: String,
    /// OAuth client identifier.
    pub client_id: String,
    /// OAuth client secret.
    pub client_secret: String,
    /// Authorization code received from callback.
    pub code: String,
    /// Redirect URI to echo in token exchange.
    pub redirect_uri: String,
    /// PKCE verifier paired with `code_challenge`.
    pub code_verifier: String,
    /// Client auth style for token endpoint.
    #[serde(default)]
    pub auth_style: AuthStyle,
}

/// Exchange authorization code for tokens.
pub async fn exchange_code(req: &TokenExchangeRequest) -> Result<serde_json::Value, String> {
    let client = oauth_token_http_client()?;

    let mut form: Vec<(&str, &str)> = vec![
        ("grant_type", "authorization_code"),
        ("code", req.code.as_str()),
        ("redirect_uri", req.redirect_uri.as_str()),
        ("code_verifier", req.code_verifier.as_str()),
    ];

    let mut builder = client.post(&req.token_url);
    match req.auth_style {
        AuthStyle::Header => {
            builder = builder.basic_auth(&req.client_id, Some(&req.client_secret));
            builder = builder.form(&form);
        },
        AuthStyle::PostBody => {
            form.push(("client_id", req.client_id.as_str()));
            form.push(("client_secret", req.client_secret.as_str()));
            builder = builder.form(&form);
        },
    }

    let response = builder
        .send()
        .await
        .map_err(|e| format!("token exchange request failed: {e}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("token endpoint returned {status}"));
    }
    read_token_response_limited(response, OAUTH_TOKEN_HTTP_MAX_RESPONSE_BYTES).await
}

#[cfg(test)]
mod tests {
    use nebula_credential::credentials::oauth2::AuthStyle;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    use super::*;

    #[test]
    fn authorization_uri_contains_pkce_fields() {
        let req = AuthorizationUriRequest {
            auth_url: "https://provider.example.com/oauth/authorize".to_owned(),
            token_url: "https://provider.example.com/oauth/token".to_owned(),
            client_id: "client_123".to_owned(),
            client_secret: "secret_123".to_owned(),
            redirect_uri: "https://app.example.com/callback".to_owned(),
            scopes: Some("read write".to_owned()),
            auth_style: None,
        };

        let url = build_authorization_uri(&req, "signed_state", "code_challenge_123")
            .expect("auth url should build");
        let text = url.to_string();
        assert!(text.contains("code_challenge_method=S256"));
        assert!(text.contains("code_challenge=code_challenge_123"));
        assert!(text.contains("state=signed_state"));
    }

    /// Drain a single HTTP/1.1 request until the header block ends; enough for a POST with small
    /// form body.
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

    fn sample_exchange() -> TokenExchangeRequest {
        TokenExchangeRequest {
            token_url: String::new(), // set per test
            client_id: "client-id".to_owned(),
            client_secret: "client-secret".to_owned(),
            code: "auth-code".to_owned(),
            redirect_uri: "https://app.example.com/cb".to_owned(),
            code_verifier: "pkce-verifier".to_owned(),
            auth_style: AuthStyle::Header,
        }
    }

    #[tokio::test]
    async fn token_exchange_succeeds_for_small_response() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local addr");
        let body = br#"{"access_token":"t","token_type":"Bearer"}"#;
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            drain_incoming_request(&mut stream).await;
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(head.as_bytes()).await.expect("write head");
            stream.write_all(body).await.expect("write body");
        });

        let mut req = sample_exchange();
        req.token_url = format!("http://127.0.0.1:{}/token", addr.port());
        let val = exchange_code(&req)
            .await
            .expect("small token body should parse");
        assert_eq!(val["access_token"], "t");
    }

    #[tokio::test]
    async fn token_exchange_rejects_oversized_content_length() {
        let max = OAUTH_TOKEN_HTTP_MAX_RESPONSE_BYTES;
        let body_len = max + 1;

        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local addr");

        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            drain_incoming_request(&mut stream).await;
            // Body is never read: the client must fail closed on `Content-Length` alone.
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body_len
            );
            let _ = stream.write_all(head.as_bytes()).await;
        });

        let mut req = sample_exchange();
        req.token_url = format!("http://127.0.0.1:{}/token", addr.port());
        let err = exchange_code(&req)
            .await
            .expect_err("oversized Content-Length should fail");
        assert!(
            err.to_lowercase().contains("too large")
                || err.to_lowercase().contains("exceeded")
                || err.to_lowercase().contains("exceeds"),
            "expected size gate error, got: {err}"
        );
    }
}
