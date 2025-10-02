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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_secure_string_creation() {
        let secret = SecureString::new("my-secret");
        assert_eq!(secret.expose(), "my-secret");
    }

    #[test]
    fn test_secure_string_expose() {
        let secret = SecureString::new("password123");
        assert_eq!(secret.expose(), "password123");
    }

    #[test]
    fn test_secure_string_with_exposed() {
        let secret = SecureString::new("test-value");
        let result = secret.with_exposed(|s| s.len());
        assert_eq!(result, 10);
    }

    #[test]
    fn test_secure_string_constant_time_eq() {
        let s1 = SecureString::new("same-value");
        let s2 = SecureString::new("same-value");
        let s3 = SecureString::new("different");

        assert!(s1.eq_ct(&s2));
        assert!(!s1.eq_ct(&s3));
    }

    #[test]
    fn test_secure_string_debug_does_not_leak() {
        let secret = SecureString::new("super-secret-password");
        let debug_str = format!("{:?}", secret);

        assert!(!debug_str.contains("super-secret"));
        assert!(!debug_str.contains("password"));
        assert!(debug_str.contains("REDACTED"));
    }

    #[test]
    fn test_secure_string_serialization() {
        let original = SecureString::new("test-secret");
        let json = serde_json::to_string(&original).expect("serialization should work");

        // Should be base64 encoded
        assert!(!json.contains("test-secret"));

        let deserialized: SecureString =
            serde_json::from_str(&json).expect("deserialization should work");
        assert_eq!(deserialized.expose(), "test-secret");
    }

    #[test]
    fn test_secure_string_clone() {
        let original = SecureString::new("cloneable");
        let cloned = original.clone();
        assert_eq!(original.expose(), cloned.expose());
    }

    #[test]
    fn test_secure_string_empty() {
        let empty = SecureString::new("");
        assert_eq!(empty.expose(), "");
    }

    #[test]
    fn test_secure_string_unicode() {
        let unicode = SecureString::new("こんにちは🎌");
        assert_eq!(unicode.expose(), "こんにちは🎌");
    }
}
