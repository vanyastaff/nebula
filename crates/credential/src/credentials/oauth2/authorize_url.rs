//! Authorization URL construction for the OAuth2 Authorization Code grant.
//!
//! This module is intentionally **HTTP-client-free**: it only builds the
//! browser redirect URL (RFC 6749 §4.1.1 + RFC 7636 PKCE query parameters).
//! Token exchange and refresh live in [`super::flow`] behind the
//! `oauth2-http` feature (ADR-0031 incremental split).

use super::config::OAuth2Config;
use crate::error::CredentialError;

/// Build the authorization URL for the Authorization Code grant.
///
/// Appends every query parameter required by RFC 6749 §4.1.1 plus the
/// RFC 7636 PKCE extension and the anti-CSRF `state` parameter. The
/// config MUST come from the `AuthCodeBuilder` in `config`, which
/// guarantees that `config.pkce` and `config.redirect_uri` are both
/// `Some(_)` — callers cannot hand us a misconfigured [`OAuth2Config`]
/// without a compile error. The runtime `ok_or_else` branches are there
/// only to defend against struct-literal construction and malformed
/// deserialized records.
///
/// Uses [`url::Url`] query encoding so special characters in
/// `client_id`, `redirect_uri`, and scope values are properly
/// percent-encoded.
pub(crate) fn build_auth_url(
    config: &OAuth2Config,
    client_id: &str,
    code_challenge: &str,
    state: &str,
) -> Result<String, CredentialError> {
    let redirect_uri = config
        .redirect_uri
        .as_deref()
        .ok_or_else(|| provider_error("authorization_code config missing redirect_uri".into()))?;
    let pkce_method = config
        .pkce
        .ok_or_else(|| provider_error("authorization_code config missing pkce method".into()))?;

    let mut url = url::Url::parse(&config.auth_url)
        .map_err(|e| provider_error(format!("invalid auth_url: {e}")))?;

    {
        let mut q = url.query_pairs_mut();
        q.append_pair("response_type", "code");
        q.append_pair("client_id", client_id);
        q.append_pair("redirect_uri", redirect_uri);

        if !config.scopes.is_empty() {
            q.append_pair("scope", &config.scopes.join(" "));
        }

        q.append_pair("state", state);
        q.append_pair("code_challenge", code_challenge);
        q.append_pair("code_challenge_method", pkce_method.as_str());
    }

    Ok(url.to_string())
}

fn provider_error(message: String) -> CredentialError {
    CredentialError::Provider(message)
}

#[cfg(test)]
mod tests {
    use super::{super::config::OAuth2Config, *};

    const CALLBACK: &str = "https://app.example.com/cb";

    /// RFC 7636 appendix B vector (section 4.2).
    const RFC7636_VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    const RFC7636_CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

    #[test]
    fn build_auth_url_includes_code_challenge_s256() {
        let config = OAuth2Config::authorization_code(CALLBACK)
            .auth_url("https://example.com/auth")
            .token_url("https://example.com/token")
            .build();

        let url = build_auth_url(&config, "cid", RFC7636_CHALLENGE, "state_abc").unwrap();
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains(&format!("code_challenge={RFC7636_CHALLENGE}")));
    }

    #[test]
    fn build_auth_url_includes_state_and_redirect_uri() {
        let config = OAuth2Config::authorization_code(CALLBACK)
            .auth_url("https://example.com/auth")
            .token_url("https://example.com/token")
            .build();

        let url = build_auth_url(&config, "cid", "chal", "state_abc").unwrap();
        assert!(url.contains("state=state_abc"));
        // `CALLBACK` contains `://` which percent-encodes as `%3A%2F%2F`.
        assert!(
            url.contains("redirect_uri=https%3A%2F%2Fapp.example.com%2Fcb"),
            "redirect_uri not percent-encoded in URL: {url}"
        );
    }

    #[test]
    fn build_auth_url_verifier_hashes_to_challenge() {
        // Guard against a future refactor breaking the PKCE helper chain.
        let challenge = crate::generate_code_challenge(RFC7636_VERIFIER);
        assert_eq!(challenge, RFC7636_CHALLENGE);
    }

    #[test]
    fn build_auth_url_without_scopes() {
        let config = OAuth2Config::authorization_code(CALLBACK)
            .auth_url("https://example.com/auth")
            .token_url("https://example.com/token")
            .build();

        let url = build_auth_url(&config, "cid", "chal", "st").unwrap();
        assert!(!url.contains("scope="));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("client_id=cid"));
    }

    #[test]
    fn build_auth_url_with_scopes() {
        let config = OAuth2Config::authorization_code(CALLBACK)
            .auth_url("https://example.com/auth")
            .token_url("https://example.com/token")
            .scopes(["read", "write"])
            .build();

        let url = build_auth_url(&config, "cid", "chal", "st").unwrap();
        assert!(url.contains("scope=read+write"));
    }

    #[test]
    fn invalid_auth_url_returns_error() {
        let config = OAuth2Config::authorization_code(CALLBACK)
            .auth_url("not a url")
            .token_url("https://t.com/token")
            .build();

        let result = build_auth_url(&config, "cid", "chal", "st");
        assert!(result.is_err());
    }
}
