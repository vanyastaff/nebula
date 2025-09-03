use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use subtle::ConstantTimeEq;

/// Secure string that zeros memory on drop
#[derive(Clone)]
pub struct SecureString(SecretString);

impl SecureString {
    /// Create new secure string
    pub fn new(s: impl Into<String>) -> Self {
        Self(SecretString::from(s.into()))
    }

    /// Expose the secret (use with caution)
    pub fn expose(&self) -> &str {
        self.0.expose_secret()
    }

    /// Execute function with exposed secret
    pub fn with_exposed<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&str) -> R,
    {
        f(self.0.expose_secret())
    }

    /// Constant-time equality check
    pub fn eq_ct(&self, other: &Self) -> bool {
        let a = self.0.expose_secret().as_bytes();
        let b = other.0.expose_secret().as_bytes();
        a.ct_eq(b).into()
    }
}

impl Serialize for SecureString {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let encoded = B64.encode(self.0.expose_secret().as_bytes());
        serializer.serialize_str(&encoded)
    }
}

impl<'de> Deserialize<'de> for SecureString {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let encoded = String::deserialize(deserializer)?;
        let decoded = B64
            .decode(encoded.as_bytes())
            .map_err(serde::de::Error::custom)?;
        let s = String::from_utf8(decoded).map_err(serde::de::Error::custom)?;
        Ok(SecureString::new(s))
    }
}

impl std::fmt::Debug for SecureString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SecureString[REDACTED]")
    }
}
