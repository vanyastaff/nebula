//! Utility modules

pub mod crypto;
pub mod retry;
pub mod secret_string;
pub mod time;
pub mod validation;

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
pub use time::{from_unix_timestamp, to_unix_timestamp, unix_now};
pub use validation::validate_encrypted_size;
