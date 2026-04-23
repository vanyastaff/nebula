//! Secret string type backed by the `secrecy` crate with automatic zeroization.

use std::fmt;

use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

/// Secret string with automatic memory zeroization.
///
/// Thin wrapper around [`secrecy::SecretString`] that adds sentinel-aware
/// serde (the default `Serialize` writes `"[REDACTED]"` and `Deserialize`
/// rejects the sentinel) plus convenience helpers (`len`, `is_empty`).
///
/// Access the plaintext via [`expose_secret()`](SecretString::expose_secret),
/// which returns `&str` directly.
/// Memory is automatically zeroed when the value is dropped.
#[derive(Clone)]
pub struct SecretString {
    inner: secrecy::SecretString,
}

impl SecretString {
    /// Creates a new secret from any string-like value.
    pub fn new<S: Into<String>>(s: S) -> Self {
        Self {
            inner: secrecy::SecretString::from(s.into()),
        }
    }

    /// Exposes the secret value. The caller is responsible for not leaking it.
    pub fn expose_secret(&self) -> &str {
        self.inner.expose_secret()
    }

    /// Returns the length without exposing content beyond this call.
    pub fn len(&self) -> usize {
        self.inner.expose_secret().len()
    }

    /// Checks if empty without exposing content beyond this call.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.expose_secret().is_empty()
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

impl Zeroize for SecretString {
    fn zeroize(&mut self) {
        self.inner.zeroize();
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
