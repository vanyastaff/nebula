//! OAuth2 provider configuration — built via typestate builders.
//!
//! The builder surface enforces grant-type-specific requirements at
//! compile time:
//!
//! - [`AuthCodeBuilder`] requires `redirect_uri` as a constructor argument (RFC 6749 §4.1.3), and
//!   unconditionally enables PKCE S256 (RFC 7636 + RFC 8252 §6).
//! - [`ClientCredentialsBuilder`] has no `redirect_uri` method and no `pkce` method — neither
//!   concept applies.
//! - [`DeviceCodeBuilder`] likewise has no `redirect_uri`/`pkce` methods.
//!
//! Closes the missing-`redirect_uri` / missing-`state` / missing-PKCE
//! holes from GitHub issues #250 and #251.

use serde::{Deserialize, Serialize};

/// How client credentials are sent in the OAuth2 token request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AuthStyle {
    /// RFC 6749: `Authorization: Basic base64(client_id:client_secret)` — default
    #[default]
    Header,
    /// `client_id` + `client_secret` as POST body form fields.
    ///
    /// Required by: GitHub, Slack, some legacy providers.
    PostBody,
}

/// OAuth2 grant type (RFC 6749 / RFC 8628).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum GrantType {
    /// Authorization Code flow — user browser redirect (default)
    #[default]
    AuthorizationCode,
    /// Client Credentials — server-to-server, no user interaction
    ClientCredentials,
    /// Device Authorization Grant (RFC 8628) — for CLI/TV apps
    DeviceCode,
}

/// PKCE code-challenge method (RFC 7636 §4.2).
///
/// Only `S256` is supported. `plain` is deliberately not implemented —
/// RFC 8252 §6 requires S256 for any client that can compute SHA-256,
/// which is every client we ship.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PkceMethod {
    /// `code_challenge = BASE64URL(SHA256(code_verifier))`
    #[default]
    S256,
}

impl PkceMethod {
    /// Wire representation sent as `code_challenge_method`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::S256 => "S256",
        }
    }
}

/// Provider-specific OAuth2 configuration.
///
/// Build via [`OAuth2Config::authorization_code()`] / `client_credentials()` /
/// `device_code()`. Each returns a grant-specific builder with only the
/// methods that make sense for that grant, so misconfiguration fails at
/// compile time rather than at runtime.
///
/// # AuthorizationCode invariants
///
/// - `redirect_uri == Some(_)` — the exact URI registered with the provider; echoed on the
///   token-exchange request per RFC 6749 §4.1.3.
/// - `pkce == Some(PkceMethod::S256)` — PKCE protection is mandatory.
///
/// For other grant types both fields are `None`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuth2Config {
    pub auth_url: String,
    pub token_url: String,
    pub scopes: Vec<String>,
    pub grant_type: GrantType,
    pub auth_style: AuthStyle,
    /// PKCE method. `Some(_)` iff `grant_type == AuthorizationCode`.
    #[serde(default)]
    pub pkce: Option<PkceMethod>,
    /// Redirect URI. `Some(_)` iff `grant_type == AuthorizationCode`.
    #[serde(default)]
    pub redirect_uri: Option<String>,
}

impl OAuth2Config {
    /// Start building an Authorization Code flow config.
    ///
    /// `redirect_uri` is a constructor argument rather than an optional
    /// builder method so a caller cannot accidentally produce a config
    /// that is missing it. The value must exactly match what is
    /// registered with the provider (RFC 6749 §4.1.3).
    pub fn authorization_code(redirect_uri: impl Into<String>) -> AuthCodeBuilder {
        AuthCodeBuilder {
            redirect_uri: redirect_uri.into(),
            auth_url: String::new(),
            token_url: String::new(),
            scopes: Vec::new(),
            auth_style: AuthStyle::Header,
            pkce: PkceMethod::S256,
        }
    }

    /// Start building a Client Credentials flow config.
    pub fn client_credentials() -> ClientCredentialsBuilder {
        ClientCredentialsBuilder {
            auth_url: String::new(),
            token_url: String::new(),
            scopes: Vec::new(),
            auth_style: AuthStyle::Header,
        }
    }

    /// Start building a Device Code flow config.
    pub fn device_code() -> DeviceCodeBuilder {
        DeviceCodeBuilder {
            auth_url: String::new(),
            token_url: String::new(),
            scopes: Vec::new(),
            auth_style: AuthStyle::Header,
        }
    }
}

/// Builder for Authorization Code grant configs.
///
/// Construction requires a redirect URI (see
/// [`OAuth2Config::authorization_code`]). PKCE S256 is enabled by
/// default and cannot be disabled — pass a different [`PkceMethod`] to
/// `.pkce()` if a future variant is added.
#[derive(Debug)]
pub struct AuthCodeBuilder {
    redirect_uri: String,
    auth_url: String,
    token_url: String,
    scopes: Vec<String>,
    auth_style: AuthStyle,
    pkce: PkceMethod,
}

impl AuthCodeBuilder {
    pub fn auth_url(mut self, url: impl Into<String>) -> Self {
        self.auth_url = url.into();
        self
    }

    pub fn token_url(mut self, url: impl Into<String>) -> Self {
        self.token_url = url.into();
        self
    }

    pub fn scopes<I, S>(mut self, scopes: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.scopes = scopes.into_iter().map(Into::into).collect();
        self
    }

    pub fn auth_style(mut self, style: AuthStyle) -> Self {
        self.auth_style = style;
        self
    }

    pub fn pkce(mut self, method: PkceMethod) -> Self {
        self.pkce = method;
        self
    }

    pub fn build(self) -> OAuth2Config {
        OAuth2Config {
            auth_url: self.auth_url,
            token_url: self.token_url,
            scopes: self.scopes,
            grant_type: GrantType::AuthorizationCode,
            auth_style: self.auth_style,
            pkce: Some(self.pkce),
            redirect_uri: Some(self.redirect_uri),
        }
    }
}

/// Builder for Client Credentials grant configs.
#[derive(Debug)]
pub struct ClientCredentialsBuilder {
    auth_url: String,
    token_url: String,
    scopes: Vec<String>,
    auth_style: AuthStyle,
}

impl ClientCredentialsBuilder {
    pub fn auth_url(mut self, url: impl Into<String>) -> Self {
        self.auth_url = url.into();
        self
    }

    pub fn token_url(mut self, url: impl Into<String>) -> Self {
        self.token_url = url.into();
        self
    }

    pub fn scopes<I, S>(mut self, scopes: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.scopes = scopes.into_iter().map(Into::into).collect();
        self
    }

    pub fn auth_style(mut self, style: AuthStyle) -> Self {
        self.auth_style = style;
        self
    }

    pub fn build(self) -> OAuth2Config {
        OAuth2Config {
            auth_url: self.auth_url,
            token_url: self.token_url,
            scopes: self.scopes,
            grant_type: GrantType::ClientCredentials,
            auth_style: self.auth_style,
            pkce: None,
            redirect_uri: None,
        }
    }
}

/// Builder for Device Code grant configs.
#[derive(Debug)]
pub struct DeviceCodeBuilder {
    auth_url: String,
    token_url: String,
    scopes: Vec<String>,
    auth_style: AuthStyle,
}

impl DeviceCodeBuilder {
    pub fn auth_url(mut self, url: impl Into<String>) -> Self {
        self.auth_url = url.into();
        self
    }

    pub fn token_url(mut self, url: impl Into<String>) -> Self {
        self.token_url = url.into();
        self
    }

    pub fn scopes<I, S>(mut self, scopes: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.scopes = scopes.into_iter().map(Into::into).collect();
        self
    }

    pub fn auth_style(mut self, style: AuthStyle) -> Self {
        self.auth_style = style;
        self
    }

    pub fn build(self) -> OAuth2Config {
        OAuth2Config {
            auth_url: self.auth_url,
            token_url: self.token_url,
            scopes: self.scopes,
            grant_type: GrantType::DeviceCode,
            auth_style: self.auth_style,
            pkce: None,
            redirect_uri: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CALLBACK: &str = "https://app.example.com/oauth2/callback";

    #[test]
    fn authorization_code_builder_sets_fields() {
        let config = OAuth2Config::authorization_code(CALLBACK)
            .auth_url("https://example.com/auth")
            .token_url("https://example.com/token")
            .build();
        assert_eq!(config.auth_url, "https://example.com/auth");
        assert_eq!(config.token_url, "https://example.com/token");
        assert_eq!(config.grant_type, GrantType::AuthorizationCode);
        assert_eq!(config.auth_style, AuthStyle::Header);
        assert_eq!(config.pkce, Some(PkceMethod::S256));
        assert_eq!(config.redirect_uri.as_deref(), Some(CALLBACK));
    }

    #[test]
    fn authorization_code_default_scopes_is_empty() {
        let config = OAuth2Config::authorization_code(CALLBACK)
            .auth_url("https://a.com/auth")
            .token_url("https://a.com/token")
            .build();
        assert!(config.scopes.is_empty());
    }

    #[test]
    fn authorization_code_scopes_can_be_set() {
        let config = OAuth2Config::authorization_code(CALLBACK)
            .auth_url("https://a.com/auth")
            .token_url("https://a.com/token")
            .scopes(["read", "write"])
            .build();
        assert_eq!(config.scopes, vec!["read", "write"]);
    }

    #[test]
    fn client_credentials_builder_omits_pkce_and_redirect() {
        let config = OAuth2Config::client_credentials()
            .auth_url("https://a.com/auth")
            .token_url("https://a.com/token")
            .build();
        assert_eq!(config.grant_type, GrantType::ClientCredentials);
        assert_eq!(config.pkce, None);
        assert_eq!(config.redirect_uri, None);
    }

    #[test]
    fn device_code_builder_omits_pkce_and_redirect() {
        let config = OAuth2Config::device_code()
            .auth_url("https://a.com/auth")
            .token_url("https://a.com/token")
            .build();
        assert_eq!(config.grant_type, GrantType::DeviceCode);
        assert_eq!(config.pkce, None);
        assert_eq!(config.redirect_uri, None);
    }

    #[test]
    fn pkce_method_s256_is_default_on_auth_code_builder() {
        let config = OAuth2Config::authorization_code(CALLBACK)
            .auth_url("https://a.com/auth")
            .token_url("https://a.com/token")
            .build();
        assert_eq!(config.pkce, Some(PkceMethod::S256));
    }

    #[test]
    fn post_body_auth_style() {
        let config = OAuth2Config::authorization_code(CALLBACK)
            .auth_url("https://github.com/login/oauth/authorize")
            .token_url("https://github.com/login/oauth/access_token")
            .auth_style(AuthStyle::PostBody)
            .build();
        assert_eq!(config.auth_style, AuthStyle::PostBody);
    }
}
