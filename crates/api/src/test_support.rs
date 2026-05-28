//! Test-only OAuth bypass helpers. Gated by `#[cfg(nebula_test_util)]`.
//!
//! ## Why this module exists
//!
//! Integration tests for the Plane-A OAuth identity flow (PR-4) need to
//! exercise the server-side IdP HTTP call sites (token POST, userinfo
//! GET, OIDC discovery GET) against `wiremock` listening on `127.0.0.1`.
//! The strict anti-SSRF gate
//! ([`crate::transport::oauth::flow::validate_oauth_outbound_url`])
//! rejects loopback / private / non-HTTPS URLs by design — production
//! traffic must not reach internal infra.
//!
//! Per ADR-0085 D-14 + D-9-WAVE6, this module exposes ALL three bypass
//! helpers needed to mount wiremock against `127.0.0.1` for tests:
//!
//! - [`exchange_code_unchecked`] — token endpoint POST, skips
//!   `validate_oauth_outbound_url`.
//! - [`oauth_token_http_client_test_unchecked`] — returns the same
//!   shared `reqwest::Client` used in production, intended for direct
//!   userinfo / verified-emails GETs from test code that has already
//!   confirmed it is using a safe localhost target.
//! - [`fetch_oidc_discovery_unchecked`] — PR-3 will populate this once
//!   the discovery cache lands; for now this is a placeholder so PR-2's
//!   `test_support` scaffold compiles and integration test files can
//!   import the module path.
//!
//! ## Production-build guard
//!
//! The release-build guard in `crates/api/src/lib.rs`
//! (`#[cfg(all(nebula_test_util, not(debug_assertions)))]
//! compile_error!`) refuses to compile if the cfg is set in a release
//! build. Production binaries (no `RUSTFLAGS` opt-in) do NOT contain
//! this module at all.
//!
//! ## How tests opt in
//!
//! ```sh
//! RUSTFLAGS="--cfg nebula_test_util" cargo nextest run -p nebula-api \
//!     --test oauth_provider_e2e
//! ```

use thiserror::Error;

// Re-export `flow::exchange_code_unchecked` for integration tests.
//
// `exchange_code_unchecked` is private to `flow.rs` and the in-crate
// tests reach it directly; integration tests live in a separate crate
// and need this `pub` re-export to call it. The `nebula_test_util` cfg
// gate ensures the symbol is unreachable from production builds.
pub use crate::transport::oauth::flow::exchange_code_unchecked;

use crate::transport::oauth::http::oauth_token_http_client;

/// Return the same shared `reqwest::Client` used by production OAuth
/// HTTP calls, intended for direct userinfo / verified-emails GETs
/// from test code mounting wiremock on localhost. Bypassing
/// `validate_oauth_outbound_url` is the caller's responsibility — this
/// helper only exposes the client without invoking the SSRF gate.
///
/// Production code MUST NOT use this helper; the `nebula_test_util` cfg
/// gate guarantees it does not exist in release builds.
#[must_use]
pub fn oauth_token_http_client_test_unchecked() -> &'static reqwest::Client {
    oauth_token_http_client()
}

/// Test-only OIDC discovery fetcher: same code path as production but
/// with `oauth_allow_insecure_localhost = true` forced so wiremock
/// listeners on `127.0.0.1`/`localhost` are accepted by the
/// flag-aware gate on the discovery-returned authorize URL.
///
/// Server-side fetched URLs (token / userinfo / jwks) still go through
/// the STRICT `validate_oauth_outbound_url` gate — wiremock setups
/// MUST publish endpoint URLs as HTTPS-with-self-signed-cert when
/// using this helper for end-to-end fixtures. Most integration
/// fixtures point the discovery doc at a wiremock-served
/// `.well-known/openid-configuration` whose returned
/// `authorization_endpoint` is `http://localhost:PORT/authorize` and
/// whose `token_endpoint` / `userinfo_endpoint` go through
/// `oauth_token_http_client_test_unchecked` (which bypasses
/// `validate_oauth_outbound_url` entirely).
///
/// PR-3 wire-up: was a `PlaceholderUntilPr3` placeholder; now
/// delegates to the real cache.
///
/// # Errors
///
/// Returns the same [`FetchDiscoveryError`] (alias of
/// `crate::transport::oauth::discovery::DiscoveryError`) as the
/// production path.
pub async fn fetch_oidc_discovery_unchecked(
    url: &str,
) -> Result<OidcDiscovery, FetchDiscoveryError> {
    // Per Codex / Copilot wave-1 PR-3 review: must skip the strict
    // gate on the discovery URL itself so wiremock on `127.0.0.1`
    // works; production `fetch_oidc_discovery` always strict-gates
    // the discovery URL. Body cap / disabled redirects / child-URL
    // re-validation all stay in force.
    crate::transport::oauth::discovery::fetch_oidc_discovery_skipping_url_gate(url, true).await
}

/// Test-only: clear the process-wide discovery cache so tests can
/// exercise the fetch path repeatedly without cross-test
/// contamination.
pub fn clear_discovery_cache_for_tests() {
    crate::transport::oauth::discovery::clear_discovery_cache_for_tests();
}
