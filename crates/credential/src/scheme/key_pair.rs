//! Asymmetric key pair authentication (SSH, PGP, crypto wallets).

use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::{AuthScheme, SecretString};

/// Asymmetric key pair with optional passphrase and algorithm hint.
///
/// Covers SSH key pairs, PGP keys, ECDSA/RSA signing keys, and other
/// public/private key authentication mechanisms.
///
/// Per Tech Spec §15.5 — `SensitiveScheme`: private key + optional
/// passphrase are secret; public key + algorithm hint are non-secret
/// metadata.
///
/// # Examples
///
/// ```
/// use nebula_credential::{SecretString, scheme::KeyPair};
///
/// let kp = KeyPair::new(
///     "ssh-ed25519 AAAA...",
///     SecretString::new("-----BEGIN OPENSSH PRIVATE KEY-----..."),
/// )
/// .with_algorithm("ed25519");
/// ```
#[derive(Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop, AuthScheme)]
#[auth_scheme(pattern = KeyPair, sensitive)]
pub struct KeyPair {
    #[zeroize(skip)]
    public_key: String,
    #[serde(with = "crate::serde_secret")]
    private_key: SecretString,
    #[serde(default, with = "crate::serde_secret::option")]
    passphrase: Option<SecretString>,
    #[zeroize(skip)]
    algorithm: Option<String>,
}

impl KeyPair {
    /// Creates a new key pair with the given public and private key.
    #[must_use]
    pub fn new(public_key: impl Into<String>, private_key: SecretString) -> Self {
        Self {
            public_key: public_key.into(),
            private_key,
            passphrase: None,
            algorithm: None,
        }
    }

    /// Sets the passphrase protecting the private key.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_passphrase(mut self, passphrase: SecretString) -> Self {
        self.passphrase = Some(passphrase);
        self
    }

    /// Sets the algorithm identifier (e.g., `"ed25519"`, `"rsa"`, `"ecdsa"`).
    #[must_use = "builder methods must be chained or built"]
    pub fn with_algorithm(mut self, algorithm: impl Into<String>) -> Self {
        self.algorithm = Some(algorithm.into());
        self
    }

    /// Returns the public key.
    pub fn public_key(&self) -> &str {
        &self.public_key
    }

    /// Returns the private key secret.
    pub fn private_key(&self) -> &SecretString {
        &self.private_key
    }

    /// Returns the optional passphrase protecting the private key.
    pub fn passphrase(&self) -> Option<&SecretString> {
        self.passphrase.as_ref()
    }

    /// Returns the optional algorithm identifier.
    pub fn algorithm(&self) -> Option<&str> {
        self.algorithm.as_deref()
    }
}

impl std::fmt::Debug for KeyPair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyPair")
            .field(
                "public_key",
                &format_args!("[{} bytes]", self.public_key.len()),
            )
            .field("private_key", &"[REDACTED]")
            .field(
                "passphrase",
                &self.passphrase.as_ref().map(|_| "[REDACTED]"),
            )
            .field("algorithm", &self.algorithm)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AuthPattern;

    #[test]
    fn pattern_is_key_pair() {
        assert_eq!(KeyPair::pattern(), AuthPattern::KeyPair);
    }

    #[test]
    fn debug_redacts_secrets() {
        let kp = KeyPair::new("ssh-ed25519 AAAA...", SecretString::new("PRIVATE_KEY_DATA"))
            .with_passphrase(SecretString::new("my-passphrase"))
            .with_algorithm("ed25519");
        let debug = format!("{kp:?}");
        assert!(debug.contains("[19 bytes]"));
        assert!(debug.contains("ed25519"));
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("PRIVATE_KEY_DATA"));
        assert!(!debug.contains("my-passphrase"));
        assert!(!debug.contains("ssh-ed25519"));
    }
}
