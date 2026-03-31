//! Newtype for credential type keys.
//!
//! Provides a thin wrapper around `&'static str` with a convenience
//! macro for compile-time construction.

use std::fmt;

/// Stable identifier for a credential type (e.g., `"github_oauth2"`).
///
/// Wraps `&'static str`. Use the [`credential_key!`] macro for construction.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct CredentialKey(&'static str);

impl CredentialKey {
    /// Creates a new credential key.
    ///
    /// Prefer [`credential_key!`] for compile-time construction.
    pub const fn new(key: &'static str) -> Self {
        Self(key)
    }

    /// Returns the key as a string slice.
    pub const fn as_str(&self) -> &'static str {
        self.0
    }
}

impl fmt::Display for CredentialKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl fmt::Debug for CredentialKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CredentialKey({:?})", self.0)
    }
}

impl AsRef<str> for CredentialKey {
    fn as_ref(&self) -> &str {
        self.0
    }
}

impl From<&'static str> for CredentialKey {
    fn from(key: &'static str) -> Self {
        Self(key)
    }
}

impl PartialEq<str> for CredentialKey {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for CredentialKey {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

/// Creates a [`CredentialKey`] at compile time.
///
/// # Examples
///
/// ```
/// use nebula_credential::credential_key;
///
/// const KEY: nebula_credential::CredentialKey = credential_key!("github_oauth2");
/// assert_eq!(KEY.as_str(), "github_oauth2");
/// ```
#[macro_export]
macro_rules! credential_key {
    ($key:expr) => {
        $crate::key::CredentialKey::new($key)
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_creation() {
        let key = CredentialKey::new("test");
        assert_eq!(key.as_str(), "test");
    }

    #[test]
    fn key_display() {
        let key = CredentialKey::new("oauth2");
        assert_eq!(format!("{key}"), "oauth2");
    }

    #[test]
    fn key_debug() {
        let key = CredentialKey::new("oauth2");
        assert_eq!(format!("{key:?}"), "CredentialKey(\"oauth2\")");
    }

    #[test]
    fn key_equality() {
        let a = CredentialKey::new("test");
        let b = CredentialKey::new("test");
        assert_eq!(a, b);
    }

    #[test]
    fn key_str_equality() {
        let key = CredentialKey::new("test");
        assert_eq!(key, "test");
        assert_eq!(key, *"test");
    }

    #[test]
    fn macro_creates_key() {
        const KEY: CredentialKey = crate::credential_key!("my_cred");
        assert_eq!(KEY.as_str(), "my_cred");
    }

    #[test]
    fn key_is_const() {
        const KEY: CredentialKey = CredentialKey::new("const_key");
        assert_eq!(KEY.as_str(), "const_key");
    }

    #[test]
    fn key_from_static_str() {
        let key: CredentialKey = "test".into();
        assert_eq!(key.as_str(), "test");
    }
}
