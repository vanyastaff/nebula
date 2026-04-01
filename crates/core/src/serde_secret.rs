//! Serde helpers for [`SecretString`] that preserve the actual value.
//!
//! The default `SecretString` `Serialize` impl writes `[REDACTED]` (safe for
//! logs) — this module writes the real value so encrypted-at-rest state
//! round-trips correctly.
//!
//! Use with `#[serde(with = "nebula_core::serde_secret")]` on `SecretString` fields.

use crate::SecretString;
use serde::{Deserialize, Deserializer, Serializer};

/// Serialize the actual secret value (for encrypted-at-rest storage only).
pub fn serialize<S: Serializer>(secret: &SecretString, s: S) -> Result<S::Ok, S::Error> {
    secret.expose_secret(|v| s.serialize_str(v))
}

/// Deserialize a string into a `SecretString`.
pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<SecretString, D::Error> {
    String::deserialize(d).map(SecretString::new)
}
