//! Serde helpers for [`SecretString`] that preserve the actual value.
//!
//! Use with `#[serde(with = "nebula_credential::serde_secret")]` on `SecretString` fields.

use serde::{Deserialize, Deserializer, Serializer};

use crate::SecretString;

/// Serialize the actual secret value (for encrypted-at-rest storage only).
pub fn serialize<S: Serializer>(secret: &SecretString, s: S) -> Result<S::Ok, S::Error> {
    secret.expose_secret(|v| s.serialize_str(v))
}

/// Deserialize a string into a `SecretString`.
pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<SecretString, D::Error> {
    String::deserialize(d).map(SecretString::new)
}
