//! Secret material types for `Field::Secret`.
//!
//! `SecretValue` is **not** encryption-at-rest, and **not** a key-derivation
//! layer — both belong to `nebula-credential` / storage, which own the
//! AES-256-GCM + Argon2id pipeline. This module provides only the in-memory
//! hygiene a schema layer needs: zeroizing buffers, redacted `Debug` /
//! `Display` / `Serialize` defaults, a constant-time `PartialEq`, an audited
//! [`SecretString::expose`] / [`SecretBytes::expose`], and an explicit
//! [`SecretWire`] for callers that must serialize plaintext to an
//! already-encrypted channel.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use zeroize::Zeroizing;

/// String returned by JSON helpers when a secret must not be leaked on the wire.
pub const SECRET_REDACTED: &str = "<redacted>";

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
    /// Emits a `tracing::trace!` line with a caller location for diagnostics.
    /// Enable the `audit-secret-expose` feature to escalate to `tracing::debug!`
    /// for compliance / audit trails.
    #[inline]
    #[track_caller]
    pub fn expose(&self) -> &str {
        let s = self.0.as_str();
        #[cfg(feature = "audit-secret-expose")]
        tracing::debug!(
            target: "nebula_schema::secret",
            location = %std::panic::Location::caller(),
            "SecretString::expose"
        );
        #[cfg(not(feature = "audit-secret-expose"))]
        tracing::trace!(
            target: "nebula_schema::secret",
            location = %std::panic::Location::caller(),
            "SecretString::expose"
        );
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

// `SecretString` deliberately rejects deserialization. Secret material must
// **never** be reconstructed from wire bytes that the schema layer sees:
//
// - Schema definitions (`Field::Secret`) flow over `serde` for catalog / plugin manifests; allowing
//   `SecretString` here would let a default value or a leaked test fixture round-trip plaintext
//   through schema storage.
// - Resolved secret values are always introduced by the resolve pipeline (via `SecretValue::string`),
//   not by parsing wire JSON.
//
// As a result, `Schema` definitions must NOT contain a `default` for a
// `Field::Secret`; the lint pass in `crate::lint` enforces this with the
// `secret.default_forbidden` code (Severity::Error). To populate a secret
// field, configure it via the credential setup form.
impl<'de> Deserialize<'de> for SecretString {
    fn deserialize<D: Deserializer<'de>>(_deserializer: D) -> Result<Self, D::Error> {
        Err(serde::de::Error::custom(
            "SecretString cannot be constructed from a deserializer — \
             secret material must originate from the resolve pipeline, \
             not from wire JSON",
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

/// Opaque byte secret (binary tokens, externally-hashed outputs).
#[derive(Clone)]
pub struct SecretBytes(Zeroizing<Vec<u8>>);

impl SecretBytes {
    /// Return raw bytes.
    ///
    /// Emits a `tracing::trace!` line with a caller location for diagnostics.
    /// Enable the `audit-secret-expose` feature to escalate to `tracing::debug!`
    /// for compliance / audit trails.
    #[inline]
    #[track_caller]
    pub fn expose(&self) -> &[u8] {
        #[cfg(feature = "audit-secret-expose")]
        tracing::debug!(
            target: "nebula_schema::secret",
            location = %std::panic::Location::caller(),
            "SecretBytes::expose"
        );
        #[cfg(not(feature = "audit-secret-expose"))]
        tracing::trace!(
            target: "nebula_schema::secret",
            location = %std::panic::Location::caller(),
            "SecretBytes::expose"
        );
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

// `Zeroizing<Vec<u8>>` already zeroizes the heap buffer via its blanket
// `Drop` impl — no manual `Drop` needed here. (Symmetric with
// `SecretString` which relies on `Zeroizing<String>` for the same
// guarantee.)

#[cfg(test)]
mod tests {
    use super::*;

    // ── SecretString ────────────────────────────────────────────────────────

    #[test]
    fn string_expose_returns_plaintext() {
        let s = SecretString::new("hunter2".to_owned());
        assert_eq!(s.expose(), "hunter2");
        assert!(!s.is_empty());
        assert!(SecretString::new(String::new()).is_empty());
    }

    #[test]
    fn string_debug_does_not_echo_plaintext() {
        let s = SecretString::new("hunter2".to_owned());
        assert_eq!(format!("{s:?}"), "SecretString(<redacted>)");
        assert!(!format!("{s:?}").contains("hunter2"));
    }

    #[test]
    fn string_display_redacts() {
        let s = SecretString::new("hunter2".to_owned());
        assert_eq!(format!("{s}"), SECRET_REDACTED);
        assert!(!format!("{s}").contains("hunter2"));
    }

    #[test]
    fn string_serialize_redacts() {
        let s = SecretString::new("hunter2".to_owned());
        assert_eq!(serde_json::to_value(&s).expect("json"), redacted());
    }

    #[test]
    fn string_eq_is_value_consistent() {
        let a = SecretString::new("same".to_owned());
        let b = SecretString::new("same".to_owned());
        let c = SecretString::new("different".to_owned());
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn string_deserialize_is_rejected() {
        assert!(serde_json::from_str::<SecretString>("\"x\"").is_err());
    }

    // ── SecretBytes ───────────────────────────────────────────────────────────

    #[test]
    fn bytes_expose_returns_raw() {
        let v = SecretValue::bytes(vec![1u8, 2, 3]);
        let SecretValue::Bytes(b) = &v else {
            panic!("expected bytes");
        };
        assert_eq!(b.expose(), &[1, 2, 3]);
        assert!(!b.is_empty());
    }

    #[test]
    fn bytes_debug_does_not_echo_plaintext() {
        let v = SecretValue::bytes(vec![0xde, 0xad]);
        let SecretValue::Bytes(b) = &v else {
            panic!("expected bytes");
        };
        assert_eq!(format!("{b:?}"), "SecretBytes(<redacted>)");
    }

    #[test]
    fn bytes_serialize_redacts_not_hex() {
        // Bare `SecretBytes` serialization must redact — only `SecretWire`
        // emits the hex plaintext form.
        let v = SecretValue::bytes(vec![0x61, 0x62]);
        let SecretValue::Bytes(b) = &v else {
            panic!("expected bytes");
        };
        assert_eq!(serde_json::to_value(b).expect("json"), redacted());
    }

    #[test]
    fn bytes_eq_is_value_consistent() {
        let a = SecretValue::bytes(vec![1u8, 2, 3]);
        let b = SecretValue::bytes(vec![1u8, 2, 3]);
        let c = SecretValue::bytes(vec![9u8]);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn bytes_deserialize_is_rejected() {
        assert!(serde_json::from_str::<SecretBytes>("\"x\"").is_err());
    }

    // ── SecretValue ───────────────────────────────────────────────────────────

    #[test]
    fn value_is_empty_tracks_inner() {
        assert!(SecretValue::string(String::new()).is_empty());
        assert!(!SecretValue::string("x".to_owned()).is_empty());
        assert!(SecretValue::bytes(Vec::new()).is_empty());
        assert!(!SecretValue::bytes(vec![0u8]).is_empty());
    }

    #[test]
    fn value_to_json_and_serialize_redact_both_variants() {
        assert_eq!(SecretValue::string("s".to_owned()).to_json(), redacted());
        assert_eq!(SecretValue::bytes(vec![1u8]).to_json(), redacted());
        assert_eq!(
            serde_json::to_value(SecretValue::string("s".to_owned())).expect("json"),
            redacted()
        );
    }

    // ── SecretWire ────────────────────────────────────────────────────────────

    #[test]
    fn wire_serializes_string_plaintext() {
        let v = SecretValue::string("plaintext".to_owned());
        assert_eq!(
            serde_json::to_value(SecretWire(&v)).expect("json"),
            serde_json::Value::String("plaintext".into())
        );
    }

    #[test]
    fn wire_serializes_bytes_as_hex() {
        let v = SecretValue::bytes(vec![0x61, 0x62, 0x63]);
        assert_eq!(
            serde_json::to_value(SecretWire(&v)).expect("json"),
            serde_json::Value::String("616263".into())
        );
    }

    #[test]
    fn wire_debug_does_not_echo_plaintext() {
        let v = SecretValue::string("topsecret".to_owned());
        assert_eq!(format!("{:?}", SecretWire(&v)), "SecretWire(<plaintext>)");
        assert!(!format!("{:?}", SecretWire(&v)).contains("topsecret"));
    }

    fn redacted() -> serde_json::Value {
        serde_json::Value::String(SECRET_REDACTED.to_owned())
    }
}
