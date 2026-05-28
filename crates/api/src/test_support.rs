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

/// Failure modes for [`fetch_oidc_discovery_unchecked`].
///
/// Typed (not stringly) error per the public-API discipline — tests
/// can `match` on a variant instead of substring-grepping a message.
/// PR-3 lands the real fetch behavior; this typed shape is forward-
/// compatible.
#[derive(Debug, Error)]
pub enum DiscoveryUncheckedError {
    /// The discovery fetch is not yet wired up. Returned by the PR-2
    /// placeholder; PR-3 replaces this variant with real fetch / parse
    /// errors.
    #[error(
        "fetch_oidc_discovery_unchecked: PR-3 has not yet landed (openspec T3.1); the placeholder always returns this variant"
    )]
    PlaceholderUntilPr3,
}

/// Placeholder for the OIDC discovery doc fetcher bypass, to be wired
/// in PR-3 once `crates/api/src/transport/oauth/discovery.rs` lands
/// per T3.1. Defined here in PR-2 so the integration test crate has a
/// stable module path to import. Returns
/// [`DiscoveryUncheckedError::PlaceholderUntilPr3`] until PR-3 fills it
/// in.
///
/// # Errors
///
/// Always returns `Err(PlaceholderUntilPr3)` until PR-3 lands the
/// discovery cache.
pub async fn fetch_oidc_discovery_unchecked(_url: &str) -> Result<(), DiscoveryUncheckedError> {
    Err(DiscoveryUncheckedError::PlaceholderUntilPr3)
}
