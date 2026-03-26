//! Database connection authentication.

use nebula_core::AuthScheme;
use serde::{Deserialize, Serialize};

use crate::utils::SecretString;

/// Full database connection auth material.
///
/// Produced by: Database credential, Vault dynamic secrets.
/// Consumed by: Postgres, MySQL, MongoDB resources.
#[derive(Clone, Serialize, Deserialize)]
pub struct DatabaseAuth {
    /// Database host.
    pub host: String,
    /// Connection port.
    pub port: u16,
    /// Database name.
    pub database: String,
    /// Username for authentication.
    pub username: String,
    password: SecretString,
    /// SSL/TLS mode for the connection.
    pub ssl_mode: SslMode,
}

/// SSL/TLS mode for database connections.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[non_exhaustive]
pub enum SslMode {
    /// SSL disabled.
    Disabled,
    /// Prefer SSL but allow plaintext.
    #[default]
    Prefer,
    /// Require SSL.
    Require,
    /// Require SSL and verify CA certificate.
    VerifyCa,
    /// Require SSL and verify both CA and hostname.
    VerifyFull,
}

impl DatabaseAuth {
    /// Creates a new database auth with default SSL mode (`Prefer`).
    pub fn new(
        host: impl Into<String>,
        port: u16,
        database: impl Into<String>,
        username: impl Into<String>,
        password: SecretString,
    ) -> Self {
        Self {
            host: host.into(),
            port,
            database: database.into(),
            username: username.into(),
            password,
            ssl_mode: SslMode::default(),
        }
    }

    /// Sets the SSL mode for the connection.
    pub fn with_ssl_mode(mut self, mode: SslMode) -> Self {
        self.ssl_mode = mode;
        self
    }

    /// Returns the password secret.
    pub fn password(&self) -> &SecretString {
        &self.password
    }
}

impl AuthScheme for DatabaseAuth {}

impl std::fmt::Debug for DatabaseAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DatabaseAuth")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("database", &self.database)
            .field("username", &self.username)
            .field("password", &"[REDACTED]")
            .field("ssl_mode", &self.ssl_mode)
            .finish()
    }
}
