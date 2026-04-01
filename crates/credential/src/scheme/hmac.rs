//! HMAC-based authentication.

use nebula_core::AuthScheme;
use serde::{Deserialize, Serialize};

use nebula_core::SecretString;

/// HMAC signing secret for request/webhook authentication.
///
/// Produced by: HMAC credential configurations.
/// Consumed by: Webhook signature verification, API request signing.
#[derive(Clone, Serialize, Deserialize)]
pub struct HmacSecret {
    /// The signing secret.
    #[serde(with = "nebula_core::serde_secret")]
    secret: SecretString,
    /// Hash algorithm (e.g., `"sha256"`, `"sha512"`).
    pub algorithm: String,
}

impl HmacSecret {
    /// Creates a new HMAC secret with the given signing key and algorithm.
    pub fn new(secret: SecretString, algorithm: impl Into<String>) -> Self {
        Self {
            secret,
            algorithm: algorithm.into(),
        }
    }

    /// Returns the signing secret.
    pub fn secret(&self) -> &SecretString {
        &self.secret
    }

    /// Exposes the raw secret for signing operations.
    pub fn expose_secret<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&str) -> R,
    {
        self.secret.expose_secret(f)
    }
}

impl AuthScheme for HmacSecret {
    const KIND: &'static str = "hmac";
}

impl std::fmt::Debug for HmacSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HmacSecret")
            .field("secret", &"[REDACTED]")
            .field("algorithm", &self.algorithm)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_is_correct() {
        assert_eq!(HmacSecret::KIND, "hmac");
    }

    #[test]
    fn debug_redacts_secrets() {
        let hmac = HmacSecret::new(SecretString::new("my-secret"), "sha256");
        let debug = format!("{hmac:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("my-secret"));
    }

    #[test]
    fn expose_secret_returns_value() {
        let hmac = HmacSecret::new(SecretString::new("key123"), "sha256");
        let result = hmac.expose_secret(|s| s.to_owned());
        assert_eq!(result, "key123");
    }
}
