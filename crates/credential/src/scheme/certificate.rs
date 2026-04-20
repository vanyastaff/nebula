//! X.509 client certificate authentication (mTLS, TLS client auth).

use serde::{Deserialize, Serialize};

use crate::{AuthScheme, SecretString};

/// X.509 client certificate with private key for mutual TLS authentication.
///
/// Produced by: mTLS credential configurations.
/// Consumed by: HTTPS APIs requiring client certificates, gRPC, internal
/// service mesh, and any TLS endpoint that validates client identity.
///
/// # Examples
///
/// ```
/// use nebula_credential::{SecretString, scheme::Certificate};
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
    #[serde(with = "crate::serde_secret")]
    private_key: SecretString,
    #[serde(with = "crate::serde_secret::option")]
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

    #[test]
    fn serde_roundtrip_preserves_secrets() {
        // Certificate uses `#[serde(with = "serde_secret")]` which preserves
        // the actual value (unlike the default SecretString Serialize that
        // writes "[REDACTED]"). This test verifies that contract.
        let original = Certificate::new(
            "-----BEGIN CERTIFICATE-----\nMIIB...",
            SecretString::new("-----BEGIN PRIVATE KEY-----\nMIIE..."),
        )
        .with_passphrase(SecretString::new("hunter2"));

        let json = serde_json::to_string(&original).expect("Certificate must serialize");
        let decoded: Certificate =
            serde_json::from_str(&json).expect("Certificate must deserialize");

        assert_eq!(decoded.cert_chain(), original.cert_chain());
        decoded
            .private_key()
            .expose_secret(|v| assert_eq!(v, "-----BEGIN PRIVATE KEY-----\nMIIE..."));
        decoded
            .passphrase()
            .expect("passphrase must survive roundtrip")
            .expose_secret(|v| assert_eq!(v, "hunter2"));
    }
}
