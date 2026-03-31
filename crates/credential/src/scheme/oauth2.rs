//! OAuth2 token -- consumer-facing (no refresh internals).

use nebula_core::AuthScheme;
use serde::{Deserialize, Serialize};

use crate::utils::SecretString;

/// OAuth2 bearer token with metadata.
///
/// This is the consumer-facing scheme -- it does NOT contain `refresh_token`
/// or `client_secret`. Those stay in `OAuth2State` ([`CredentialState`]).
///
/// Produced by: OAuth2 credential via `project()`.
/// Consumed by: HTTP APIs requiring OAuth2 bearer auth.
///
/// [`CredentialState`]: crate::state::CredentialState
#[derive(Clone, Serialize, Deserialize)]
pub struct OAuth2Token {
    access_token: SecretString,
    /// Token type (typically `"Bearer"`).
    pub token_type: String,
    /// Granted scopes.
    pub scopes: Vec<String>,
    /// When the access token expires, if known.
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl OAuth2Token {
    /// Creates a new OAuth2 token with default type `"Bearer"`.
    pub fn new(access_token: SecretString) -> Self {
        Self {
            access_token,
            token_type: "Bearer".into(),
            scopes: Vec::new(),
            expires_at: None,
        }
    }

    /// Sets the granted scopes.
    pub fn with_scopes(mut self, scopes: Vec<String>) -> Self {
        self.scopes = scopes;
        self
    }

    /// Sets the token expiration time.
    pub fn with_expires_at(mut self, at: chrono::DateTime<chrono::Utc>) -> Self {
        self.expires_at = Some(at);
        self
    }

    /// Returns the access token secret.
    pub fn access_token(&self) -> &SecretString {
        &self.access_token
    }

    /// Formats as `<token_type> <token>` for the Authorization header.
    pub fn bearer_header(&self) -> String {
        self.access_token
            .expose_secret(|t| format!("{} {t}", self.token_type))
    }

    /// Returns `true` if the token has expired.
    pub fn is_expired(&self) -> bool {
        self.expires_at.is_some_and(|at| at <= chrono::Utc::now())
    }
}

impl AuthScheme for OAuth2Token {
    const KIND: &'static str = "oauth2";

    fn expires_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        self.expires_at
    }
}

impl std::fmt::Debug for OAuth2Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuth2Token")
            .field("access_token", &"[REDACTED]")
            .field("token_type", &self.token_type)
            .field("scopes", &self.scopes)
            .field("expires_at", &self.expires_at)
            .finish()
    }
}
