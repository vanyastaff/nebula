//! SSH authentication.

use nebula_core::AuthScheme;
use serde::{Deserialize, Serialize};

use crate::utils::SecretString;

/// SSH connection authentication material.
///
/// Produced by: SSH credential configurations.
/// Consumed by: SSH/SFTP resources, Git-over-SSH.
#[derive(Clone, Serialize, Deserialize)]
pub struct SshAuth {
    /// Remote host.
    pub host: String,
    /// Connection port (typically 22).
    pub port: u16,
    /// Username for authentication.
    pub username: String,
    /// Authentication method.
    pub method: SshAuthMethod,
}

/// SSH authentication method.
#[derive(Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub enum SshAuthMethod {
    /// Password-based authentication.
    Password {
        /// The password secret.
        password: SecretString,
    },
    /// Public/private key pair authentication.
    KeyPair {
        /// PEM-encoded private key.
        private_key: SecretString,
        /// Optional passphrase for the private key.
        passphrase: Option<SecretString>,
    },
    /// SSH agent forwarding (no local secrets).
    Agent,
}

impl SshAuth {
    /// Creates a new SSH auth with password authentication.
    pub fn with_password(
        host: impl Into<String>,
        port: u16,
        username: impl Into<String>,
        password: SecretString,
    ) -> Self {
        Self {
            host: host.into(),
            port,
            username: username.into(),
            method: SshAuthMethod::Password { password },
        }
    }

    /// Creates a new SSH auth with key pair authentication.
    pub fn with_key_pair(
        host: impl Into<String>,
        port: u16,
        username: impl Into<String>,
        private_key: SecretString,
        passphrase: Option<SecretString>,
    ) -> Self {
        Self {
            host: host.into(),
            port,
            username: username.into(),
            method: SshAuthMethod::KeyPair {
                private_key,
                passphrase,
            },
        }
    }

    /// Creates a new SSH auth using the SSH agent.
    pub fn with_agent(host: impl Into<String>, port: u16, username: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            port,
            username: username.into(),
            method: SshAuthMethod::Agent,
        }
    }
}

impl AuthScheme for SshAuth {
    const KIND: &'static str = "ssh";
}

impl std::fmt::Debug for SshAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SshAuth")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("method", &self.method)
            .finish()
    }
}

impl std::fmt::Debug for SshAuthMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Password { .. } => f
                .debug_struct("Password")
                .field("password", &"[REDACTED]")
                .finish(),
            Self::KeyPair { passphrase, .. } => f
                .debug_struct("KeyPair")
                .field("private_key", &"[REDACTED]")
                .field(
                    "passphrase",
                    if passphrase.is_some() {
                        &"Some([REDACTED])"
                    } else {
                        &"None"
                    },
                )
                .finish(),
            Self::Agent => write!(f, "Agent"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_is_correct() {
        assert_eq!(SshAuth::KIND, "ssh");
    }

    #[test]
    fn debug_redacts_password() {
        let auth =
            SshAuth::with_password("example.com", 22, "root", SecretString::new("s3cr3t-pw"));
        let debug = format!("{auth:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("s3cr3t-pw"));
    }

    #[test]
    fn debug_redacts_key_pair() {
        let auth = SshAuth::with_key_pair(
            "example.com",
            22,
            "root",
            SecretString::new("-----BEGIN RSA-----"),
            Some(SecretString::new("my-secret-phrase")),
        );
        let debug = format!("{auth:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("-----BEGIN RSA-----"));
        assert!(!debug.contains("my-secret-phrase"));
    }

    #[test]
    fn debug_agent_has_no_secrets() {
        let auth = SshAuth::with_agent("example.com", 22, "root");
        let debug = format!("{auth:?}");
        assert!(debug.contains("Agent"));
    }
}
