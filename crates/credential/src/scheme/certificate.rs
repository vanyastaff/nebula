//! Client certificate (mTLS) authentication.

use nebula_core::AuthScheme;
use serde::{Deserialize, Serialize};

use crate::utils::SecretString;

/// Client certificate authentication for mutual TLS.
///
/// Produced by: mTLS credential configurations.
/// Consumed by: HTTPS APIs, gRPC, internal service mesh.
#[derive(Clone, Serialize, Deserialize)]
pub struct CertificateAuth {
    /// PEM-encoded client certificate.
    cert_pem: SecretString,
    /// PEM-encoded private key.
    key_pem: SecretString,
    /// Optional PEM-encoded CA certificate for verification.
    pub ca_pem: Option<String>,
}

impl CertificateAuth {
    /// Creates a new certificate auth with client cert and private key.
    pub fn new(cert_pem: SecretString, key_pem: SecretString) -> Self {
        Self {
            cert_pem,
            key_pem,
            ca_pem: None,
        }
    }

    /// Sets the CA certificate for verification.
    pub fn with_ca_pem(mut self, ca_pem: impl Into<String>) -> Self {
        self.ca_pem = Some(ca_pem.into());
        self
    }

    /// Returns the PEM-encoded client certificate.
    pub fn cert_pem(&self) -> &SecretString {
        &self.cert_pem
    }

    /// Returns the PEM-encoded private key.
    pub fn key_pem(&self) -> &SecretString {
        &self.key_pem
    }
}

impl AuthScheme for CertificateAuth {
    const KIND: &'static str = "certificate";
}

impl std::fmt::Debug for CertificateAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CertificateAuth")
            .field("cert_pem", &"[REDACTED]")
            .field("key_pem", &"[REDACTED]")
            .field("ca_pem", &self.ca_pem.as_deref().map(|_| "[PRESENT]"))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_is_correct() {
        assert_eq!(CertificateAuth::KIND, "certificate");
    }

    #[test]
    fn debug_redacts_secrets() {
        let auth = CertificateAuth::new(
            SecretString::new("-----BEGIN CERTIFICATE-----"),
            SecretString::new("-----BEGIN PRIVATE KEY-----"),
        )
        .with_ca_pem("-----BEGIN CA-----");
        let debug = format!("{auth:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("-----BEGIN CERTIFICATE-----"));
        assert!(!debug.contains("-----BEGIN PRIVATE KEY-----"));
        assert!(!debug.contains("-----BEGIN CA-----"));
    }

    #[test]
    fn accessors_return_secrets() {
        let auth = CertificateAuth::new(SecretString::new("cert"), SecretString::new("key"));
        auth.cert_pem().expose_secret(|v| assert_eq!(v, "cert"));
        auth.key_pem().expose_secret(|v| assert_eq!(v, "key"));
    }
}
