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
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::flow::{validate_oauth_authorize_url, validate_oauth_outbound_url};
use super::http::oauth_token_http_client;

/// Time budget for the discovery doc GET. Matches the token-endpoint
/// 5000ms default per REQ-obs-001; the discovery doc is small JSON
/// and shouldn't take longer.
const DISCOVERY_FETCH_TIMEOUT: Duration = Duration::from_secs(5);

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
    // Step 1: strict gate on the discovery URL itself.
    validate_oauth_outbound_url(url).map_err(|reason| DiscoveryError::DiscoveryUrlRejected {
        url: url.to_owned(),
        reason,
    })?;

    // Step 2: cache check (entry lives until process restart).
    if let Some(cached) = cache().get(url) {
        return Ok(cached.clone());
    }

    // Step 3: GET with bounded timeout via the shared OAuth HTTP
    // client (per ADR-0085 D-7: same client + policy as token POST).
    let response = oauth_token_http_client()
        .get(url)
        .timeout(DISCOVERY_FETCH_TIMEOUT)
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
    let body = response
        .bytes()
        .await
        .map_err(|source| DiscoveryError::HttpError {
            url: url.to_owned(),
            source,
        })?;

    // Step 4: parse JSON.
    let discovery: OidcDiscovery =
        serde_json::from_slice(&body).map_err(|source| DiscoveryError::ParseError {
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
/// [`OAuthProviderConfig`]. The OAuth `start_oauth` and
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
