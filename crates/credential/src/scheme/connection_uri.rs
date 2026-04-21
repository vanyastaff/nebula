//! Compound connection URI authentication (postgres://, redis://, etc.).

use serde::{Deserialize, Serialize};

use crate::{AuthScheme, SecretString};

/// A connection URI that encodes all credentials and connection parameters.
///
/// Covers database connection strings (`postgres://user:pass@host/db`),
/// cache URIs (`redis://:token@host`), message broker URIs, and any other
/// service where a single URI is the complete authentication material.
///
/// The URI is treated as a secret because it typically embeds credentials.
///
/// # Examples
///
/// ```
/// use nebula_credential::{SecretString, scheme::ConnectionUri};
///
/// let uri = ConnectionUri::new(SecretString::new(
///     "postgres://alice:secret@db.example.com/mydb",
/// ));
/// ```
#[derive(Clone, Serialize, Deserialize, AuthScheme)]
#[auth_scheme(pattern = ConnectionUri)]
pub struct ConnectionUri {
    #[serde(with = "crate::serde_secret")]
    uri: SecretString,
}

impl ConnectionUri {
    /// Creates a new connection URI credential.
    #[must_use]
    pub fn new(uri: SecretString) -> Self {
        Self { uri }
    }

    /// Returns the connection URI secret.
    pub fn uri(&self) -> &SecretString {
        &self.uri
    }
}

impl std::fmt::Debug for ConnectionUri {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectionUri")
            .field("uri", &"[REDACTED]")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AuthPattern;

    #[test]
    fn pattern_is_connection_uri() {
        assert_eq!(ConnectionUri::pattern(), AuthPattern::ConnectionUri);
    }

    #[test]
    fn debug_redacts_uri() {
        let uri = ConnectionUri::new(SecretString::new(
            "postgres://alice:password123@db.example.com/prod",
        ));
        let debug = format!("{uri:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("password123"));
        assert!(!debug.contains("postgres://"));
    }
}
