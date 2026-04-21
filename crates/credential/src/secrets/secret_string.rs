//! Secret string type with automatic zeroization.

use std::fmt;

use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

/// Secret string with automatic memory zeroization.
///
/// Secrets are never exposed directly -- they must be accessed within
/// a closure scope using [`expose_secret`](SecretString::expose_secret).
/// Memory is automatically zeroed when the value is dropped.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct SecretString {
    inner: String,
}

impl SecretString {
    /// Creates a new secret from any string-like value.
    pub fn new<S: Into<String>>(s: S) -> Self {
        Self { inner: s.into() }
    }

    /// Accesses secret value within a closure scope.
    pub fn expose_secret<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&str) -> R,
    {
        f(&self.inner)
    }

    /// Returns the length without exposing content.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Checks if empty without exposing content.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

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

/// Sentinel written by the default `Serialize` impl.
pub(crate) const REDACTED_SENTINEL: &str = "[REDACTED]";

/// # Serde contract (non-roundtrippable by design)
///
/// `Serialize` always writes the `"[REDACTED]"` sentinel -- the real secret
/// value is **never** emitted. This ensures secrets cannot leak through
/// JSON logs, API responses, or debug dumps.
///
/// `Deserialize` accepts any string **except** `"[REDACTED]"`. Attempting
/// to deserialize the sentinel is rejected with an error. This prevents
/// accidentally round-tripping a redacted placeholder back into the system
/// as if it were a real secret.
///
/// To serialize/deserialize a `SecretString` that preserves the actual value
/// (e.g., for encrypted-at-rest storage), use the `serde_secret` helper
/// module instead of the default impls.
impl Serialize for SecretString {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(REDACTED_SENTINEL)
    }
}

impl<'de> Deserialize<'de> for SecretString {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        if s == REDACTED_SENTINEL {
            return Err(serde::de::Error::custom(
                "refusing to deserialize SecretString from the `[REDACTED]` sentinel",
            ));
        }
        Ok(SecretString::new(s))
    }
}
