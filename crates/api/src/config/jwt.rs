use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::errors::ApiConfigError;

/// Validated HS256 signing key.
///
/// Construction via [`JwtSecret::new`] is the ONLY place length and
/// known-bad-value checks live. Any `JwtSecret` in hand is valid.
///
/// `Debug` redacts the secret contents so accidental `{:?}` prints
/// never leak key material into logs.
#[derive(Clone)]
pub struct JwtSecret(Arc<str>);

impl JwtSecret {
    /// Minimum length for HS256. RFC 7518 §3.2 requires "a key of
    /// the same size as the hash output"; for HS256 that is 32 bytes.
    pub const MIN_BYTES: usize = 32;

    /// The well-known development placeholder. Explicitly rejected
    /// even if someone leaks it back in via an env var.
    pub const DEV_PLACEHOLDER: &'static str = "dev-secret-change-in-production";

    /// Validate and wrap a raw secret string.
    ///
    /// # Errors
    ///
    /// - [`ApiConfigError::JwtSecretTooShort`] if the input is shorter than [`Self::MIN_BYTES`]
    ///   bytes.
    /// - [`ApiConfigError::JwtSecretIsDevPlaceholder`] if the input matches the well-known
    ///   development placeholder.
    pub fn new(raw: impl Into<Arc<str>>) -> Result<Self, ApiConfigError> {
        let raw = raw.into();
        if raw.as_ref() == Self::DEV_PLACEHOLDER {
            return Err(ApiConfigError::JwtSecretIsDevPlaceholder);
        }
        if raw.len() < Self::MIN_BYTES {
            return Err(ApiConfigError::JwtSecretTooShort {
                got: raw.len(),
                min: Self::MIN_BYTES,
            });
        }
        Ok(Self(raw))
    }

    /// Return the raw secret bytes for signature verification.
    #[inline]
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    /// Generate a random 32-byte secret, hex-encoded (64 chars).
    ///
    /// Intended for dev-mode ephemeral startup. **Never** call this
    /// in production code paths — it bypasses the guarantee that the
    /// operator has explicitly configured an auth key.
    pub(super) fn generate_ephemeral() -> Self {
        use rand::RngExt;

        let mut rng = rand::rng();
        let bytes: [u8; 32] = rng.random();
        let mut hex = String::with_capacity(64);
        for b in bytes {
            // Two hex chars per byte — never fails.
            hex.push(char::from_digit(u32::from(b >> 4), 16).unwrap_or('0'));
            hex.push(char::from_digit(u32::from(b & 0x0f), 16).unwrap_or('0'));
        }
        Self(Arc::from(hex))
    }

    /// Unchecked constructor for the `test-util` feature.
    ///
    /// Only reachable behind `#[cfg(any(test, feature = "test-util"))]`,
    /// so production builds cannot accidentally bypass validation.
    #[cfg(any(test, feature = "test-util"))]
    pub(super) fn for_test_unchecked(raw: &'static str) -> Self {
        Self(Arc::from(raw))
    }
}

impl std::fmt::Debug for JwtSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("JwtSecret([REDACTED])")
    }
}

// Serde: serialize as redacted so accidental config dumps (e.g. via
// `serde_json::to_string(&config)`) never leak the secret. Deserialize
// goes through the validating `new` constructor.
impl Serialize for JwtSecret {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str("[REDACTED]")
    }
}

impl<'de> Deserialize<'de> for JwtSecret {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(de)?;
        JwtSecret::new(raw).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jwt_secret_debug_is_redacted() {
        let secret = JwtSecret::new("this-is-a-32-byte-minimum-secret!!".to_string()).unwrap();
        let formatted = format!("{secret:?}");
        assert!(formatted.contains("REDACTED"));
        assert!(!formatted.contains("this-is-a-32-byte"));
    }

    #[test]
    fn jwt_secret_new_rejects_placeholder() {
        let err = JwtSecret::new(JwtSecret::DEV_PLACEHOLDER.to_string())
            .expect_err("placeholder must be rejected");
        assert!(matches!(err, ApiConfigError::JwtSecretIsDevPlaceholder));
    }
}
