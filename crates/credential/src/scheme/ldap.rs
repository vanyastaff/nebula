//! LDAP authentication.

use nebula_core::AuthScheme;
use serde::{Deserialize, Serialize};

use nebula_core::SecretString;

/// LDAP directory authentication material.
///
/// Produced by: LDAP credential configurations.
/// Consumed by: LDAP directory resources, Active Directory integrations.
#[derive(Clone, Serialize, Deserialize)]
pub struct LdapAuth {
    /// LDAP server host.
    pub host: String,
    /// LDAP server port (typically 389 or 636).
    pub port: u16,
    /// TLS mode for the connection.
    pub tls_mode: LdapTlsMode,
    /// Base DN for searches (e.g., `"dc=example,dc=com"`).
    pub base_dn: Option<String>,
    /// Bind method for authentication.
    pub bind_method: LdapBindMethod,
}

/// TLS mode for LDAP connections.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[non_exhaustive]
pub enum LdapTlsMode {
    /// No TLS (plaintext).
    #[default]
    None,
    /// Upgrade to TLS via STARTTLS after connecting.
    StartTls,
    /// Connect directly over TLS (LDAPS, port 636).
    Ldaps,
}

/// LDAP bind method for authentication.
#[derive(Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub enum LdapBindMethod {
    /// Simple bind with DN and password.
    Simple {
        /// Distinguished name to bind as.
        bind_dn: String,
        /// Bind password.
        #[serde(with = "nebula_core::serde_secret")]
        password: SecretString,
    },
    /// Anonymous bind (no credentials).
    Anonymous,
}

impl LdapAuth {
    /// Creates a new LDAP auth with simple bind.
    pub fn simple(
        host: impl Into<String>,
        port: u16,
        bind_dn: impl Into<String>,
        password: SecretString,
    ) -> Self {
        Self {
            host: host.into(),
            port,
            tls_mode: LdapTlsMode::default(),
            base_dn: None,
            bind_method: LdapBindMethod::Simple {
                bind_dn: bind_dn.into(),
                password,
            },
        }
    }

    /// Creates a new LDAP auth with anonymous bind.
    pub fn anonymous(host: impl Into<String>, port: u16) -> Self {
        Self {
            host: host.into(),
            port,
            tls_mode: LdapTlsMode::default(),
            base_dn: None,
            bind_method: LdapBindMethod::Anonymous,
        }
    }

    /// Sets the TLS mode.
    pub fn with_tls_mode(mut self, mode: LdapTlsMode) -> Self {
        self.tls_mode = mode;
        self
    }

    /// Sets the base DN.
    pub fn with_base_dn(mut self, base_dn: impl Into<String>) -> Self {
        self.base_dn = Some(base_dn.into());
        self
    }
}

impl AuthScheme for LdapAuth {
    const KIND: &'static str = "ldap";
}

impl std::fmt::Debug for LdapAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LdapAuth")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("tls_mode", &self.tls_mode)
            .field("base_dn", &self.base_dn)
            .field("bind_method", &self.bind_method)
            .finish()
    }
}

impl std::fmt::Debug for LdapBindMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Simple { bind_dn, .. } => f
                .debug_struct("Simple")
                .field("bind_dn", bind_dn)
                .field("password", &"[REDACTED]")
                .finish(),
            Self::Anonymous => write!(f, "Anonymous"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_is_correct() {
        assert_eq!(LdapAuth::KIND, "ldap");
    }

    #[test]
    fn debug_redacts_password() {
        let auth = LdapAuth::simple(
            "ldap.example.com",
            389,
            "cn=admin,dc=example,dc=com",
            SecretString::new("admin-pass"),
        )
        .with_tls_mode(LdapTlsMode::StartTls)
        .with_base_dn("dc=example,dc=com");
        let debug = format!("{auth:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("admin-pass"));
        assert!(debug.contains("cn=admin"));
    }

    #[test]
    fn debug_anonymous_has_no_secrets() {
        let auth = LdapAuth::anonymous("ldap.example.com", 389);
        let debug = format!("{auth:?}");
        assert!(debug.contains("Anonymous"));
    }
}
