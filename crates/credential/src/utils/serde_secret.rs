//! Serde helpers for transparent [`SecretString`] serialization.
//!
//! Used for fields that must survive JSON round-trips (e.g., stored
//! encrypted at rest via `EncryptionLayer`). The default `SecretString`
//! serializer writes `"[REDACTED]"` which destroys the value.

use crate::SecretString;
use serde::{Deserialize, Deserializer, Serializer};

/// Serialize a [`SecretString`] by exposing its actual value.
///
/// # Warning
///
/// Only use this for fields that are encrypted at rest. The secret
/// will appear in plaintext in the serialized output.
pub fn serialize<S>(value: &SecretString, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    value.expose_secret(|s| serializer.serialize_str(s))
}

/// Deserialize a string into a [`SecretString`].
pub fn deserialize<'de, D>(deserializer: D) -> Result<SecretString, D::Error>
where
    D: Deserializer<'de>,
{
    String::deserialize(deserializer).map(SecretString::new)
}
