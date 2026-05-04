//! Per-trigger authentication for slug-routed webhooks.
//!
//! Each registered trigger carries a [`WebhookAuthConfig`] describing
//! how incoming requests must authenticate. The dispatcher runs the
//! check before handing the event to the engine sink — failures map
//! to 401 (or 500 if the operator misconfigured the trigger).
//!
//! Three policies ship in M3.3:
//!
//! - [`WebhookAuthConfig::None`] — public webhook. The handler accepts any body. Matches
//!   `SignaturePolicy::OptionalAcceptUnsigned` on the typed-action surface.
//! - [`WebhookAuthConfig::HmacSha256`] — verify an HMAC-SHA-256 of the raw body against a
//!   configured shared secret. Matches the typed-action `Required(HmacSha256)` semantics.
//! - [`WebhookAuthConfig::BearerToken`] — verify a static bearer token in the `Authorization`
//!   header. Used by simple integrations that do not need HMAC.
//!
//! All comparisons are constant-time. Reuses the
//! `verify_hmac_sha256` primitive from `nebula_action` so the
//! slug-routed surface and the typed-action transport agree on
//! signature semantics byte-for-byte.

use std::sync::Arc;

use axum::http::HeaderMap;
use nebula_action::{SignatureOutcome, WebhookRequest, verify_hmac_sha256};

use super::error::WebhookAuthError;

/// Default header name carrying the HMAC-SHA-256 signature.
///
/// Mirrors the canonical Nebula header used by the typed-action surface when the
/// action leaves the default signature header in place (see `WebhookConfig::with_header_str` /
/// `WebhookConfig::with_header` in `nebula_action`). Custom headers (e.g. `X-Hub-Signature-256`
/// for GitHub-compatible flows) are supplied per registration via the `header` field on
/// [`WebhookAuthConfig::HmacSha256`].
pub const DEFAULT_SIGNATURE_HEADER: &str = "X-Nebula-Signature";

/// Authentication policy attached to a registered trigger.
#[derive(Clone, Debug)]
pub enum WebhookAuthConfig {
    /// Public webhook — no authentication required. Caller is trusted.
    None,
    /// HMAC-SHA-256 signature of the raw body, hex-encoded, optionally
    /// prefixed with `sha256=` (the dispatcher tolerates both forms to
    /// match the GitHub / Nebula primitives' behaviour).
    HmacSha256 {
        /// Shared secret used to compute the HMAC.
        secret: Arc<[u8]>,
        /// Header name carrying the signature. Defaults to
        /// [`DEFAULT_SIGNATURE_HEADER`] when constructed via
        /// [`Self::hmac_sha256`].
        header: String,
    },
    /// Static bearer token in `Authorization: Bearer <token>`.
    BearerToken {
        /// Expected token value. Compared in constant time.
        token: Arc<str>,
    },
}

impl WebhookAuthConfig {
    /// Build a HMAC-SHA-256 policy with the canonical Nebula header.
    #[must_use]
    pub fn hmac_sha256(secret: impl Into<Arc<[u8]>>) -> Self {
        Self::HmacSha256 {
            secret: secret.into(),
            header: DEFAULT_SIGNATURE_HEADER.to_string(),
        }
    }

    /// Build a HMAC-SHA-256 policy with a custom header name (e.g.
    /// `X-Hub-Signature-256` for GitHub-compatible registrations).
    #[must_use]
    pub fn hmac_sha256_with_header(
        secret: impl Into<Arc<[u8]>>,
        header: impl Into<String>,
    ) -> Self {
        Self::HmacSha256 {
            secret: secret.into(),
            header: header.into(),
        }
    }

    /// Build a bearer-token policy.
    #[must_use]
    pub fn bearer(token: impl Into<Arc<str>>) -> Self {
        Self::BearerToken {
            token: token.into(),
        }
    }
}

/// Validate an incoming request against the registered policy.
///
/// `request` is the typed [`WebhookRequest`] the dispatcher built from
/// the axum extractors; reusing the same type lets us share the
/// signature primitive with the typed-action transport.
pub(crate) fn validate(
    config: &WebhookAuthConfig,
    request: &WebhookRequest,
) -> Result<(), WebhookAuthError> {
    match config {
        WebhookAuthConfig::None => Ok(()),
        WebhookAuthConfig::HmacSha256 { secret, header } => {
            if secret.is_empty() {
                return Err(WebhookAuthError::SecretNotConfigured);
            }
            let outcome = verify_hmac_sha256(request, secret.as_ref(), header)
                .unwrap_or(SignatureOutcome::Invalid);
            match outcome {
                SignatureOutcome::Valid => Ok(()),
                SignatureOutcome::Missing => Err(WebhookAuthError::SignatureMissing),
                SignatureOutcome::Invalid => Err(WebhookAuthError::SignatureInvalid),
                // `SignatureOutcome` is `#[non_exhaustive]`; future
                // variants are fail-closed.
                _ => Err(WebhookAuthError::SignatureInvalid),
            }
        },
        WebhookAuthConfig::BearerToken { token } => check_bearer(token, request.headers()),
    }
}

fn check_bearer(expected: &Arc<str>, headers: &HeaderMap) -> Result<(), WebhookAuthError> {
    let header = headers
        .get(axum::http::header::AUTHORIZATION)
        .ok_or(WebhookAuthError::TokenMissing)?
        .to_str()
        .map_err(|_| WebhookAuthError::TokenInvalid)?;
    let provided = header
        .strip_prefix("Bearer ")
        .ok_or(WebhookAuthError::TokenInvalid)?;

    if ct_eq_bytes(provided.as_bytes(), expected.as_bytes()) {
        Ok(())
    } else {
        Err(WebhookAuthError::TokenInvalid)
    }
}

/// Constant-time byte-slice equality.
///
/// Mirrors `subtle::ConstantTimeEq` for slices but avoids pulling
/// `subtle` into `nebula-api` (the Cargo.toml is shared with other
/// builders this milestone). The compiler must not short-circuit on
/// mismatch; we OR the per-byte XOR so the timing is governed by the
/// shorter of the two slices, with a length-mismatch always producing
/// a non-zero accumulator. Mirrors the loop pattern used inside
/// `subtle::ConstantTimeEq`.
fn ct_eq_bytes(a: &[u8], b: &[u8]) -> bool {
    let len_diff = a.len() ^ b.len();
    let mut diff: u32 = len_diff as u32;
    let min = a.len().min(b.len());
    let mut i = 0;
    while i < min {
        diff |= u32::from(a[i] ^ b[i]);
        i += 1;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use axum::{
        body::Bytes,
        http::{HeaderMap, HeaderValue, Method},
    };
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::Sha256;

    use super::*;

    type HmacSha256 = Hmac<Sha256>;

    fn sign(secret: &[u8], body: &[u8]) -> String {
        let mut mac = HmacSha256::new_from_slice(secret).unwrap();
        mac.update(body);
        format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
    }

    fn make_request(headers: HeaderMap, body: Vec<u8>) -> WebhookRequest {
        WebhookRequest::try_new(
            Method::POST,
            "/api/v1/hooks/acme/main/gh".to_string(),
            None::<String>,
            headers,
            Bytes::from(body),
        )
        .unwrap()
    }

    #[test]
    fn none_policy_passes_any_request() {
        let req = make_request(HeaderMap::new(), b"{}".to_vec());
        validate(&WebhookAuthConfig::None, &req).unwrap();
    }

    #[test]
    fn hmac_valid_signature_passes() {
        let secret: Arc<[u8]> = Arc::<[u8]>::from(b"shh".as_slice());
        let body = br#"{"ok":true}"#.to_vec();
        let mut headers = HeaderMap::new();
        headers.insert(
            DEFAULT_SIGNATURE_HEADER,
            HeaderValue::from_str(&sign(&secret, &body)).unwrap(),
        );

        let cfg = WebhookAuthConfig::hmac_sha256(secret);
        validate(&cfg, &make_request(headers, body)).unwrap();
    }

    #[test]
    fn hmac_missing_header_returns_missing() {
        let cfg = WebhookAuthConfig::hmac_sha256(Arc::<[u8]>::from(b"shh".as_slice()));
        let err = validate(&cfg, &make_request(HeaderMap::new(), b"{}".to_vec())).unwrap_err();
        assert_eq!(err, WebhookAuthError::SignatureMissing);
    }

    #[test]
    fn hmac_bad_signature_returns_invalid() {
        let mut headers = HeaderMap::new();
        headers.insert(
            DEFAULT_SIGNATURE_HEADER,
            HeaderValue::from_static("sha256=deadbeef"),
        );
        let cfg = WebhookAuthConfig::hmac_sha256(Arc::<[u8]>::from(b"shh".as_slice()));
        let err = validate(&cfg, &make_request(headers, b"{}".to_vec())).unwrap_err();
        assert_eq!(err, WebhookAuthError::SignatureInvalid);
    }

    #[test]
    fn hmac_empty_secret_returns_secret_not_configured() {
        let cfg = WebhookAuthConfig::hmac_sha256(Arc::<[u8]>::from(Vec::<u8>::new()));
        let err = validate(&cfg, &make_request(HeaderMap::new(), b"{}".to_vec())).unwrap_err();
        assert_eq!(err, WebhookAuthError::SecretNotConfigured);
    }

    #[test]
    fn hmac_custom_header_is_honored() {
        let secret: Arc<[u8]> = Arc::<[u8]>::from(b"shh".as_slice());
        let body = br#"{"github":true}"#.to_vec();
        let mut headers = HeaderMap::new();
        headers.insert(
            "X-Hub-Signature-256",
            HeaderValue::from_str(&sign(&secret, &body)).unwrap(),
        );
        let cfg = WebhookAuthConfig::hmac_sha256_with_header(secret, "X-Hub-Signature-256");
        validate(&cfg, &make_request(headers, body)).unwrap();
    }

    #[test]
    fn bearer_valid_token_passes() {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Bearer correct-horse-battery-staple"),
        );
        let cfg = WebhookAuthConfig::bearer("correct-horse-battery-staple");
        validate(&cfg, &make_request(headers, b"{}".to_vec())).unwrap();
    }

    #[test]
    fn bearer_missing_header_returns_token_missing() {
        let cfg = WebhookAuthConfig::bearer("expected");
        let err = validate(&cfg, &make_request(HeaderMap::new(), b"{}".to_vec())).unwrap_err();
        assert_eq!(err, WebhookAuthError::TokenMissing);
    }

    #[test]
    fn bearer_wrong_token_returns_token_invalid() {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Bearer wrong"),
        );
        let cfg = WebhookAuthConfig::bearer("right");
        let err = validate(&cfg, &make_request(headers, b"{}".to_vec())).unwrap_err();
        assert_eq!(err, WebhookAuthError::TokenInvalid);
    }

    #[test]
    fn bearer_non_bearer_scheme_returns_token_invalid() {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Basic dXNlcjpwYXNz"),
        );
        let cfg = WebhookAuthConfig::bearer("right");
        let err = validate(&cfg, &make_request(headers, b"{}".to_vec())).unwrap_err();
        assert_eq!(err, WebhookAuthError::TokenInvalid);
    }
}
