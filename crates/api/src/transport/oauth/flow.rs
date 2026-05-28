//! OAuth HTTP flow helpers for the API layer.
//!
//! API-owned OAuth flow: the API layer owns the OAuth flow HTTP ceremony (auth URI
//! construction, code→token exchange). Token endpoint policy and bounded
//! body reads live in the sibling [`super::http`] module.

use std::net::IpAddr;

use nebula_credential::AuthStyle;
use serde::Deserialize;
use url::{Host, Url};

pub use super::http::{
    OAUTH_TOKEN_HTTP_MAX_RESPONSE_BYTES, TokenHttpError, oauth_token_http_client,
    read_token_response_limited,
};

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
    validate_oauth_outbound_url(&req.token_url)?;
    exchange_code_unchecked(req).await
}

// `pub(crate)` so the test-only `crate::test_support` module (gated by
// `#[cfg(nebula_test_util)]`) can `pub use` this for integration tests
// against localhost wiremock IdPs per ADR-0085 D-14. Production builds
// without the cfg never compile `test_support`, so the symbol stays
// effectively private to flow.rs internals.
pub(crate) async fn exchange_code_unchecked(
    req: &TokenExchangeRequest,
) -> Result<serde_json::Value, String> {
    let client = oauth_token_http_client();

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
    read_token_response_limited(response, OAUTH_TOKEN_HTTP_MAX_RESPONSE_BYTES)
        .await
        .map_err(|e| e.to_string())
}

/// Validate that an OAuth outbound (server-side fetched) URL is safe.
///
/// API callers control these URLs via operator config or via OIDC
/// discovery doc parsing. This strict gate rejects non-HTTPS schemes
/// and obvious loopback / private / link-local / multicast hosts
/// before `reqwest` can reach internal services (anti-SSRF).
///
/// Per ADR-0085 D-9-WAVE6, this strict gate applies to:
/// - Token endpoint POST.
/// - Userinfo endpoint GET.
/// - GitHub-style verified-emails endpoint GET (per D-5 wave-6).
/// - JWKS endpoint (D-16 deferred but URL still validated when present).
/// - OIDC discovery doc GET, AND each URL returned in the discovery
///   response before the cache insert (D-15-WAVE6).
///
/// The browser-fetched authorize URL goes through the **flag-aware**
/// [`validate_oauth_authorize_url`] instead, which respects
/// `oauth_allow_insecure_localhost` for dev convenience without
/// weakening server-side anti-SSRF.
///
/// Renamed from `validate_token_endpoint` in wave-6 to signal the
/// generalized scope. A `validate_token_endpoint` shim is kept below
/// for backward source-compat during the PR-2..PR-4 transition; new
/// callers should use `validate_oauth_outbound_url` directly.
pub fn validate_oauth_outbound_url(raw: &str) -> Result<(), String> {
    let url = Url::parse(raw).map_err(|e| format!("invalid OAuth outbound URL: {e}"))?;
    if url.scheme() != "https" {
        return Err("OAuth outbound URL must use https".to_owned());
    }

    validate_token_endpoint_host(url.host())?;
    Ok(())
}

/// Deprecated alias — use [`validate_oauth_outbound_url`].
///
/// Kept as a shim during the PR-2..PR-4 wave-6/-7 transition so existing
/// call sites compile while they migrate. New code MUST call
/// `validate_oauth_outbound_url` directly.
#[deprecated(
    since = "0.1.0",
    note = "renamed to validate_oauth_outbound_url per ADR-0085 D-9-WAVE6; the rename signals the gate covers ALL server-side OAuth fetches, not just the token endpoint"
)]
pub fn validate_token_endpoint(raw: &str) -> Result<(), String> {
    validate_oauth_outbound_url(raw)
}

/// Flag-aware validator for the **browser-fetched** OAuth authorize URL.
///
/// Per ADR-0085 D-9-WAVE6 wave-7 split (Codex F.2): the authorize URL
/// is fetched by the user's browser, NOT the server, so it has no
/// SSRF surface. To enable localhost-IdP dev workflows (e.g. wiremock
/// on `127.0.0.1` serving the authorize page), this validator accepts
/// `http://localhost(:port)?(/.*)?` when the operator opts in via the
/// `oauth_allow_insecure_localhost` flag AND the binary is NOT a
/// release build (`debug_assertions` enabled).
///
/// Production builds (`!cfg!(debug_assertions)`) reject the relaxation
/// regardless of the flag, mirroring the `nebula_test_util` cfg's
/// release-build guard in `crates/api/src/lib.rs`.
///
/// This gate is intended ONLY for `OAuthEndpoints::Manual.authorize_url`
/// (static operator config) and the dynamic `authorize_url` returned
/// inside an [`crate::config::OAuthEndpoints::Oidc`] discovery
/// response. All other URL fields must go through the strict
/// [`validate_oauth_outbound_url`].
pub fn validate_oauth_authorize_url(
    raw: &str,
    oauth_allow_insecure_localhost: bool,
    in_release_build: bool,
) -> Result<(), String> {
    let url = Url::parse(raw).map_err(|e| format!("invalid OAuth authorize URL: {e}"))?;

    let scheme = url.scheme();
    let is_localhost_http = scheme == "http"
        && matches!(url.host(), Some(Host::Domain(h)) if h.eq_ignore_ascii_case("localhost"));

    if scheme == "https" {
        validate_token_endpoint_host(url.host())?;
        return Ok(());
    }

    if is_localhost_http && oauth_allow_insecure_localhost && !in_release_build {
        // Browser-fetched URL on localhost during dev/integration runs.
        // No SSRF surface; explicit operator opt-in.
        return Ok(());
    }

    if is_localhost_http && in_release_build {
        return Err(
            "OAuth authorize URL: oauth_allow_insecure_localhost has no effect in release builds"
                .to_owned(),
        );
    }

    Err("OAuth authorize URL must use https (or http://localhost when oauth_allow_insecure_localhost = true in dev builds)".to_owned())
}

fn validate_token_endpoint_host(host: Option<Host<&str>>) -> Result<(), String> {
    match host.ok_or_else(|| "OAuth token endpoint must include a host".to_owned())? {
        Host::Domain(host) if host.eq_ignore_ascii_case("localhost") => {
            Err("OAuth token endpoint must not target localhost".to_owned())
        },
        Host::Domain(_) => Ok(()),
        Host::Ipv4(ip) if forbidden_token_endpoint_ip(IpAddr::V4(ip)) => {
            Err("OAuth token endpoint must not target private or local addresses".to_owned())
        },
        Host::Ipv4(_) => Ok(()),
        Host::Ipv6(ip) if forbidden_token_endpoint_ip(IpAddr::V6(ip)) => {
            Err("OAuth token endpoint must not target private or local addresses".to_owned())
        },
        Host::Ipv6(_) => Ok(()),
    }
}

fn forbidden_token_endpoint_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_unspecified()
                || ip.is_broadcast()
        },
        IpAddr::V6(ip) => {
            if let Some(mapped) = ip.to_ipv4_mapped() {
                return forbidden_token_endpoint_ip(IpAddr::V4(mapped));
            }
            let first = ip.segments()[0];
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || matches!(first & 0xfe00, 0xfc00)
                || matches!(first & 0xffc0, 0xfe80)
                || matches!(first & 0xffc0, 0xfec0)
        },
    }
}

#[cfg(test)]
mod tests {
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
    async fn token_exchange_rejects_loopback_token_url() {
        let mut req = sample_exchange();
        req.token_url = "http://127.0.0.1:1/token".to_owned();

        let err = exchange_code(&req)
            .await
            .expect_err("loopback token URLs must fail before any request");
        // Error message updated in PR-2 from "token endpoint" to
        // "outbound URL" after the validate_token_endpoint ->
        // validate_oauth_outbound_url rename (ADR-0085 D-9-WAVE6
        // generalized the gate to all server-side OAuth fetches).
        let lower = err.to_lowercase();
        assert!(
            lower.contains("outbound url") || lower.contains("must use https"),
            "expected endpoint validation error, got: {err}"
        );
    }

    #[test]
    fn token_endpoint_rejects_ipv4_mapped_ipv6_private_addresses() {
        for raw in [
            "https://[::ffff:7f00:1]/token",
            "https://[::ffff:a00:1]/token",
            "https://[::ffff:a9fe:1]/token",
            "https://[ff02::1]/token",
            "https://[fec0::1]/token",
        ] {
            let err = validate_oauth_outbound_url(raw)
                .expect_err("private IPv4-mapped and local IPv6 addresses must be rejected");
            // PR-2: error message text changed with the rename per D-9-WAVE6.
            let lower = err.to_lowercase();
            assert!(
                lower.contains("outbound url")
                    || lower.contains("private or local")
                    || lower.contains("must include a host"),
                "expected endpoint validation error for {raw}, got: {err}"
            );
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
            let n = body.len();
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {n}\r\nConnection: close\r\n\r\n"
            );
            stream.write_all(head.as_bytes()).await.expect("write head");
            stream.write_all(body).await.expect("write body");
        });

        let mut req = sample_exchange();
        req.token_url = format!("http://127.0.0.1:{}/token", addr.port());
        let val = exchange_code_unchecked(&req)
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
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {body_len}\r\nConnection: close\r\n\r\n"
            );
            let _ = stream.write_all(head.as_bytes()).await;
        });

        let mut req = sample_exchange();
        req.token_url = format!("http://127.0.0.1:{}/token", addr.port());
        let err = exchange_code_unchecked(&req)
            .await
            .expect_err("oversized Content-Length should fail");
        assert!(
            err.to_lowercase().contains("too large")
                || err.to_lowercase().contains("exceeded")
                || err.to_lowercase().contains("exceeds"),
            "expected size gate error, got: {err}"
        );
    }

    /// `Content-Length` missing: `bytes_stream()` must still cap the body (e.g. chunked).
    #[tokio::test]
    async fn token_exchange_rejects_oversized_streaming_body_without_content_length() {
        let max = OAUTH_TOKEN_HTTP_MAX_RESPONSE_BYTES;
        let one_chunk = max + 1;
        // Single chunk, HTTP/1.1 chunked, no `Content-Length`.
        // Body bytes are never parsed as successful JSON: size gate fails first.
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

        let mut req = sample_exchange();
        req.token_url = format!("http://127.0.0.1:{}/token", addr.port());
        let err = exchange_code_unchecked(&req)
            .await
            .expect_err("streaming body over max should fail");
        let lower = err.to_lowercase();
        assert!(
            lower.contains("exceeded"),
            "expected streaming cap (exceeded) error, got: {err}"
        );
    }

    #[tokio::test]
    async fn token_exchange_fails_on_non_success_status() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local addr");
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            drain_incoming_request(&mut stream).await;
            const BODY: &[u8] = b"{\"error\":\"invalid_client\",\"error_description\":\"bad\"}";
            let n = BODY.len();
            let head = format!(
                "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: {n}\r\nConnection: close\r\n\r\n"
            );
            if stream.write_all(head.as_bytes()).await.is_err() {
                return;
            }
            let _ = stream.write_all(BODY).await;
        });

        let mut req = sample_exchange();
        req.token_url = format!("http://127.0.0.1:{}/token", addr.port());
        let err = exchange_code_unchecked(&req)
            .await
            .expect_err("401 from token endpoint should map to error");
        assert!(
            err.contains("401") || err.to_lowercase().contains("unauthorized"),
            "expected non-success status in error, got: {err}"
        );
    }

    #[tokio::test]
    async fn token_exchange_fails_on_invalid_json_body_for_200() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local addr");
        let body: &[u8] = b"this is not json {";
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            drain_incoming_request(&mut stream).await;
            let n = body.len();
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {n}\r\nConnection: close\r\n\r\n"
            );
            if stream.write_all(head.as_bytes()).await.is_err() {
                return;
            }
            let _ = stream.write_all(body).await;
        });

        let mut req = sample_exchange();
        req.token_url = format!("http://127.0.0.1:{}/token", addr.port());
        let err = exchange_code_unchecked(&req)
            .await
            .expect_err("invalid json on 2xx should fail");
        let lower = err.to_lowercase();
        assert!(
            lower.contains("parse") || lower.contains("json") || lower.contains("token response"),
            "expected JSON parse / token response error, got: {err}"
        );
    }
}
