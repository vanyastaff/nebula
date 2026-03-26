//! Bearer token authentication.

use nebula_core::AuthScheme;
use serde::{Deserialize, Serialize};

use crate::utils::SecretString;

/// Bearer token for HTTP Authorization header.
///
/// Produced by: API Key, OAuth2, Service Account, SAML bearer.
/// Consumed by: HTTP APIs (GitHub, Slack, OpenAI, etc.)
#[derive(Clone, Serialize, Deserialize)]
pub struct BearerToken {
    token: SecretString,
}

impl BearerToken {
    /// Creates a new bearer token.
    pub fn new(token: SecretString) -> Self {
        Self { token }
    }

    /// Returns the token value for use in headers.
    ///
    /// Use sparingly -- prefer [`bearer_header`](Self::bearer_header)
    /// which formats the full Authorization header value.
    pub fn expose(&self) -> &SecretString {
        &self.token
    }

    /// Formats as `Bearer <token>` for the Authorization header.
    pub fn bearer_header(&self) -> String {
        self.token.expose_secret(|t| format!("Bearer {t}"))
    }
}

impl AuthScheme for BearerToken {}

impl std::fmt::Debug for BearerToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BearerToken")
            .field("token", &"[REDACTED]")
            .finish()
    }
}
