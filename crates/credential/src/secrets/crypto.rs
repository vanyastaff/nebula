//! OAuth2/PKCE utilities + base64 serde helpers for credential state.
//!
//! AES-256-GCM encryption and Argon2id KDF moved to `nebula-crypto` (ADR-0088).
//! PKCE / OAuth-state helpers stay here because they travel with the OAuth
//! protocol, not generic byte crypto.

use sha2::{Digest, Sha256};

// ============================================================================
// OAuth2/PKCE Utilities
// ============================================================================

/// Generate random state parameter for OAuth2 (URL-safe base64)
#[must_use]
pub fn generate_random_state() -> String {
    use rand::RngExt;
    let mut rng = rand::rng();
    let random_bytes: [u8; 32] = rng.random();
    base64_url_encode(&random_bytes)
}

/// Generate PKCE code verifier (43-128 characters, URL-safe)
#[must_use]
pub fn generate_pkce_verifier() -> String {
    use rand::RngExt;
    let mut rng = rand::rng();
    let random_bytes: [u8; 32] = rng.random();
    base64_url_encode(&random_bytes)
}

/// Generate PKCE code challenge from verifier using S256 method
#[must_use]
pub fn generate_code_challenge(verifier: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let hash = hasher.finalize();
    base64_url_encode(&hash)
}

/// Encode bytes as URL-safe base64 (no padding)
fn base64_url_encode(input: &[u8]) -> String {
    base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, input)
}

/// Serde helpers for base64 encoding of byte vectors.
///
/// Use with `#[serde(with = "crate::secrets::serde_base64")]` on `Vec<u8>` fields
/// to serialize as base64 strings in JSON, ensuring binary data survives
/// round-trips.
pub mod serde_base64 {
    use base64::{Engine, engine::general_purpose::STANDARD};
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_random_state() {
        let state1 = generate_random_state();
        let state2 = generate_random_state();

        assert_ne!(state1, state2);
        assert!(!state1.contains('+'));
        assert!(!state1.contains('/'));
        assert!(!state1.contains('='));
    }

    #[test]
    fn test_pkce_flow() {
        let verifier = generate_pkce_verifier();
        let challenge = generate_code_challenge(&verifier);

        assert_eq!(verifier.len(), 43);
        assert_eq!(challenge.len(), 43);

        let challenge2 = generate_code_challenge(&verifier);
        assert_eq!(challenge, challenge2);

        let verifier2 = generate_pkce_verifier();
        let challenge3 = generate_code_challenge(&verifier2);
        assert_ne!(challenge, challenge3);
    }

    #[test]
    fn test_pkce_rfc7636_example() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let expected_challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

        let challenge = generate_code_challenge(verifier);
        assert_eq!(challenge, expected_challenge);
    }
}
