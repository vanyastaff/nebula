//! Serde helpers for [`SecretString`] that preserve the actual value.
//!
//! Use with `#[serde(with = "nebula_credential::serde_secret")]` for
//! `SecretString` fields or `#[serde(with = "nebula_credential::serde_secret::option")]`
//! for `Option<SecretString>` fields.

use serde::{Deserialize, Deserializer, Serializer};

use super::SecretString;

/// Serialize the actual secret value (for encrypted-at-rest storage only).
pub fn serialize<S: Serializer>(secret: &SecretString, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(secret.expose_secret())
}

/// Deserialize a string into a `SecretString`.
pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<SecretString, D::Error> {
    String::deserialize(d).map(SecretString::new)
}

/// Serde helpers for `Option<SecretString>`. Use as:
/// `#[serde(with = "nebula_credential::serde_secret::option")]`.
pub mod option {
    use super::*;

    /// Serialize an optional secret value (for encrypted-at-rest storage only).
    pub fn serialize<S: Serializer>(
        secret: &Option<SecretString>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        match secret {
            Some(secret) => s.serialize_str(secret.expose_secret()),
            None => s.serialize_none(),
        }
    }

    /// Deserialize an optional string into an `Option<SecretString>`.
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<SecretString>, D::Error> {
        Option::<String>::deserialize(d).map(|opt| opt.map(SecretString::new))
    }
}
