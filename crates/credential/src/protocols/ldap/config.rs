//! LDAP-specific configuration types.

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// TLS mode for LDAP connections.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TlsMode {
    /// Plaintext connection (development only — not recommended for production).
    #[default]
    None,
    /// TLS from connection start (`ldaps://`, default port 636).
    Tls,
    /// STARTTLS upgrade on a plaintext connection (port 389).
    StartTls,
}

/// LDAP-specific configuration passed via `#[credential(extends = LdapProtocol)]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LdapConfig {
    /// Transport security mode.
    pub tls: TlsMode,
    /// Connection timeout.
    #[serde(with = "humantime_serde")]
    pub timeout: Duration,
    /// Optional PEM-encoded CA certificate for TLS verification.
    pub ca_cert: Option<String>,
}

impl Default for LdapConfig {
    fn default() -> Self {
        Self {
            tls: TlsMode::None,
            timeout: Duration::from_secs(30),
            ca_cert: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_tls_mode_is_none() {
        assert_eq!(TlsMode::default(), TlsMode::None);
    }

    #[test]
    fn default_config_timeout_is_30s() {
        let config = LdapConfig::default();
        assert_eq!(config.timeout, Duration::from_secs(30));
        assert_eq!(config.tls, TlsMode::None);
        assert!(config.ca_cert.is_none());
    }
}
