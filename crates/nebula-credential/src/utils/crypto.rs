//! Cryptographic utilities for credentials
//!
//! Provides AES-256-GCM encryption with Argon2id key derivation,
//! OAuth2/PKCE utilities, and secure random generation.

use crate::core::CryptoError;
use aes_gcm::{
    Aes256Gcm,
    aead::{Aead, KeyInit},
};
use argon2::{Argon2, ParamsBuilder};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use zeroize::{Zeroize, ZeroizeOnDrop};

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
    pub fn new(nonce: [u8; 12], ciphertext: Vec<u8>, tag: [u8; 16]) -> Self {
        Self {
            version: Self::CURRENT_VERSION,
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

/// Nonce generator with atomic counter (prevents reuse)
struct NonceGenerator {
    counter: AtomicU64,
}

impl NonceGenerator {
    fn new() -> Self {
        Self {
            counter: AtomicU64::new(0),
        }
    }

    fn next(&self) -> aes_gcm::Nonce<aes_gcm::aes::cipher::typenum::U12> {
        let value = self.counter.fetch_add(1, Ordering::SeqCst);
        // Convert u64 to 12-byte nonce (8 bytes value + 4 bytes zeros)
        let mut nonce_bytes = [0u8; 12];
        nonce_bytes[0..8].copy_from_slice(&value.to_le_bytes());
        *aes_gcm::Nonce::from_slice(&nonce_bytes)
    }
}

// Global nonce generator (thread-safe)
static NONCE_GEN: OnceLock<NonceGenerator> = OnceLock::new();

fn nonce_generator() -> &'static NonceGenerator {
    NONCE_GEN.get_or_init(NonceGenerator::new)
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
pub fn encrypt(key: &EncryptionKey, plaintext: &[u8]) -> Result<EncryptedData, CryptoError> {
    let cipher = Aes256Gcm::new_from_slice(key.as_bytes())
        .map_err(|e| CryptoError::EncryptionFailed(e.to_string()))?;

    let nonce = nonce_generator().next();

    let ciphertext = cipher
        .encrypt(&nonce, plaintext)
        .map_err(|e| CryptoError::EncryptionFailed(e.to_string()))?;

    // Split ciphertext and tag (last 16 bytes)
    let (ct, tag_slice) = ciphertext.split_at(ciphertext.len() - 16);
    let mut tag = [0u8; 16];
    tag.copy_from_slice(tag_slice);

    let mut nonce_bytes = [0u8; 12];
    nonce_bytes.copy_from_slice(nonce.as_slice());

    Ok(EncryptedData::new(nonce_bytes, ct.to_vec(), tag))
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
pub fn decrypt(key: &EncryptionKey, encrypted: &EncryptedData) -> Result<Vec<u8>, CryptoError> {
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

    Ok(plaintext)
}

// ============================================================================
// OAuth2/PKCE Utilities
// ============================================================================

/// Generate random state parameter for OAuth2 (URL-safe base64)
#[must_use]
pub fn generate_random_state() -> String {
    use rand::Rng;
    let mut rng = rand::rng();
    let random_bytes: [u8; 32] = rng.random();
    base64_url_encode(&random_bytes)
}

/// Generate PKCE code verifier (43-128 characters, URL-safe)
#[must_use]
pub fn generate_pkce_verifier() -> String {
    use rand::Rng;
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

        let encrypted = EncryptedData::new(nonce, ciphertext.clone(), tag);
        assert_eq!(encrypted.version, EncryptedData::CURRENT_VERSION);
        assert_eq!(encrypted.nonce, nonce);
        assert_eq!(encrypted.ciphertext, ciphertext);
        assert_eq!(encrypted.tag, tag);
    }

    #[test]
    fn test_encrypted_data_version_check() {
        let encrypted = EncryptedData::new([0u8; 12], vec![], [0u8; 16]);
        assert!(encrypted.is_supported_version());

        let mut unsupported = encrypted.clone();
        unsupported.version = 99;
        assert!(!unsupported.is_supported_version());
    }

    #[test]
    fn test_nonce_generator_uniqueness() {
        let generator = NonceGenerator::new();
        let nonce1 = generator.next();
        let nonce2 = generator.next();
        let nonce3 = generator.next();

        assert_ne!(nonce1.as_slice(), nonce2.as_slice());
        assert_ne!(nonce2.as_slice(), nonce3.as_slice());
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
