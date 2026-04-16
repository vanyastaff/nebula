//! Request signing credentials (HMAC, SigV4, webhook signatures).

use serde::{Deserialize, Serialize};

use crate::{AuthScheme, SecretString};

/// A signing key used to authenticate requests via HMAC or similar algorithms.
///
/// Covers HMAC-SHA256, AWS SigV4, webhook signature secrets, and other
/// request-signing mechanisms where a shared secret is used to compute
/// a signature over request data.
///
/// # Examples
///
/// ```
/// use nebula_credential::{SecretString, scheme::SigningKey};
///
/// let key = SigningKey::new(SecretString::new("whsec_abc123"), "hmac-sha256");
/// ```
#[derive(Clone, Serialize, Deserialize, AuthScheme)]
#[auth_scheme(pattern = RequestSigning)]
pub struct SigningKey {
    #[serde(with = "crate::serde_secret")]
    key: SecretString,
    algorithm: String,
}

impl SigningKey {
    /// Creates a new signing key with the given secret and algorithm.
    #[must_use]
    pub fn new(key: SecretString, algorithm: impl Into<String>) -> Self {
        Self {
            key,
            algorithm: algorithm.into(),
        }
    }

    /// Returns the signing key secret.
    pub fn key(&self) -> &SecretString {
        &self.key
    }

    /// Returns the algorithm identifier (e.g., `"hmac-sha256"`, `"sigv4"`).
    pub fn algorithm(&self) -> &str {
        &self.algorithm
    }
}

impl std::fmt::Debug for SigningKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SigningKey")
            .field("key", &"[REDACTED]")
            .field("algorithm", &self.algorithm)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use nebula_core::{AuthPattern, AuthScheme as _};

    use super::*;

    #[test]
    fn pattern_is_request_signing() {
        assert_eq!(SigningKey::pattern(), AuthPattern::RequestSigning);
    }

    #[test]
    fn debug_redacts_key() {
        let key = SigningKey::new(SecretString::new("super-secret-key"), "hmac-sha256");
        let debug = format!("{key:?}");
        assert!(debug.contains("hmac-sha256"));
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("super-secret-key"));
    }
}
