//! Secure string type with automatic zeroization

use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

/// Secure string that zeroizes on drop
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct SecureString(String);

impl SecureString {
    /// Create a new secure string
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Expose the inner value (use carefully!)
    #[must_use] 
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Convert to String (consumes self, use carefully!)
    #[must_use] 
    pub fn into_string(mut self) -> String {
        std::mem::take(&mut self.0)
    }
}

impl std::fmt::Debug for SecureString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[REDACTED]")
    }
}

impl std::fmt::Display for SecureString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[REDACTED]")
    }
}

impl Serialize for SecureString {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for SecureString {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        String::deserialize(deserializer).map(SecureString::new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_secure_string_debug() {
        let secret = SecureString::new("super_secret_password");
        let debug_str = format!("{:?}", secret);
        assert_eq!(debug_str, "[REDACTED]");
        assert!(!debug_str.contains("super_secret"));
    }

    #[test]
    fn test_secure_string_display() {
        let secret = SecureString::new("api_key_12345");
        let display_str = format!("{}", secret);
        assert_eq!(display_str, "[REDACTED]");
        assert!(!display_str.contains("api_key"));
    }

    #[test]
    fn test_secure_string_expose() {
        let secret = SecureString::new("my_secret");
        assert_eq!(secret.expose(), "my_secret");
    }

    #[test]
    fn test_secure_string_serialize() {
        let secret = SecureString::new("test_value");
        let json = serde_json::to_string(&secret).unwrap();
        assert_eq!(json, "\"test_value\"");
    }

    #[test]
    fn test_secure_string_deserialize() {
        let json = "\"deserialized_secret\"";
        let secret: SecureString = serde_json::from_str(json).unwrap();
        assert_eq!(secret.expose(), "deserialized_secret");
    }
}
