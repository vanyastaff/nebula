//! Serde helpers for [`Option<SecretString>`] that preserve the actual value.
//!
//! Use with `#[serde(with = "nebula_credential::option_serde_secret")]`.

use serde::{Deserialize, Deserializer, Serializer};

use crate::SecretString;

/// Serialize an optional secret value (for encrypted-at-rest storage only).
pub fn serialize<S: Serializer>(secret: &Option<SecretString>, s: S) -> Result<S::Ok, S::Error> {
    match secret {
        Some(secret) => secret.expose_secret(|v| s.serialize_str(v)),
        None => s.serialize_none(),
    }
}

/// Deserialize an optional string into an `Option<SecretString>`.
pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<SecretString>, D::Error> {
    Option::<String>::deserialize(d).map(|opt| opt.map(SecretString::new))
}
