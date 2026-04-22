//! Secret material and optional KDF parameters for `Field::Secret`.
//!
//! `SecretValue` is **not** encryption-at-rest (that is `nebula-credential` /
//! storage). It **does** provide zeroizing buffers, redacted `Debug` /
//! `Display` / `Serialize` defaults, an audited [`SecretString::expose`] /
//! [`SecretBytes::expose`], and an explicit [`SecretWire`] for callers that
//! must persist plaintext to an already-encrypted channel.
//!
//! Architecture: [`ADR-0034`](../../../docs/adr/0034-schema-secret-value-credential-seam.md)
//! in-repo; design details in
//! `docs/superpowers/specs/2026-04-16-nebula-schema-phase3-security-design.md`.

use std::{fmt, ops::DerefMut};

use argon2::{Argon2, Params};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;
use zeroize::{Zeroize, Zeroizing};

/// String returned by JSON helpers when a secret must not be leaked on the wire.
pub const SECRET_REDACTED: &str = "<redacted>";

// ── KDF error + salt ---------------------------------------------------------

/// Failure while hashing a secret at resolve time.
#[derive(Debug, Error)]
pub enum KdfError {
    /// User-facing misconfiguration in `KdfParams` (including salt length/parity
    /// rules that are not a [`hex::FromHexError`].
    #[error("invalid KDF parameters: {0}")]
    InvalidParams(String),
    /// Invalid hex in `salt_hex` (chained from [`hex::FromHexError`]).
    #[error("invalid KDF parameters: {0}")]
    InvalidSaltHex(#[from] hex::FromHexError),
    /// Argon2 `Params` construction failed (e.g. cost tuple out of range for the
    /// underlying CSPRNG parameters).
    #[error("invalid KDF parameters: {0}")]
    Argon2Params(#[source] argon2::Error),
    /// Argon2 hashing failed (password can be `hash_password_into`).
    #[error("KDF operation failed: {0}")]
    KdfHash(#[source] argon2::Error),
}

fn decode_salt(salt_hex: &str) -> Result<Vec<u8>, KdfError> {
    let s = salt_hex.trim().trim_start_matches("0x");
    if !s.len().is_multiple_of(2) {
        return Err(KdfError::InvalidParams(
            "KDF salt_hex must be even-length".into(),
        ));
    }
    let bytes = hex::decode(s).map_err(KdfError::InvalidSaltHex)?;
    if !(8..=64).contains(&bytes.len()) {
        return Err(KdfError::InvalidParams(
            "KDF salt must decode to 8–64 bytes after hex decode".into(),
        ));
    }
    Ok(bytes)
}

// ── KDF parameters (schema wire / builder) ---------------------------------

/// Key-derivation parameters for an optional post-input hashing step.
///
/// When set on a [`super::field::SecretField`], the resolve pipeline replaces a
/// user `string` secret with [`SecretValue::Bytes`] (never the KDF *password*
/// in resolved storage).
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "algorithm", rename_all = "snake_case", deny_unknown_fields)]
pub enum KdfParams {
    /// Argon2id (RFC 9106) using the `argon2` crate.
    Argon2id {
        /// Salt as even-length hex (no `0x`); decoded length 8–32 bytes recommended.
        salt_hex: String,
        /// `m` cost (memory, `KiB`). Must match the Argon2 `Params` range.
        memory_kib: u32,
        /// `t` cost (iterations).
        time_cost: u32,
        /// `p` cost (parallelism / lanes). Must be at least 1.
        #[serde(default = "default_p_cost")]
        parallelism: u8,
    },
}

const fn default_p_cost() -> u8 {
    1
}

// Product-level guardrails for resolve-time KDF cost to prevent runaway
// resource usage from unbounded user-supplied schema parameters.
const MAX_KDF_MEMORY_KIB: u32 = 256 * 1024;
const MAX_KDF_TIME_COST: u32 = 10;
const MAX_KDF_PARALLELISM: u8 = 16;

// ── SecretString / SecretBytes / SecretValue --------------------------------

/// UTF-8 secret (common for passwords and API keys).
///
/// Stored as [`Zeroizing<String>`] so [`SecretString::expose`] is infallible
/// without `unsafe` (this crate uses `#![forbid(unsafe_code)]`).
#[derive(Clone)]
pub struct SecretString(Zeroizing<String>);

impl SecretString {
    /// Build from a `String` (moves; original allocation is not preserved as `String`).
    #[must_use]
    pub fn new(value: String) -> Self {
        Self(Zeroizing::new(value))
    }

    /// Redacted by default: [`SECRET_REDACTED`].
    #[inline]
    #[must_use]
    pub fn redacted_json() -> serde_json::Value {
        serde_json::Value::String(SECRET_REDACTED.to_owned())
    }

    /// Return the raw UTF-8 secret for trusted consumers.
    ///
    /// Emits a [`tracing::debug!`] line with a caller location to support audits.
    #[inline]
    #[track_caller]
    pub fn expose(&self) -> &str {
        let s = self.0.as_str();
        tracing::debug!(target: "nebula_schema::secret", location = %std::panic::Location::caller(), "SecretString::expose");
        s
    }

    /// Returns `true` when the underlying buffer is empty.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretString(")?;
        f.write_str(SECRET_REDACTED)?;
        f.write_str(")")
    }
}

impl fmt::Display for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(SECRET_REDACTED)
    }
}

impl Serialize for SecretString {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(SECRET_REDACTED)
    }
}

impl<'de> Deserialize<'de> for SecretString {
    fn deserialize<D: Deserializer<'de>>(_deserializer: D) -> Result<Self, D::Error> {
        Err(serde::de::Error::custom(
            "SecretString cannot be constructed from deserializer",
        ))
    }
}

impl PartialEq for SecretString {
    fn eq(&self, other: &Self) -> bool {
        use subtle::ConstantTimeEq;
        self.0.as_bytes().ct_eq(other.0.as_bytes()).into()
    }
}

impl Eq for SecretString {}

/// Opaque byte secret (hashed outputs, binary tokens).
#[derive(Clone)]
pub struct SecretBytes(Zeroizing<Vec<u8>>);

impl SecretBytes {
    pub(crate) fn from_vec_unchecked(v: Vec<u8>) -> Self {
        Self(Zeroizing::new(v))
    }

    /// Return raw bytes.
    #[inline]
    #[track_caller]
    pub fn expose(&self) -> &[u8] {
        tracing::debug!(target: "nebula_schema::secret", location = %std::panic::Location::caller(), "SecretBytes::expose");
        &self.0
    }

    /// Returns `true` when the buffer is empty.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for SecretBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretBytes(")?;
        f.write_str(SECRET_REDACTED)?;
        f.write_str(")")
    }
}

impl fmt::Display for SecretBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(SECRET_REDACTED)
    }
}

impl Serialize for SecretBytes {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(SECRET_REDACTED)
    }
}

impl<'de> Deserialize<'de> for SecretBytes {
    fn deserialize<D: Deserializer<'de>>(_deserializer: D) -> Result<Self, D::Error> {
        Err(serde::de::Error::custom(
            "SecretBytes cannot be constructed from deserializer",
        ))
    }
}

impl PartialEq for SecretBytes {
    fn eq(&self, other: &Self) -> bool {
        use subtle::ConstantTimeEq;
        self.0.as_slice().ct_eq(other.0.as_slice()).into()
    }
}

impl Eq for SecretBytes {}

/// Runtime secret value attached to a schema `Secret` field.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretValue {
    /// Secret UTF-8 string.
    String(SecretString),
    /// Opaque byte secret.
    Bytes(SecretBytes),
}

impl SecretValue {
    /// Wrap a `String` secret.
    #[must_use]
    pub fn string(value: String) -> Self {
        Self::String(SecretString::new(value))
    }

    /// Wrap raw bytes.
    pub fn bytes(value: impl Into<Zeroizing<Vec<u8>>>) -> Self {
        Self::Bytes(SecretBytes(value.into()))
    }

    /// `true` when the secret is an empty string or empty buffer.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        match self {
            Self::String(s) => s.is_empty(),
            Self::Bytes(b) => b.is_empty(),
        }
    }

    /// JSON form used by wire helpers; always the redacted string token.
    #[must_use]
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            Self::String(_) | Self::Bytes(_) => SecretString::redacted_json(),
        }
    }
}

impl Serialize for SecretValue {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.to_json().serialize(serializer)
    }
}

/// Explicit wrapper: serializes a [`SecretValue`] to plaintext (audited; use
/// for encrypted at-rest paths only).
pub struct SecretWire<'a>(pub &'a SecretValue);

impl fmt::Debug for SecretWire<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretWire(<plaintext>)")
    }
}

impl Serialize for SecretWire<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.0 {
            SecretValue::String(s) => s.expose().serialize(serializer),
            SecretValue::Bytes(b) => serializer.serialize_str(&hex::encode(b.expose())),
        }
    }
}

impl KdfParams {
    /// Run the configured KDF over `password` and return hashed secret bytes.
    ///
    /// # Errors
    ///
    /// Returns [`KdfError`] when parameters are invalid, salt decoding fails, or
    /// hashing fails.
    pub fn hash_password(&self, password: &[u8]) -> Result<SecretValue, KdfError> {
        match self {
            Self::Argon2id {
                salt_hex,
                memory_kib,
                time_cost,
                parallelism,
            } => {
                if *parallelism == 0 {
                    return Err(KdfError::InvalidParams(
                        "Argon2 parallelism must be >= 1".into(),
                    ));
                }
                if *memory_kib > MAX_KDF_MEMORY_KIB {
                    return Err(KdfError::InvalidParams(format!(
                        "Argon2 memory_kib must be <= {MAX_KDF_MEMORY_KIB}"
                    )));
                }
                if *time_cost > MAX_KDF_TIME_COST {
                    return Err(KdfError::InvalidParams(format!(
                        "Argon2 time_cost must be <= {MAX_KDF_TIME_COST}"
                    )));
                }
                if *parallelism > MAX_KDF_PARALLELISM {
                    return Err(KdfError::InvalidParams(format!(
                        "Argon2 parallelism must be <= {MAX_KDF_PARALLELISM}"
                    )));
                }
                let salt = decode_salt(salt_hex)?;
                let out_len: usize = 32;
                let params = Params::new(
                    *memory_kib,
                    *time_cost,
                    u32::from(*parallelism),
                    Some(out_len),
                )
                .map_err(KdfError::Argon2Params)?;
                let argon2 =
                    Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
                let mut out = vec![0u8; out_len];
                argon2
                    .hash_password_into(password, &salt, &mut out)
                    .map_err(KdfError::KdfHash)?;
                Ok(SecretValue::Bytes(SecretBytes::from_vec_unchecked(out)))
            },
        }
    }
}

impl Drop for SecretBytes {
    fn drop(&mut self) {
        self.0.deref_mut().as_mut_slice().zeroize();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_debug_does_not_echo_plaintext() {
        let s = SecretString::new("hunter2".to_owned());
        let dbg = format!("{s:?}");
        assert!(!dbg.contains("hunter2"));
    }

    #[test]
    fn string_serialize_redacts() {
        let s = SecretString::new("hunter2".to_owned());
        let j = serde_json::to_value(&s).expect("json");
        assert_eq!(j, json_redacted());
    }

    fn json_redacted() -> serde_json::Value {
        serde_json::Value::String(SECRET_REDACTED.to_owned())
    }

    #[test]
    fn wire_serializes_bytes_as_hex() {
        let b = SecretBytes::from_vec_unchecked(vec![0x61, 0x62, 0x63]);
        let v = SecretValue::Bytes(b);
        let ser = serde_json::to_value(SecretWire(&v)).expect("json");
        assert_eq!(ser, serde_json::Value::String("616263".into()));
    }

    #[test]
    fn kdf_invalid_salt_hex_chains_from_hex_error() {
        let kdf = KdfParams::Argon2id {
            // Even length but non-hex characters → `hex::FromHexError`.
            salt_hex: "gggggggggggggggg".to_owned(),
            memory_kib: 4096,
            time_cost: 1,
            parallelism: 1,
        };
        let err = kdf.hash_password(b"pw").expect_err("invalid hex");
        let KdfError::InvalidSaltHex(_) = &err else {
            panic!("expected InvalidSaltHex, got {err:?}");
        };
        assert!(std::error::Error::source(&err).is_some());
        let msg = err.to_string();
        assert!(msg.contains("invalid KDF parameters"), "msg was: {msg}");
    }

    #[test]
    fn kdf_rejects_excessive_resource_costs() {
        let kdf = KdfParams::Argon2id {
            salt_hex: "0011223344556677".to_owned(),
            memory_kib: MAX_KDF_MEMORY_KIB + 1,
            time_cost: 1,
            parallelism: 1,
        };
        let err = kdf
            .hash_password(b"pw")
            .expect_err("must reject high memory");
        assert!(matches!(err, KdfError::InvalidParams(_)));

        let kdf = KdfParams::Argon2id {
            salt_hex: "0011223344556677".to_owned(),
            memory_kib: 4096,
            time_cost: MAX_KDF_TIME_COST + 1,
            parallelism: 1,
        };
        let err = kdf
            .hash_password(b"pw")
            .expect_err("must reject high time_cost");
        assert!(matches!(err, KdfError::InvalidParams(_)));

        let kdf = KdfParams::Argon2id {
            salt_hex: "0011223344556677".to_owned(),
            memory_kib: 4096,
            time_cost: 1,
            parallelism: MAX_KDF_PARALLELISM + 1,
        };
        let err = kdf
            .hash_password(b"pw")
            .expect_err("must reject high parallelism");
        assert!(matches!(err, KdfError::InvalidParams(_)));
    }
}
