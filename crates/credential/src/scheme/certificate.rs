//! X.509 client certificate authentication (mTLS, TLS client auth).

use nebula_core::SecretString;

use crate::AuthScheme;
use serde::{Deserialize, Serialize};

/// X.509 client certificate with private key for mutual TLS authentication.
///
/// Produced by: mTLS credential configurations.
/// Consumed by: HTTPS APIs requiring client certificates, gRPC, internal
/// service mesh, and any TLS endpoint that validates client identity.
///
/// # Examples
///
/// ```
/// use nebula_credential::scheme::Certificate;
/// use nebula_core::SecretString;
///
/// let cert = Certificate::new(
///     "-----BEGIN CERTIFICATE-----\n...".to_string(),
///     SecretString::new("-----BEGIN PRIVATE KEY-----\n..."),
/// );
/// ```
#[derive(Clone, Serialize, Deserialize, AuthScheme)]
#[auth_scheme(pattern = Certificate)]
pub struct Certificate {
    cert_chain: String,
    #[serde(with = "nebula_core::serde_secret")]
    private_key: SecretString,
    #[serde(with = "nebula_core::option_serde_secret")]
    passphrase: Option<SecretString>,
}

impl Certificate {
    /// Creates a new certificate credential with a cert chain and private key.
    #[must_use]
    pub fn new(cert_chain: impl Into<String>, private_key: SecretString) -> Self {
        Self {
            cert_chain: cert_chain.into(),
            private_key,
            passphrase: None,
        }
    }

    /// Sets the passphrase protecting the private key.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_passphrase(mut self, passphrase: SecretString) -> Self {
        self.passphrase = Some(passphrase);
        self
    }

    /// Returns the PEM-encoded certificate chain.
    pub fn cert_chain(&self) -> &str {
        &self.cert_chain
    }

    /// Returns the PEM-encoded private key secret.
    pub fn private_key(&self) -> &SecretString {
        &self.private_key
    }

    /// Returns the optional passphrase protecting the private key.
    pub fn passphrase(&self) -> Option<&SecretString> {
        self.passphrase.as_ref()
    }
}

impl std::fmt::Debug for Certificate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Certificate")
            .field(
                "cert_chain",
                &format_args!("[{} bytes]", self.cert_chain.len()),
            )
            .field("private_key", &"[REDACTED]")
            .field(
                "passphrase",
                &self.passphrase.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use nebula_core::{AuthPattern, AuthScheme as _};

    use super::*;

    #[test]
    fn pattern_is_certificate() {
        assert_eq!(Certificate::pattern(), AuthPattern::Certificate);
    }

    #[test]
    fn debug_redacts_secrets() {
        let cert = Certificate::new(
            "-----BEGIN CERTIFICATE-----",
            SecretString::new("-----BEGIN PRIVATE KEY-----"),
        )
        .with_passphrase(SecretString::new("my-passphrase"));
        let debug = format!("{cert:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("-----BEGIN PRIVATE KEY-----"));
        assert!(!debug.contains("my-passphrase"));
    }

    #[test]
    fn accessors_return_values() {
        let cert = Certificate::new("-----BEGIN CERTIFICATE-----", SecretString::new("key-data"));
        assert_eq!(cert.cert_chain(), "-----BEGIN CERTIFICATE-----");
        cert.private_key()
            .expose_secret(|v| assert_eq!(v, "key-data"));
        assert!(cert.passphrase().is_none());
    }
}
