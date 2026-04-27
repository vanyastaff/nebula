//! Cryptographic utilities for credentials
//!
//! Provides AES-256-GCM encryption with Argon2id key derivation,
//! OAuth2/PKCE utilities, and secure random generation.

use aes_gcm::{
    Aes256Gcm,
    aead::{Aead, KeyInit, Payload},
};
use argon2::{Argon2, ParamsBuilder};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::error::CryptoError;

// ============================================================================
// Encryption & Key Derivation
// ============================================================================

/// 256-bit AES encryption key with automatic zeroization
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct EncryptionKey {
    key: [u8; 32], // 256 bits
}

impl EncryptionKey {
    /// Derive encryption key from password using Argon2id
    ///
    /// Parameters: 19 MiB memory, 2 iterations, 32-byte output
    /// Takes 100-200ms for security (prevents brute force)
    ///
    /// # Arguments
    ///
    /// * `password` - Master password for key derivation
    /// * `salt` - 16-byte salt (must be stored with encrypted data)
    ///
    /// # Errors
    ///
    /// Returns `CryptoError::KeyDerivation` if derivation fails
    pub fn derive_from_password(password: &str, salt: &[u8; 16]) -> Result<Self, CryptoError> {
        let params = ParamsBuilder::new()
            .m_cost(19456) // 19 MiB memory
            .t_cost(2) // 2 iterations
            .p_cost(1) // 1 thread
            .output_len(32)
            .build()
            .map_err(|e| CryptoError::KeyDerivation(e.to_string()))?;

        let argon2 = Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);

        let mut key = [0u8; 32];
        argon2
            .hash_password_into(password.as_bytes(), salt, &mut key)
            .map_err(|e| CryptoError::KeyDerivation(e.to_string()))?;

        Ok(Self { key })
    }

    /// Load key directly from bytes (from secure storage)
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self { key: bytes }
    }

    /// Get key bytes for cryptographic operations
    pub(crate) fn as_bytes(&self) -> &[u8; 32] {
        &self.key
    }
}

/// Encrypted credential data with authentication
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedData {
    /// Algorithm version for future migrations
    pub version: u8,

    /// Which encryption key was used. Empty after deserialization of
    /// pre-rotation data (`#[serde(default)]`). The encryption layer
    /// rejects empty key IDs at runtime — a migration must re-encrypt
    /// all records before upgrading.
    #[serde(default)]
    pub key_id: String,

    /// 96-bit nonce (12 bytes) for AES-GCM
    pub nonce: [u8; 12],

    /// Encrypted ciphertext
    pub ciphertext: Vec<u8>,

    /// 128-bit authentication tag (16 bytes)
    pub tag: [u8; 16],
}

impl EncryptedData {
    /// Current encryption version (AES-256-GCM)
    pub const CURRENT_VERSION: u8 = 1;

    /// Create new encrypted data structure
    pub fn new(
        key_id: impl Into<String>,
        nonce: [u8; 12],
        ciphertext: Vec<u8>,
        tag: [u8; 16],
    ) -> Self {
        Self {
            version: Self::CURRENT_VERSION,
            key_id: key_id.into(),
            nonce,
            ciphertext,
            tag,
        }
    }

    /// Check if version is supported
    pub fn is_supported_version(&self) -> bool {
        self.version == Self::CURRENT_VERSION
    }
}

// Previous versions of this module wiped `ciphertext`/`nonce`/`tag` on
// Drop, but these are public ciphertext bytes by design — they live on
// disk and in envelopes. Only plaintext (already wrapped in `Zeroizing`)
// needs scrubbing. No `Drop` impl here; the default auto-derived drop
// releases the Vec without burning cycles on a false sense of security.

/// Generate a fresh 96-bit random AES-GCM nonce.
///
/// Per NIST SP 800-38D §8.2.2 a fully-random 96-bit nonce is safe up to
/// roughly 2^32 encryptions per key under the birthday bound. The previous
/// implementation combined a 64-bit in-process counter with only 32 bits of
/// randomness, which left exactly 32 bits of collision protection across
/// restarts — after ~65 k restart-encryptions per key the collision
/// probability crossed 50%. AES-GCM nonce reuse is catastrophic (full
/// plaintext recovery + authentication forgery), so we take the NIST random
/// path and read the full 12 bytes from the OS CSPRNG on every call.
fn fresh_nonce() -> aes_gcm::Nonce<aes_gcm::aes::cipher::typenum::U12> {
    use rand::RngExt;

    let mut rng = rand::rng();
    let nonce_bytes: [u8; 12] = rng.random();
    *aes_gcm::Nonce::from_slice(&nonce_bytes)
}

/// Encrypt plaintext using AES-256-GCM
///
/// # Arguments
///
/// * `key` - Encryption key (256 bits)
/// * `plaintext` - Data to encrypt
///
/// # Returns
///
/// Encrypted data with nonce and authentication tag
///
/// # Errors
///
/// Returns `CryptoError::EncryptionFailed` if encryption fails
///
/// **SEC-11 (security hardening 2026-04-27 Stage 1).** Renamed from
/// `encrypt` and gated `#[cfg(test)]` to remove the no-AAD path from any
/// production surface. Plugins, external callers, and even
/// non-test internal code cannot construct legacy (no-AAD) envelopes
/// anymore. The function remains reachable strictly inside this module's
/// `mod tests` for the `decrypt_no_aad_data_with_aad_fails` regression
/// test that pins the AAD-mandatory rejection on the decrypt side.
#[cfg(test)]
fn encrypt_no_aad(key: &EncryptionKey, plaintext: &[u8]) -> Result<EncryptedData, CryptoError> {
    let cipher = Aes256Gcm::new_from_slice(key.as_bytes())
        .map_err(|e| CryptoError::EncryptionFailed(e.to_string()))?;

    let nonce = fresh_nonce();

    let ciphertext = cipher
        .encrypt(&nonce, plaintext)
        .map_err(|e| CryptoError::EncryptionFailed(e.to_string()))?;

    // Split ciphertext and tag (last 16 bytes)
    let (ct, tag_slice) = ciphertext.split_at(ciphertext.len() - 16);
    let mut tag = [0u8; 16];
    tag.copy_from_slice(tag_slice);

    let mut nonce_bytes = [0u8; 12];
    nonce_bytes.copy_from_slice(nonce.as_slice());

    Ok(EncryptedData::new("", nonce_bytes, ct.to_vec(), tag))
}

/// Decrypt ciphertext using AES-256-GCM with constant-time tag comparison
///
/// # Arguments
///
/// * `key` - Encryption key (256 bits)
/// * `encrypted` - Encrypted data with nonce and tag
///
/// # Returns
///
/// Decrypted plaintext
///
/// # Errors
///
/// Returns `CryptoError::DecryptionFailed` if decryption fails
/// Returns `CryptoError::UnsupportedVersion` if encryption version not supported
pub fn decrypt(
    key: &EncryptionKey,
    encrypted: &EncryptedData,
) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
    if !encrypted.is_supported_version() {
        return Err(CryptoError::UnsupportedVersion(encrypted.version));
    }

    let cipher =
        Aes256Gcm::new_from_slice(key.as_bytes()).map_err(|_| CryptoError::DecryptionFailed)?;

    let nonce = aes_gcm::Nonce::from_slice(&encrypted.nonce);

    // Combine ciphertext and tag for decryption
    let mut ciphertext_with_tag = encrypted.ciphertext.clone();
    ciphertext_with_tag.extend_from_slice(&encrypted.tag);

    let plaintext = cipher
        .decrypt(nonce, ciphertext_with_tag.as_ref())
        .map_err(|_| CryptoError::DecryptionFailed)?;

    Ok(Zeroizing::new(plaintext))
}

/// Encrypt plaintext using AES-256-GCM with Additional Authenticated Data (AAD).
///
/// AAD binds the ciphertext to a context (e.g., credential ID), preventing
/// record-swapping attacks where encrypted data is copied between records.
/// The AAD is authenticated but not encrypted -- it must be provided again
/// at decryption time for verification.
///
/// # Arguments
///
/// * `key` - Encryption key (256 bits)
/// * `plaintext` - Data to encrypt
/// * `aad` - Additional authenticated data (e.g., credential ID bytes)
///
/// # Returns
///
/// Encrypted data with nonce and authentication tag
///
/// # Errors
///
/// Returns `CryptoError::EncryptionFailed` if encryption fails
pub fn encrypt_with_aad(
    key: &EncryptionKey,
    plaintext: &[u8],
    aad: &[u8],
) -> Result<EncryptedData, CryptoError> {
    let cipher = Aes256Gcm::new_from_slice(key.as_bytes())
        .map_err(|e| CryptoError::EncryptionFailed(e.to_string()))?;

    let nonce = fresh_nonce();

    let payload = Payload {
        msg: plaintext,
        aad,
    };

    let ciphertext = cipher
        .encrypt(&nonce, payload)
        .map_err(|e| CryptoError::EncryptionFailed(e.to_string()))?;

    // Split ciphertext and tag (last 16 bytes)
    let (ct, tag_slice) = ciphertext.split_at(ciphertext.len() - 16);
    let mut tag = [0u8; 16];
    tag.copy_from_slice(tag_slice);

    let mut nonce_bytes = [0u8; 12];
    nonce_bytes.copy_from_slice(nonce.as_slice());

    Ok(EncryptedData::new("", nonce_bytes, ct.to_vec(), tag))
}

/// Decrypt ciphertext using AES-256-GCM with Additional Authenticated Data (AAD).
///
/// The AAD provided must match the AAD used during encryption, otherwise
/// decryption fails. This prevents record-swapping attacks.
///
/// # Arguments
///
/// * `key` - Encryption key (256 bits)
/// * `encrypted` - Encrypted data with nonce and tag
/// * `aad` - Additional authenticated data (must match what was used for encryption)
///
/// # Returns
///
/// Decrypted plaintext
///
/// # Errors
///
/// Returns `CryptoError::DecryptionFailed` if decryption fails or AAD mismatch
/// Returns `CryptoError::UnsupportedVersion` if encryption version not supported
pub fn decrypt_with_aad(
    key: &EncryptionKey,
    encrypted: &EncryptedData,
    aad: &[u8],
) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
    if !encrypted.is_supported_version() {
        return Err(CryptoError::UnsupportedVersion(encrypted.version));
    }

    let cipher =
        Aes256Gcm::new_from_slice(key.as_bytes()).map_err(|_| CryptoError::DecryptionFailed)?;

    let nonce = aes_gcm::Nonce::from_slice(&encrypted.nonce);

    // Combine ciphertext and tag for decryption
    let mut ciphertext_with_tag = encrypted.ciphertext.clone();
    ciphertext_with_tag.extend_from_slice(&encrypted.tag);

    let payload = Payload {
        msg: ciphertext_with_tag.as_ref(),
        aad,
    };

    let plaintext = cipher
        .decrypt(nonce, payload)
        .map_err(|_| CryptoError::DecryptionFailed)?;

    Ok(Zeroizing::new(plaintext))
}

/// Encrypt plaintext using AES-256-GCM with AAD, recording the key identity.
///
/// Like [`encrypt_with_aad`] but stores `key_id` in the resulting [`EncryptedData`]
/// so the `EncryptionLayer` (in `nebula-storage`) can select the correct
/// decryption key during rotation.
///
/// # Arguments
///
/// * `key` - Encryption key (256 bits)
/// * `key_id` - Identifier for this key (stored in the envelope for decryption lookup)
/// * `plaintext` - Data to encrypt
/// * `aad` - Additional authenticated data (e.g., credential ID bytes)
///
/// # Returns
///
/// Encrypted data with nonce, authentication tag, and key_id
///
/// # Errors
///
/// Returns `CryptoError::EncryptionFailed` if encryption fails
pub fn encrypt_with_key_id(
    key: &EncryptionKey,
    key_id: &str,
    plaintext: &[u8],
    aad: &[u8],
) -> Result<EncryptedData, CryptoError> {
    if key_id.is_empty() {
        return Err(CryptoError::EncryptionFailed(
            "key_id must not be empty for new encryptions".into(),
        ));
    }

    let cipher = Aes256Gcm::new_from_slice(key.as_bytes())
        .map_err(|e| CryptoError::EncryptionFailed(e.to_string()))?;

    let nonce = fresh_nonce();

    let payload = Payload {
        msg: plaintext,
        aad,
    };

    let ciphertext = cipher
        .encrypt(&nonce, payload)
        .map_err(|e| CryptoError::EncryptionFailed(e.to_string()))?;

    // Split ciphertext and tag (last 16 bytes)
    let (ct, tag_slice) = ciphertext.split_at(ciphertext.len() - 16);
    let mut tag = [0u8; 16];
    tag.copy_from_slice(tag_slice);

    let mut nonce_bytes = [0u8; 12];
    nonce_bytes.copy_from_slice(nonce.as_slice());

    Ok(EncryptedData::new(key_id, nonce_bytes, ct.to_vec(), tag))
}

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

    // ========================================================================
    // Encryption Tests
    // ========================================================================

    #[test]
    fn test_encryption_key_from_bytes() {
        let bytes = [42u8; 32];
        let key = EncryptionKey::from_bytes(bytes);
        assert_eq!(key.as_bytes(), &bytes);
    }

    #[test]
    fn test_encrypted_data_new() {
        let nonce = [1u8; 12];
        let ciphertext = vec![2u8; 32];
        let tag = [3u8; 16];

        let encrypted = EncryptedData::new("key-1", nonce, ciphertext.clone(), tag);
        assert_eq!(encrypted.version, EncryptedData::CURRENT_VERSION);
        assert_eq!(encrypted.key_id, "key-1");
        assert_eq!(encrypted.nonce, nonce);
        assert_eq!(encrypted.ciphertext, ciphertext);
        assert_eq!(encrypted.tag, tag);
    }

    #[test]
    fn encrypted_data_stores_key_id() {
        let encrypted = EncryptedData::new("my-key", [0u8; 12], vec![], [0u8; 16]);
        assert_eq!(encrypted.key_id, "my-key");

        let legacy = EncryptedData::new("", [0u8; 12], vec![], [0u8; 16]);
        assert_eq!(legacy.key_id, "");
    }

    #[test]
    fn test_encrypted_data_version_check() {
        let encrypted = EncryptedData::new("", [0u8; 12], vec![], [0u8; 16]);
        assert!(encrypted.is_supported_version());

        let mut unsupported = encrypted;
        unsupported.version = 99;
        assert!(!unsupported.is_supported_version());
    }

    #[test]
    fn fresh_nonce_uniqueness() {
        // A fully-random 96-bit nonce has a 2^48 birthday bound; three
        // samples should never collide in practice.
        let nonce1 = fresh_nonce();
        let nonce2 = fresh_nonce();
        let nonce3 = fresh_nonce();

        assert_ne!(nonce1.as_slice(), nonce2.as_slice());
        assert_ne!(nonce2.as_slice(), nonce3.as_slice());
        assert_ne!(nonce1.as_slice(), nonce3.as_slice());
    }

    #[test]
    fn fresh_nonce_has_nonzero_entropy() {
        // Smoke test that the OS CSPRNG path is wired up — an all-zero
        // nonce across multiple calls would indicate it is not.
        let n1 = fresh_nonce();
        let n2 = fresh_nonce();
        let zero = [0u8; 12];
        assert!(n1.as_slice() != zero || n2.as_slice() != zero);
    }

    // ========================================================================
    // AAD Encryption Tests
    // ========================================================================

    #[test]
    fn encrypt_with_aad_round_trips() {
        let key = EncryptionKey::from_bytes([0x42; 32]);
        let plaintext = b"hello world";
        let aad = b"credential-id-1";

        let encrypted = encrypt_with_aad(&key, plaintext, aad).unwrap();
        let decrypted = decrypt_with_aad(&key, &encrypted, aad).unwrap();
        assert_eq!(decrypted.as_slice(), plaintext);
    }

    #[test]
    fn decrypt_with_wrong_aad_fails() {
        let key = EncryptionKey::from_bytes([0x42; 32]);
        let plaintext = b"hello world";

        let encrypted = encrypt_with_aad(&key, plaintext, b"cred-1").unwrap();
        let result = decrypt_with_aad(&key, &encrypted, b"cred-2");
        assert!(result.is_err());
    }

    #[test]
    fn decrypt_aad_data_without_aad_fails() {
        let key = EncryptionKey::from_bytes([0x42; 32]);
        let plaintext = b"hello world";

        // Encrypt with AAD, try to decrypt without
        let encrypted = encrypt_with_aad(&key, plaintext, b"cred-1").unwrap();
        let result = decrypt(&key, &encrypted);
        assert!(result.is_err());
    }

    #[test]
    fn encrypt_with_key_id_stores_key_id_and_round_trips() {
        let key = EncryptionKey::from_bytes([0x42; 32]);
        let plaintext = b"hello world";
        let aad = b"credential-id-1";

        let encrypted = encrypt_with_key_id(&key, "rotation-key-2", plaintext, aad).unwrap();
        assert_eq!(encrypted.key_id, "rotation-key-2");

        let decrypted = decrypt_with_aad(&key, &encrypted, aad).unwrap();
        assert_eq!(decrypted.as_slice(), plaintext);
    }

    #[test]
    fn decrypt_no_aad_data_with_aad_fails() {
        let key = EncryptionKey::from_bytes([0x42; 32]);
        let plaintext = b"hello world";

        // Encrypt without AAD, try to decrypt with AAD
        let encrypted = encrypt_no_aad(&key, plaintext).unwrap();
        let result = decrypt_with_aad(&key, &encrypted, b"cred-1");
        assert!(result.is_err());
    }

    // ========================================================================
    // OAuth2/PKCE Tests
    // ========================================================================

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
