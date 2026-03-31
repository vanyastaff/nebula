//! OAuth2 provider configuration — built via builder, const-friendly.

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

/// Provider-specific OAuth2 configuration.
///
/// Build via [`OAuth2Config::authorization_code()`] or other constructors.
/// Generated as a const by the `#[oauth2(...)]` macro attribute.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuth2Config {
    pub auth_url: String,
    pub token_url: String,
    pub scopes: Vec<String>,
    pub grant_type: GrantType,
    pub auth_style: AuthStyle,
    pub pkce: bool,
}

impl OAuth2Config {
    /// Start building an Authorization Code flow config.
    pub fn authorization_code() -> OAuth2ConfigBuilder {
        OAuth2ConfigBuilder::new(GrantType::AuthorizationCode)
    }

    /// Start building a Client Credentials flow config.
    pub fn client_credentials() -> OAuth2ConfigBuilder {
        OAuth2ConfigBuilder::new(GrantType::ClientCredentials)
    }

    /// Start building a Device Code flow config.
    pub fn device_code() -> OAuth2ConfigBuilder {
        OAuth2ConfigBuilder::new(GrantType::DeviceCode)
    }
}

/// Builder for [`OAuth2Config`].
#[derive(Debug)]
pub struct OAuth2ConfigBuilder {
    grant_type: GrantType,
    auth_url: String,
    token_url: String,
    scopes: Vec<String>,
    auth_style: AuthStyle,
    pkce: bool,
}

impl OAuth2ConfigBuilder {
    fn new(grant_type: GrantType) -> Self {
        Self {
            grant_type,
            auth_url: String::new(),
            token_url: String::new(),
            scopes: Vec::new(),
            auth_style: AuthStyle::Header,
            pkce: false,
        }
    }

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

    pub fn pkce(mut self, enabled: bool) -> Self {
        self.pkce = enabled;
        self
    }

    pub fn build(self) -> OAuth2Config {
        OAuth2Config {
            auth_url: self.auth_url,
            token_url: self.token_url,
            scopes: self.scopes,
            grant_type: self.grant_type,
            auth_style: self.auth_style,
            pkce: self.pkce,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_sets_auth_and_token_urls() {
        let config = OAuth2Config::authorization_code()
            .auth_url("https://example.com/auth")
            .token_url("https://example.com/token")
            .build();
        assert_eq!(config.auth_url, "https://example.com/auth");
        assert_eq!(config.token_url, "https://example.com/token");
        assert_eq!(config.grant_type, GrantType::AuthorizationCode);
        assert_eq!(config.auth_style, AuthStyle::Header);
        assert!(!config.pkce);
    }

    #[test]
    fn default_scopes_is_empty() {
        let config = OAuth2Config::authorization_code()
            .auth_url("https://a.com/auth")
            .token_url("https://a.com/token")
            .build();
        assert!(config.scopes.is_empty());
    }

    #[test]
    fn scopes_can_be_set() {
        let config = OAuth2Config::authorization_code()
            .auth_url("https://a.com/auth")
            .token_url("https://a.com/token")
            .scopes(["read", "write"])
            .build();
        assert_eq!(config.scopes, vec!["read", "write"]);
    }

    #[test]
    fn client_credentials_grant_type() {
        let config = OAuth2Config::client_credentials()
            .auth_url("https://a.com/auth")
            .token_url("https://a.com/token")
            .build();
        assert_eq!(config.grant_type, GrantType::ClientCredentials);
    }

    #[test]
    fn post_body_auth_style() {
        let config = OAuth2Config::authorization_code()
            .auth_url("https://github.com/login/oauth/authorize")
            .token_url("https://github.com/login/oauth/access_token")
            .auth_style(AuthStyle::PostBody)
            .build();
        assert_eq!(config.auth_style, AuthStyle::PostBody);
    }
}
