//! Test-only OAuth bypass helpers. Gated by `#[cfg(nebula_test_util)]`.
//!
//! ## Why this module exists
//!
//! Integration tests for the Plane-A OAuth identity flow (PR-4) need to
//! exercise the server-side IdP HTTP call sites (token POST, userinfo
//! GET, OIDC discovery GET) against `wiremock` listening on `127.0.0.1`.
//! The strict anti-SSRF gate
//! ([`crate::transport::oauth::flow::validate_oauth_outbound_url`])
//! rejects loopback / private / non-HTTPS URLs by design \u2014 production
//! traffic must not reach internal infra.
//!
//! Per ADR-0085 D-14 + D-9-WAVE6, this module exposes ALL three bypass
//! helpers needed to mount wiremock against `127.0.0.1` for tests:
//!
//! - [`exchange_code_unchecked`] \u2014 token endpoint POST, skips
//!   `validate_oauth_outbound_url`.
//! - [`oauth_token_http_client_test_unchecked`] \u2014 returns the same
//!   shared `reqwest::Client` used in production, intended for direct
//!   userinfo / verified-emails GETs from test code that has already
//!   confirmed it is using a safe localhost target.
//! - [`fetch_oidc_discovery_unchecked`] \u2014 PR-3 will populate this once
//!   the discovery cache lands; for now this is a placeholder so PR-2's\n//!   `test_support` scaffold compiles and integration test files can\n//!   import the module path.\n//!\n//! ## Production-build guard\n//!\n//! The release-build guard in `crates/api/src/lib.rs`\n//! (`#[cfg(all(nebula_test_util, not(debug_assertions)))]\n//! compile_error!`) refuses to compile if the cfg is set in a release\n//! build. Production binaries (no `RUSTFLAGS` opt-in) do NOT contain\n//! this module at all.\n//!\n//! ## How tests opt in\n//!\n//! ```sh\n//! RUSTFLAGS=\"--cfg nebula_test_util\" cargo nextest run -p nebula-api \\\n//!     --test oauth_provider_e2e\n//! ```\n\n// Re-export `flow::exchange_code_unchecked` for integration tests.\n//\n// `exchange_code_unchecked` is private to `flow.rs` and the in-crate\n// tests reach it directly; integration tests live in a separate crate\n// and need this `pub` re-export to call it. The `nebula_test_util` cfg\n// gate ensures the symbol is unreachable from production builds.\npub use crate::transport::oauth::flow::exchange_code_unchecked;\n\nuse crate::transport::oauth::http::oauth_token_http_client;\n\n/// Return the same shared `reqwest::Client` used by production OAuth\n/// HTTP calls, intended for direct userinfo / verified-emails GETs\n/// from test code mounting wiremock on localhost. Bypassing\n/// `validate_oauth_outbound_url` is the caller's responsibility \u2014 this\n/// helper only exposes the client without invoking the SSRF gate.\n///\n/// Production code MUST NOT use this helper; the `nebula_test_util` cfg\n/// gate guarantees it does not exist in release builds.\n#[must_use]\npub fn oauth_token_http_client_test_unchecked() -> &'static reqwest::Client {\n    oauth_token_http_client()\n}\n\n/// Placeholder for the OIDC discovery doc fetcher bypass, to be wired\n/// in PR-3 once `crates/api/src/transport/oauth/discovery.rs` lands\n/// per T3.1. Defined here in PR-2 so the integration test crate has a\n/// stable module path to import. Returns an error until PR-3 fills it\n/// in.\n///\n/// # Errors\n///\n/// Always returns `Err` until PR-3 lands the discovery cache.\npub async fn fetch_oidc_discovery_unchecked(_url: &str) -> Result<(), String> {\n    Err(\"fetch_oidc_discovery_unchecked: PR-3 not yet landed (T3.1)\".to_owned())\n}
