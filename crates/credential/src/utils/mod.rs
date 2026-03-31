//! Utility modules

pub mod crypto;
pub mod retry;
pub mod secret_string;

/// Serde helpers for [`SecretString`] that preserve the actual value.
///
/// The default `SecretString` `Serialize` impl writes `[REDACTED]` (safe for
/// logs) — this module writes the real value so encrypted-at-rest state
/// round-trips correctly.
///
/// Use with `#[serde(with = "crate::utils::serde_secret")]` on `SecretString` fields.
pub mod serde_secret {
    use super::SecretString;
    use serde::{Deserialize, Deserializer, Serializer};

    /// Serialize the actual secret value (for encrypted-at-rest storage only).
    pub fn serialize<S: Serializer>(secret: &SecretString, s: S) -> Result<S::Ok, S::Error> {
        secret.expose_secret(|v| s.serialize_str(v))
    }

    /// Deserialize a string into a `SecretString`.
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<SecretString, D::Error> {
        String::deserialize(d).map(SecretString::new)
    }
}

/// Serde helpers for base64 encoding of byte vectors.
///
/// Use with `#[serde(with = "crate::utils::serde_base64")]` on `Vec<u8>` fields
/// to serialize as base64 strings in JSON, ensuring binary data survives
/// round-trips.
pub mod serde_base64 {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    use serde::{Deserialize, Deserializer, Serializer};

    /// Serialize bytes as a base64 string.
    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&STANDARD.encode(bytes))
    }

    /// Deserialize a base64 string back into bytes.
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let encoded = String::deserialize(d)?;
        STANDARD.decode(encoded).map_err(serde::de::Error::custom)
    }
}

// Re-export commonly used types and functions
pub use crypto::{
    EncryptedData, EncryptionKey, decrypt, decrypt_with_aad, encrypt, encrypt_with_aad,
    generate_code_challenge, generate_pkce_verifier, generate_random_state,
};
pub use retry::{RetryPolicy, retry_with_policy};
pub use secret_string::SecretString;
