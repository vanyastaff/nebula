//! OAuth2 token -- consumer-facing (no refresh internals).

use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::{AuthPattern, AuthScheme, SecretString, scheme::SensitiveScheme};

/// OAuth2 bearer token with metadata.
///
/// This is the consumer-facing scheme -- it does NOT contain `refresh_token`
/// or `client_secret`. Those stay in `OAuth2State` ([`CredentialState`]).
///
/// Produced by: OAuth2 credential via `project()`.
/// Consumed by: HTTP APIs requiring OAuth2 bearer auth.
///
/// Per Tech Spec §15.5 — `SensitiveScheme`: access token is the secret;
/// token type, scopes, expiry are non-secret metadata.
///
/// [`CredentialState`]: crate::CredentialState
#[derive(Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct OAuth2Token {
    #[serde(with = "crate::serde_secret")]
    access_token: SecretString,
    /// Token type (typically `"Bearer"`).
    #[zeroize(skip)]
    pub token_type: String,
    /// Granted scopes.
    #[zeroize(skip)]
    pub scopes: Vec<String>,
    /// When the access token expires, if known.
    #[zeroize(skip)]
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
    ///
    /// Per Tech Spec §15.5 (closes security-lead N4): the bearer header
    /// contains the access token verbatim; returning `SecretString` forces
    /// `.expose_secret()` at the FFI boundary, eliminating accidental
    /// `Debug` / log leaks of the bearer string.
    ///
    /// SEC-09 (security hardening 2026-04-27 Stage 2): construction goes
    /// through a `Zeroizing<String>` buffer instead of `format!` so that
    /// any panic during string assembly zeros the partial bearer.
    #[must_use]
    pub fn bearer_header(&self) -> SecretString {
        let token = self.access_token.expose_secret();
        // capacity = token_type + " " + token
        let mut buf = zeroize::Zeroizing::new(String::with_capacity(
            self.token_type.len() + 1 + token.len(),
        ));
        buf.push_str(&self.token_type);
        buf.push(' ');
        buf.push_str(token);
        SecretString::new(std::mem::take(&mut *buf))
    }

    /// Returns `true` if the token has expired.
    pub fn is_expired(&self) -> bool {
        self.expires_at.is_some_and(|at| at <= chrono::Utc::now())
    }
}

impl AuthScheme for OAuth2Token {
    fn pattern() -> AuthPattern {
        AuthPattern::OAuth2
    }
}

impl SensitiveScheme for OAuth2Token {}

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
