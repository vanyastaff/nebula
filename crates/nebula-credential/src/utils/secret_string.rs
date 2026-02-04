//! Secret string type with automatic zeroization
//!
//! Provides [`SecretString`] with controlled access via closure API
//! to prevent accidental secret copying and automatic memory zeroization.

use serde::{Deserialize, Serialize};
use std::fmt;
use zeroize::{Zeroize, ZeroizeOnDrop};

/// Secret string with automatic memory zeroization
///
/// Secrets are never exposed directly - they must be accessed within
/// a closure scope using [`expose_secret`] to prevent accidental copying.
/// Memory is automatically zeroed when the value is dropped.
///
/// [`expose_secret`]: SecretString::expose_secret
///
/// # Examples
///
/// ```
/// use nebula_credential::SecretString;
///
/// let secret = SecretString::new("my-api-key");
///
/// // Access secret within closure
/// secret.expose_secret(|value| {
///     println!("Secret length: {}", value.len());
///     // Use value here - cannot escape closure scope
/// });
///
/// // Secret is redacted in debug/display output
/// println!("{:?}", secret); // Prints: [REDACTED]
/// ```
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct SecretString {
    inner: String,
}

impl SecretString {
    /// Creates a new secret from any string-like value
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_credential::SecretString;
    ///
    /// let secret = SecretString::new("my-password");
    /// let from_string = SecretString::new(String::from("another-password"));
    /// ```
    pub fn new<S: Into<String>>(s: S) -> Self {
        Self { inner: s.into() }
    }

    /// Accesses secret value within a closure scope
    ///
    /// This prevents accidental copying or leaking of the secret.
    /// The secret value cannot escape the closure.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_credential::SecretString;
    ///
    /// let secret = SecretString::new("password");
    /// let len = secret.expose_secret(|s| s.len());
    /// assert_eq!(len, 8);
    ///
    /// // Cannot return &str - won't compile
    /// // let leaked = secret.expose_secret(|s| s); // ERROR: doesn't outlive closure
    /// ```
    pub fn expose_secret<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&str) -> R,
    {
        f(&self.inner)
    }

    /// Returns the length without exposing content
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_credential::SecretString;
    ///
    /// let secret = SecretString::new("12345");
    /// assert_eq!(secret.len(), 5);
    /// ```
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Checks if empty without exposing content
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_credential::SecretString;
    ///
    /// let empty = SecretString::new("");
    /// assert!(empty.is_empty());
    ///
    /// let not_empty = SecretString::new("x");
    /// assert!(!not_empty.is_empty());
    /// ```
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

// Prevent accidental secret leakage via Debug/Display
impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

impl fmt::Display for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

// Serialize as redacted for safety
// Note: This prevents accidental secret leakage in logs
impl Serialize for SecretString {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str("[REDACTED]")
    }
}

// Deserialize from string
impl<'de> Deserialize<'de> for SecretString {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        String::deserialize(deserializer).map(SecretString::new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_secret_string_new() {
        let secret = SecretString::new("test_value");
        secret.expose_secret(|s| assert_eq!(s, "test_value"));
    }

    #[test]
    fn test_secret_string_expose_secret() {
        let secret = SecretString::new("my_secret");
        let len = secret.expose_secret(|s| s.len());
        assert_eq!(len, 9);

        let upper = secret.expose_secret(|s| s.to_uppercase());
        assert_eq!(upper, "MY_SECRET");
    }

    #[test]
    fn test_secret_string_len() {
        let secret = SecretString::new("12345");
        assert_eq!(secret.len(), 5);
    }

    #[test]
    fn test_secret_string_is_empty() {
        let empty = SecretString::new("");
        assert!(empty.is_empty());

        let not_empty = SecretString::new("x");
        assert!(!not_empty.is_empty());
    }

    #[test]
    fn test_secret_string_debug() {
        let secret = SecretString::new("super_secret_password");
        let debug_str = format!("{:?}", secret);
        assert_eq!(debug_str, "[REDACTED]");
        assert!(!debug_str.contains("super_secret"));
    }

    #[test]
    fn test_secret_string_display() {
        let secret = SecretString::new("api_key_12345");
        let display_str = format!("{}", secret);
        assert_eq!(display_str, "[REDACTED]");
        assert!(!display_str.contains("api_key"));
    }

    #[test]
    fn test_secret_string_serialize_redacted() {
        let secret = SecretString::new("should_be_redacted");
        let json = serde_json::to_string(&secret).unwrap();
        assert_eq!(json, "\"[REDACTED]\"");
        assert!(!json.contains("should_be_redacted"));
    }

    #[test]
    fn test_secret_string_deserialize() {
        let json = "\"deserialized_secret\"";
        let secret: SecretString = serde_json::from_str(json).unwrap();
        secret.expose_secret(|s| assert_eq!(s, "deserialized_secret"));
    }

    #[test]
    fn test_secret_string_clone() {
        let original = SecretString::new("clone_test");
        let cloned = original.clone();

        original.expose_secret(|s1| {
            cloned.expose_secret(|s2| {
                assert_eq!(s1, s2);
            });
        });
    }

    #[test]
    fn test_secret_cannot_escape_closure() {
        let secret = SecretString::new("trapped");

        // This demonstrates the secret stays within closure scope
        let result = secret.expose_secret(|s| {
            // We can use the secret here
            s.len()
            // But we cannot return &str - it doesn't outlive the closure
        });

        assert_eq!(result, 7);
    }
}
