//! Pre-shared symmetric key authentication (TLS-PSK, WireGuard, IoT).

use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::{AuthScheme, SecretString};

/// A pre-shared symmetric key, optionally paired with an identity hint.
///
/// Covers TLS-PSK, WireGuard pre-shared keys, IoT device symmetric keys,
/// and other protocols where both parties share a secret key out-of-band.
///
/// Per Tech Spec §15.5 — `SensitiveScheme`: shared key is the secret;
/// identity hint is non-secret metadata sent during negotiation.
///
/// # Examples
///
/// ```
/// use nebula_credential::{SecretString, scheme::SharedKey};
///
/// let key = SharedKey::new(SecretString::new("base64-encoded-key==")).with_identity("device-001");
/// ```
#[derive(Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop, AuthScheme)]
#[auth_scheme(pattern = SharedSecret, sensitive)]
pub struct SharedKey {
    #[serde(with = "crate::serde_secret")]
    key: SecretString,
    #[zeroize(skip)]
    identity: Option<String>,
}

impl SharedKey {
    /// Creates a new pre-shared key credential.
    #[must_use]
    pub fn new(key: SecretString) -> Self {
        Self {
            key,
            identity: None,
        }
    }

    /// Sets the identity hint sent alongside the key during negotiation.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_identity(mut self, identity: impl Into<String>) -> Self {
        self.identity = Some(identity.into());
        self
    }

    /// Returns the shared key secret.
    pub fn key(&self) -> &SecretString {
        &self.key
    }

    /// Returns the optional identity hint.
    pub fn identity(&self) -> Option<&str> {
        self.identity.as_deref()
    }
}

impl std::fmt::Debug for SharedKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedKey")
            .field("key", &"[REDACTED]")
            .field("identity", &self.identity)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AuthPattern;

    #[test]
    fn pattern_is_shared_secret() {
        assert_eq!(SharedKey::pattern(), AuthPattern::SharedSecret);
    }

    #[test]
    fn debug_redacts_key() {
        let key =
            SharedKey::new(SecretString::new("base64-encoded-key==")).with_identity("device-001");
        let debug = format!("{key:?}");
        assert!(debug.contains("device-001"));
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("base64-encoded-key=="));
    }
}
