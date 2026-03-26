//! HTTP Basic authentication.

use nebula_core::AuthScheme;
use serde::{Deserialize, Serialize};

use crate::utils::SecretString;

/// Username + password for HTTP Basic Auth.
///
/// Produced by: Basic auth credential.
/// Consumed by: HTTP APIs, SMTP, proxies.
#[derive(Clone, Serialize, Deserialize)]
pub struct BasicAuth {
    /// Username (not secret).
    pub username: String,
    /// Password (secret).
    password: SecretString,
}

impl BasicAuth {
    /// Creates a new basic auth with the given username and password.
    pub fn new(username: impl Into<String>, password: SecretString) -> Self {
        Self {
            username: username.into(),
            password,
        }
    }

    /// Returns the password secret.
    pub fn password(&self) -> &SecretString {
        &self.password
    }

    /// Formats as base64-encoded `username:password` for the Authorization header.
    pub fn basic_header(&self) -> String {
        use base64::Engine;
        self.password.expose_secret(|p| {
            let encoded = base64::engine::general_purpose::STANDARD
                .encode(format!("{}:{p}", self.username));
            format!("Basic {encoded}")
        })
    }
}

impl AuthScheme for BasicAuth {}

impl std::fmt::Debug for BasicAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BasicAuth")
            .field("username", &self.username)
            .field("password", &"[REDACTED]")
            .finish()
    }
}
