//! OIDC discovery doc fetcher + process-lifetime cache.
//!
//! Per ADR-0085 D-15-WAVE6: when an OAuth provider declares
//! `endpoints = { kind = "oidc", discovery_url = "..." }` in operator
//! config, Nebula resolves the real endpoints (authorize / token /
//! userinfo / jwks) by fetching the IdP's
//! `.well-known/openid-configuration` document at first
//! `start_oauth` time and caches the result for the process lifetime
//! (single-process cache; multi-replica deployments re-fetch per
//! replica, which is fine — discovery docs change rarely and a stale
//! cache is upper-bounded by process restart cadence).
//!
//! ## Anti-SSRF gates (D-9-WAVE6 + F.2 wave-7 split)
//!
//! `fetch_oidc_discovery` runs the strict
//! [`validate_oauth_outbound_url`] gate on:
//! - The `discovery_url` itself (server-side GET).
//! - Each child URL RETURNED in the parsed discovery JSON
//!   (`token_url` / `userinfo_url` / `jwks_url` when present) before
//!   the cache insert.
//!
//! And the flag-aware [`validate_oauth_authorize_url`] gate on the
//! discovery-returned `authorize_url` (browser-fetched; honors the
//! `oauth_allow_insecure_localhost` flag in dev builds only).
//!
//! ANY child URL rejection skips the cache insert and returns
//! [`DiscoveryError::EndpointSsrfRejected`] — no partial cache
//! entries.

use std::sync::OnceLock;
use std::time::Duration;

use dashmap::DashMap;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::flow::{validate_oauth_authorize_url, validate_oauth_outbound_url};

/// Time budget for the discovery doc GET. Matches the token-endpoint
/// 5000ms default per REQ-obs-001; the discovery doc is small JSON
/// and shouldn't take longer.
const DISCOVERY_FETCH_TIMEOUT: Duration = Duration::from_secs(5);

/// Response body cap for OIDC discovery (256 KiB).
///
/// Mirrors [`super::http::OAUTH_TOKEN_HTTP_MAX_RESPONSE_BYTES`] so a
/// hostile / misconfigured IdP cannot DoS the server by serving an
/// unbounded JSON blob through the discovery channel. Real discovery
/// docs are small (~2-10 KiB).
const DISCOVERY_MAX_RESPONSE_BYTES: usize = 256 * 1024;

/// HTTP client used ONLY for OIDC discovery fetches.
///
/// Separate from [`super::http::oauth_token_http_client`] because
/// discovery has a stricter posture:
/// - **Redirects disabled** (`Policy::none()`): a valid HTTPS
///   `discovery_url` cannot be allowed to redirect Nebula to
///   `http://localhost` / a private IP. Without redirects there is
///   no post-validation TOCTOU surface (Copilot wave-1 review).
/// - Same connect / read timeouts as the token client.
static OAUTH_DISCOVERY_HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

fn oauth_discovery_http_client() -> &'static reqwest::Client {
    OAUTH_DISCOVERY_HTTP_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(DISCOVERY_FETCH_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("nebula: oauth discovery http client must build")
    })
}

/// Parsed `.well-known/openid-configuration` document. Only the four
/// fields Plane A consumes are deserialized; unknown fields are
/// ignored (per the OIDC discovery spec which says clients SHOULD
/// gracefully handle additional metadata).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OidcDiscovery {
    /// OAuth authorize endpoint. Browser-fetched at `start_oauth`
    /// time. Validated by the flag-aware gate per F.2 wave-7.
    #[serde(rename = "authorization_endpoint")]
    pub authorize_url: String,
    /// OAuth token endpoint. Server-fetched at `complete_oauth` time.
    /// Validated by the strict gate.
    #[serde(rename = "token_endpoint")]
    pub token_url: String,
    /// Userinfo endpoint. Server-fetched at `complete_oauth` time
    /// with `Authorization: Bearer <access_token>`. Authoritative for
    /// `(email, sub)` per ADR-0085 D-16 (id_token JWKS validation
    /// deferred to 1.1). Validated by the strict gate.
    #[serde(rename = "userinfo_endpoint")]
    pub userinfo_url: String,
    /// Optional JWKS URL. Validated by the strict gate when present.
    /// Accepted for 1.1 forward compat; PR-4 logs id_token presence
    /// only and does NOT verify signatures in 1.0.
    #[serde(default, rename = "jwks_uri")]
    pub jwks_url: Option<String>,
}

/// Process-wide cache keyed by the operator-configured `discovery_url`.
/// Single-process scope — multi-replica deployments re-fetch per
/// replica on first use. Stale-cache risk is bounded by process
/// restart cadence; discovery docs change rarely. No TTL: a cached
/// entry lives until the process restarts.
static DISCOVERY_CACHE: OnceLock<DashMap<String, OidcDiscovery>> = OnceLock::new();

fn cache() -> &'static DashMap<String, OidcDiscovery> {
    DISCOVERY_CACHE.get_or_init(DashMap::new)
}

/// Failure modes for [`fetch_oidc_discovery`].
#[derive(Debug, Error)]
pub enum DiscoveryError {
    /// Strict gate rejected the `discovery_url` before any HTTP call.
    /// Caller typo or unsafe operator config — operator must fix
    /// `API_AUTH_OAUTH_<PROVIDER>_DISCOVERY_URL`.
    #[error("OIDC discovery URL `{url}` rejected by anti-SSRF gate: {reason}")]
    DiscoveryUrlRejected {
        /// The rejected URL.
        url: String,
        /// Reason from `validate_oauth_outbound_url`.
        reason: String,
    },
    /// Network or HTTP-level failure during the discovery doc GET.
    #[error("OIDC discovery fetch failed for `{url}`: {source}")]
    HttpError {
        /// The URL being fetched.
        url: String,
        /// Underlying `reqwest` error.
        #[source]
        source: reqwest::Error,
    },
    /// Non-200 response from the IdP discovery endpoint.
    #[error("OIDC discovery `{url}` returned HTTP {status}")]
    NonSuccessStatus {
        /// The URL being fetched.
        url: String,
        /// HTTP status code.
        status: u16,
    },
    /// Response body did not deserialize to [`OidcDiscovery`].
    /// Either malformed JSON or missing one of the required fields
    /// (`authorization_endpoint` / `token_endpoint` / `userinfo_endpoint`).
    #[error("OIDC discovery `{url}` body parse failed: {source}")]
    ParseError {
        /// The URL being fetched.
        url: String,
        /// Underlying serde error.
        #[source]
        source: serde_json::Error,
    },
    /// Discovery response body exceeded the
    /// `DISCOVERY_MAX_RESPONSE_BYTES` ceiling (256 KiB). Hostile / misconfigured
    /// IdP tried to serve an unbounded JSON blob through the discovery
    /// channel.
    #[error("OIDC discovery `{url}` body exceeded {max} bytes")]
    BodyTooLarge {
        /// The URL being fetched.
        url: String,
        /// The byte cap that was hit.
        max: usize,
    },
    /// A URL RETURNED in the discovery response failed the post-fetch
    /// SSRF re-validation (D-15-WAVE6). Hostile / misconfigured IdP
    /// tried to point Nebula at an internal address via the
    /// discovery channel.
    #[error(
        "OIDC discovery from `{discovery_url}` returned an unsafe child URL for `{field}`: {reason}"
    )]
    EndpointSsrfRejected {
        /// The discovery_url that was fetched.
        discovery_url: String,
        /// Which child field failed (`authorize_url` / `token_url` /
        /// `userinfo_url` / `jwks_url`).
        field: &'static str,
        /// Reason from the validator.
        reason: String,
    },
}

/// Fetch and cache the OIDC discovery document for `url`.
///
/// Steps per ADR-0085 D-15-WAVE6:
/// 1. Strict-gate the `discovery_url` itself.
/// 2. Cache hit returns immediately.
/// 3. GET via shared `oauth_token_http_client()` with a 5s timeout.
/// 4. Parse JSON into [`OidcDiscovery`].
/// 5. Validate each child URL per its threat model
///    (strict for token / userinfo / jwks; flag-aware for authorize).
/// 6. Cache insert + return.
///
/// `oauth_allow_insecure_localhost` is the operator's
/// `API_AUTH_OAUTH_ALLOW_INSECURE_LOCALHOST` flag (relaxes
/// `http://localhost` for the discovery-returned authorize URL ONLY
/// in dev builds).
///
/// # Errors
///
/// Returns [`DiscoveryError`] on any of the steps above. The cache
/// stays empty for the failed URL so a retry after the operator
/// fixes the issue takes effect immediately.
pub async fn fetch_oidc_discovery(
    url: &str,
    oauth_allow_insecure_localhost: bool,
) -> Result<OidcDiscovery, DiscoveryError> {
    fetch_oidc_discovery_inner(url, oauth_allow_insecure_localhost, true).await
}

/// Test-only escape hatch (gated by `#[cfg(any(test, nebula_test_util))]`)
/// that runs the same fetch flow but **skips** the strict gate on the
/// `discovery_url` itself — so wiremock fixtures on `127.0.0.1` can
/// publish a `.well-known/openid-configuration` URL.
///
/// Per Codex / Copilot wave-1 review: the PR-2 `fetch_oidc_discovery_unchecked`
/// shim that just delegated to the production path was broken for the
/// documented localhost wiremock use case because the discovery URL
/// itself was rejected before the discovery JSON could even be
/// fetched. This split fixes that while preserving:
/// - Bounded body cap (256 KiB).
/// - Disabled redirects.
/// - Post-fetch child-URL re-validation — with the flag-aware
///   authorize gate threaded through so the discovery-served
///   `authorize_url` can be `http://localhost:PORT/...` in dev.
/// - Strict gate on token / userinfo / jwks (those are server-side
///   fetches; wiremock fixtures MUST serve HTTPS-with-self-signed-cert
///   for them OR the test reaches them via `oauth_token_http_client_test_unchecked`).
#[cfg(any(test, nebula_test_util))]
pub async fn fetch_oidc_discovery_skipping_url_gate(
    url: &str,
    oauth_allow_insecure_localhost: bool,
) -> Result<OidcDiscovery, DiscoveryError> {
    fetch_oidc_discovery_inner(url, oauth_allow_insecure_localhost, false).await
}

async fn fetch_oidc_discovery_inner(
    url: &str,
    oauth_allow_insecure_localhost: bool,
    gate_discovery_url: bool,
) -> Result<OidcDiscovery, DiscoveryError> {
    // Step 1: strict gate on the discovery URL itself — unless the
    // test-only bypass set `gate_discovery_url = false` so wiremock
    // can serve the doc from `127.0.0.1`.
    if gate_discovery_url {
        validate_oauth_outbound_url(url).map_err(|reason| {
            DiscoveryError::DiscoveryUrlRejected {
                url: url.to_owned(),
                reason,
            }
        })?;
    }

    // Step 2: cache check (entry lives until process restart).
    if let Some(cached) = cache().get(url) {
        return Ok(cached.clone());
    }

    // Step 3: GET with bounded timeout via the discovery-only client
    // (redirects disabled per Copilot wave-1: a valid HTTPS
    // discovery_url MUST NOT redirect us to an unvalidated host).
    let response = oauth_discovery_http_client()
        .get(url)
        .send()
        .await
        .map_err(|source| DiscoveryError::HttpError {
            url: url.to_owned(),
            source,
        })?;
    let status = response.status();
    if !status.is_success() {
        return Err(DiscoveryError::NonSuccessStatus {
            url: url.to_owned(),
            status: status.as_u16(),
        });
    }

    // Body size cap (Copilot wave-1): mirror the token POST 256 KiB
    // ceiling so a hostile IdP cannot serve an unbounded JSON blob
    // through the discovery channel. Streaming check covers both
    // `Content-Length`-honest and chunked-no-length adversarial
    // responses.
    if let Some(claimed) = response.content_length()
        && claimed > DISCOVERY_MAX_RESPONSE_BYTES as u64
    {
        return Err(DiscoveryError::BodyTooLarge {
            url: url.to_owned(),
            max: DISCOVERY_MAX_RESPONSE_BYTES,
        });
    }
    let mut buf: Vec<u8> = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|source| DiscoveryError::HttpError {
            url: url.to_owned(),
            source,
        })?;
        if buf.len().saturating_add(chunk.len()) > DISCOVERY_MAX_RESPONSE_BYTES {
            return Err(DiscoveryError::BodyTooLarge {
                url: url.to_owned(),
                max: DISCOVERY_MAX_RESPONSE_BYTES,
            });
        }
        buf.extend_from_slice(&chunk);
    }

    // Step 4: parse JSON.
    let discovery: OidcDiscovery =
        serde_json::from_slice(&buf).map_err(|source| DiscoveryError::ParseError {
            url: url.to_owned(),
            source,
        })?;

    // Step 5: re-validate each returned child URL per its threat
    // model. ANY failure aborts cache insert.
    //
    // The browser-fetched authorize URL goes through the flag-aware
    // gate so a localhost IdP works in dev when the operator opts in.
    // Production builds (`!cfg!(debug_assertions)`) reject the flag's
    // effect inside the validator itself; we just pass `false` for
    // `in_release_build` since this call site is server-side
    // discovery cache (boot/runtime) and the flag-aware behaviour is
    // already encoded in the validator function.
    let in_release_build = !cfg!(debug_assertions);
    validate_oauth_authorize_url(
        &discovery.authorize_url,
        oauth_allow_insecure_localhost,
        in_release_build,
    )
    .map_err(|reason| DiscoveryError::EndpointSsrfRejected {
        discovery_url: url.to_owned(),
        field: "authorize_url",
        reason,
    })?;
    validate_oauth_outbound_url(&discovery.token_url).map_err(|reason| {
        DiscoveryError::EndpointSsrfRejected {
            discovery_url: url.to_owned(),
            field: "token_url",
            reason,
        }
    })?;
    validate_oauth_outbound_url(&discovery.userinfo_url).map_err(|reason| {
        DiscoveryError::EndpointSsrfRejected {
            discovery_url: url.to_owned(),
            field: "userinfo_url",
            reason,
        }
    })?;
    if let Some(ref jwks_url) = discovery.jwks_url {
        validate_oauth_outbound_url(jwks_url).map_err(|reason| {
            DiscoveryError::EndpointSsrfRejected {
                discovery_url: url.to_owned(),
                field: "jwks_url",
                reason,
            }
        })?;
    }

    // Step 6: cache insert + return.
    cache().insert(url.to_owned(), discovery.clone());
    Ok(discovery)
}

/// Resolved authorize / token / userinfo / scopes triple from an
/// `OAuthProviderConfig` (intentional bare-name to avoid an intra-doc
/// link target that's outside this module's scope). The OAuth
/// `start_oauth` and
/// `complete_oauth` paths consume this uniformly regardless of
/// whether the provider is configured as Oidc or Manual.
///
/// PR-3 consumes `authorize_url` + `scopes` for the authorize-URL
/// emission; PR-4 consumes `token_url` + `userinfo_url` for the
/// code exchange + userinfo lookup.
#[derive(Debug, Clone)]
pub struct ResolvedEndpoints {
    /// Browser-fetched authorize URL.
    pub authorize_url: String,
    /// Server-side token endpoint URL.
    pub token_url: String,
    /// Server-side userinfo endpoint URL.
    pub userinfo_url: String,
    /// Optional second userinfo endpoint for verified-email lookup
    /// (e.g. GitHub's `/user/emails`). PR-4 consumes when `Some`.
    pub verified_emails_url: Option<String>,
    /// Space-joined scopes string for the authorize URL.
    pub scopes: String,
}

/// Resolve a provider's endpoints into the runtime-uniform
/// [`ResolvedEndpoints`] shape. For Oidc providers this fetches the
/// discovery doc (cache-served after first hit) and runs the
/// post-fetch SSRF re-validation; for Manual providers it returns
/// the operator-configured URLs as-is (already validated at boot).
///
/// # Errors
///
/// Returns the underlying [`DiscoveryError`] for Oidc providers when
/// the discovery doc fetch or post-fetch validation fails.
pub async fn resolve_provider_endpoints(
    cfg: &crate::config::OAuthProviderConfig,
    oauth_allow_insecure_localhost: bool,
) -> Result<ResolvedEndpoints, DiscoveryError> {
    use crate::config::OAuthEndpoints;

    match &cfg.endpoints {
        OAuthEndpoints::Oidc { discovery_url } => {
            let doc = fetch_oidc_discovery(discovery_url, oauth_allow_insecure_localhost).await?;
            Ok(ResolvedEndpoints {
                authorize_url: doc.authorize_url,
                token_url: doc.token_url,
                userinfo_url: doc.userinfo_url,
                verified_emails_url: None,
                scopes: cfg.endpoints.scopes().join(" "),
            })
        },
        OAuthEndpoints::Manual {
            authorize_url,
            token_url,
            userinfo_url,
            verified_emails_url,
            ..
        } => Ok(ResolvedEndpoints {
            authorize_url: authorize_url.clone(),
            token_url: token_url.clone(),
            userinfo_url: userinfo_url.clone(),
            verified_emails_url: verified_emails_url.clone(),
            scopes: cfg.endpoints.scopes().join(" "),
        }),
    }
}

/// Test-only: clear the cache so tests can exercise the fetch path
/// without inter-test contamination. `pub(crate)` so the unit-test
/// module + the `nebula_test_util` cfg-gated `test_support` module
/// can reach it.
#[cfg(any(test, nebula_test_util))]
#[allow(
    dead_code,
    reason = "surfaced only through the test_support cfg-gated re-export; in regular `cargo test` the inner cache is not yet exercised"
)]
pub(crate) fn clear_discovery_cache_for_tests() {
    cache().clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::OnceLock;
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;

    /// Wave-1 Copilot: cover the strict gate that rejects unsafe
    /// discovery URLs BEFORE any HTTP call.
    #[tokio::test]
    async fn discovery_rejects_unsafe_discovery_url_via_strict_gate() {
        let err = fetch_oidc_discovery("http://10.0.0.5/.well-known/openid-configuration", false)
            .await
            .expect_err("HTTP + private IP must be rejected");
        assert!(matches!(err, DiscoveryError::DiscoveryUrlRejected { .. }));
    }

    /// Spin up a tiny TCP listener that responds with a fixed HTTP/1.1
    /// payload to the first request. Returns the bound URL.
    async fn spawn_oneshot_responder(payload: Vec<u8>) -> String {
        // Use a guaranteed-unique env-derived test target so this
        // function works under concurrent nextest runs.
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let url = format!("http://{addr}/.well-known/openid-configuration");

        tokio::spawn(async move {
            if let Ok((mut sock, _)) = listener.accept().await {
                // Drain request line (best-effort; small).
                let mut throwaway = [0u8; 4096];
                let _ = tokio::io::AsyncReadExt::read(&mut sock, &mut throwaway).await;
                let _ = sock.write_all(&payload).await;
                let _ = sock.shutdown().await;
            }
        });

        url
    }

    fn http_response_with_body(body: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        write!(
            &mut out,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
            body.len()
        )
        .unwrap();
        out.extend_from_slice(body);
        out
    }

    /// Wave-1 Copilot E.5: cover the cache-hit path and the post-fetch
    /// child-URL validation.
    ///
    /// Uses the `fetch_oidc_discovery_skipping_url_gate` test bypass
    /// to point at `127.0.0.1` (production gate would reject the
    /// loopback discovery URL before any network call).
    #[tokio::test]
    async fn discovery_caches_doc_and_serves_subsequent_calls_from_cache() {
        // Each test gets a fresh cache to avoid cross-test leakage.
        static GUARD: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
        let _lock = GUARD
            .get_or_init(|| tokio::sync::Mutex::new(()))
            .lock()
            .await;
        clear_discovery_cache_for_tests();

        let doc = serde_json::json!({
            "authorization_endpoint": "https://login.example.com/oauth/authorize",
            "token_endpoint": "https://login.example.com/oauth/token",
            "userinfo_endpoint": "https://login.example.com/oauth/userinfo"
        })
        .to_string();
        let url = spawn_oneshot_responder(http_response_with_body(doc.as_bytes())).await;

        // First call performs the actual GET.
        let first = fetch_oidc_discovery_skipping_url_gate(&url, true)
            .await
            .expect("first fetch must succeed");
        assert_eq!(first.token_url, "https://login.example.com/oauth/token");

        // Second call returns the cached copy without spawning a
        // second listener — verified by the fact that no second
        // responder is running on the URL.
        let second = fetch_oidc_discovery_skipping_url_gate(&url, true)
            .await
            .expect("second fetch must hit cache");
        assert_eq!(second.token_url, first.token_url);
    }

    /// Wave-1 Copilot E.5: cover the post-fetch child-URL re-validation
    /// when a discovery doc returns an unsafe internal-IP endpoint.
    #[tokio::test]
    async fn discovery_rejects_child_url_pointing_at_internal_ip() {
        static GUARD: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
        let _lock = GUARD
            .get_or_init(|| tokio::sync::Mutex::new(()))
            .lock()
            .await;
        clear_discovery_cache_for_tests();

        // Hostile discovery doc: valid authorize_url but internal-IP
        // token_endpoint (post-fetch strict gate must reject).
        let doc = serde_json::json!({
            "authorization_endpoint": "https://login.example.com/oauth/authorize",
            "token_endpoint": "http://10.0.0.5/internal-token",
            "userinfo_endpoint": "https://login.example.com/oauth/userinfo"
        })
        .to_string();
        let url = spawn_oneshot_responder(http_response_with_body(doc.as_bytes())).await;
        let err = fetch_oidc_discovery_skipping_url_gate(&url, true)
            .await
            .expect_err("internal-IP child URL must be rejected");
        match err {
            DiscoveryError::EndpointSsrfRejected { field, .. } => {
                assert_eq!(field, "token_url");
            },
            other => panic!("expected EndpointSsrfRejected, got: {other:?}"),
        }
    }

    /// Wave-1 Copilot E.2: cover the body-size cap.
    #[tokio::test]
    async fn discovery_rejects_oversized_body_via_content_length() {
        static GUARD: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
        let _lock = GUARD
            .get_or_init(|| tokio::sync::Mutex::new(()))
            .lock()
            .await;
        clear_discovery_cache_for_tests();

        // Claim a body length over the 256 KiB cap. The gate trips
        // before any chunk read.
        let body_len = DISCOVERY_MAX_RESPONSE_BYTES + 1;
        let mut response = Vec::new();
        write!(
            &mut response,
            "HTTP/1.1 200 OK\r\nContent-Length: {body_len}\r\n\r\n"
        )
        .unwrap();
        // No body bytes follow — we never get past the size gate.
        let url = spawn_oneshot_responder(response).await;
        let err = fetch_oidc_discovery_skipping_url_gate(&url, true)
            .await
            .expect_err("oversized Content-Length must be rejected");
        assert!(matches!(err, DiscoveryError::BodyTooLarge { .. }));
    }
}
