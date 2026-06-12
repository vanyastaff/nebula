//! AES-256-GCM authenticated encryption + Argon2id key derivation — cross-cutting
//! crypto primitives.
//!
//! Extracted from `nebula-credential` (ADR-0088) so the credential contract crate
//! no longer pulls `aes-gcm`/`argon2`, and `nebula-storage`'s `EncryptionLayer`
//! consumes the primitives directly. PKCE/OAuth-state helpers stay in
//! `nebula-credential` — they travel with the OAuth protocol, not generic crypto.
//!
//! Every plaintext buffer returned here is wrapped in `Zeroizing<T>`. Ciphertext
//! envelopes (`EncryptedData`) are public bytes by design and are not scrubbed.

use aes_gcm::{
    Aes256Gcm,
    aead::{Aead, KeyInit, Payload},
};
use argon2::{Argon2, ParamsBuilder};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

// ============================================================================
// Error
// ============================================================================

/// Cryptographic operation errors.
///
/// Errors from encryption, decryption, and key derivation operations.
///
/// Note: the stable `code` strings retain the `CREDENTIAL:CRYPTO_*` prefix from
/// before the ADR-0088 extract — they are consumed across the credential stack
/// and are kept stable by this move.
#[derive(Debug, Error, nebula_error::Classify)]
#[non_exhaustive]
pub enum CryptoError {
    /// Decryption failed - invalid key or corrupted data
    #[classify(category = "internal", code = "CREDENTIAL:CRYPTO_DECRYPT")]
    #[error("Decryption failed - invalid key or corrupted data")]
    DecryptionFailed,

    /// Encryption failed
    #[classify(category = "internal", code = "CREDENTIAL:CRYPTO_ENCRYPT")]
    #[error("Encryption failed: {0}")]
    EncryptionFailed(String),

    /// Key derivation failed
    #[classify(category = "internal", code = "CREDENTIAL:CRYPTO_KEY")]
    #[error("Key derivation failed: {0}")]
    KeyDerivation(String),

    /// Nonce generation failed
    #[classify(category = "internal", code = "CREDENTIAL:CRYPTO_NONCE")]
    #[error("Nonce generation failed")]
    NonceGeneration,

    /// Unsupported encryption version
    #[classify(category = "internal", code = "CREDENTIAL:CRYPTO_VERSION")]
    #[error("Unsupported encryption version: {0}")]
    UnsupportedVersion(u8),

    /// A rotation envelope was requested with an empty `key_id`. Every new
    /// encryption must record which key produced it, so the rotation lookup can
    /// select the right decryption key later.
    #[classify(category = "internal", code = "CREDENTIAL:CRYPTO_KEY_ID")]
    #[error("key_id must not be empty for new encryptions")]
    InvalidKeyId,
}

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
/// roughly 2^32 encryptions per key under the birthday bound. We read 12 bytes
/// from a thread-local CSPRNG (`rand::rng()`) on every call.
///
/// `rand::rng()` returns `ThreadRng` which is **CSPRNG-quality** — seeded from
/// `OsRng` (`getrandom`) at thread start and periodically reseeded from the OS.
/// The 96-bit nonce property required by NIST SP 800-38D §8.2.2 holds.
fn fresh_nonce() -> aes_gcm::Nonce<aes_gcm::aes::cipher::typenum::U12> {
    use rand::RngExt;

    let mut rng = rand::rng();
    let nonce_bytes: [u8; 12] = rng.random();
    *aes_gcm::Nonce::from_slice(&nonce_bytes)
}

/// Encrypt plaintext using AES-256-GCM (no AAD).
///
/// **SEC-11.** Gated `#[cfg(test)]` so the no-AAD path is not reachable from any
/// non-test build. Production callers must use [`encrypt_with_aad`] or
/// [`encrypt_with_key_id`]. Retained only for the `decrypt_no_aad_data_with_aad_fails`
/// regression test that pins the AAD-mandatory rejection on the decrypt side.
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
        return Err(CryptoError::InvalidKeyId);
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
// Cipher / Kdf ports (ADR-0092)
// ============================================================================

/// Authenticated-encryption primitive over [`EncryptionKey`] / [`EncryptedData`].
///
/// Inverts the concrete AES-256-GCM algorithm so a consumer (the credential
/// `EncryptionLayer`) can be generic over the cipher — the default
/// [`AesGcmCipher`], or a future ChaCha20-Poly1305 / HSM-backed impl — and so
/// tests can inject a fake. **SEC-11 is preserved by construction:** there is no
/// no-AAD encrypt method on this trait; every encryption binds AAD.
pub trait Cipher: Send + Sync {
    /// Encrypt with AAD, recording `key_id` for rotation key selection.
    ///
    /// Delegates to the free function [`encrypt_with_key_id`]; rejects an empty
    /// `key_id` with [`CryptoError::InvalidKeyId`].
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::EncryptionFailed`] on cipher failure or
    /// [`CryptoError::InvalidKeyId`] for an empty `key_id`.
    fn encrypt_with_key_id(
        &self,
        key: &EncryptionKey,
        key_id: &str,
        plaintext: &[u8],
        aad: &[u8],
    ) -> Result<EncryptedData, CryptoError>;

    /// Encrypt with AAD but no recorded key id (non-rotation path).
    ///
    /// Delegates to the free function [`encrypt_with_aad`].
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::EncryptionFailed`] on cipher failure.
    fn encrypt_with_aad(
        &self,
        key: &EncryptionKey,
        plaintext: &[u8],
        aad: &[u8],
    ) -> Result<EncryptedData, CryptoError>;

    /// Decrypt, verifying AAD and the algorithm version.
    ///
    /// Delegates to the free function [`decrypt_with_aad`].
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::DecryptionFailed`] on AAD mismatch or corruption,
    /// or [`CryptoError::UnsupportedVersion`] for an unknown envelope version.
    fn decrypt_with_aad(
        &self,
        key: &EncryptionKey,
        encrypted: &EncryptedData,
        aad: &[u8],
    ) -> Result<Zeroizing<Vec<u8>>, CryptoError>;
}

/// Password-based key-derivation primitive.
///
/// Inverts the concrete Argon2id KDF; the default is [`Argon2Kdf`].
pub trait Kdf: Send + Sync {
    /// Derive a 256-bit [`EncryptionKey`] from a password and 16-byte salt.
    ///
    /// Delegates to [`EncryptionKey::derive_from_password`].
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::KeyDerivation`] if derivation fails.
    fn derive(&self, password: &str, salt: &[u8; 16]) -> Result<EncryptionKey, CryptoError>;
}

/// Default [`Cipher`] — AES-256-GCM, delegating to the crate free functions.
///
/// Zero-config: a unit struct that carries no key material.
#[derive(Debug, Clone, Copy, Default)]
pub struct AesGcmCipher;

impl Cipher for AesGcmCipher {
    fn encrypt_with_key_id(
        &self,
        key: &EncryptionKey,
        key_id: &str,
        plaintext: &[u8],
        aad: &[u8],
    ) -> Result<EncryptedData, CryptoError> {
        encrypt_with_key_id(key, key_id, plaintext, aad)
    }

    fn encrypt_with_aad(
        &self,
        key: &EncryptionKey,
        plaintext: &[u8],
        aad: &[u8],
    ) -> Result<EncryptedData, CryptoError> {
        encrypt_with_aad(key, plaintext, aad)
    }

    fn decrypt_with_aad(
        &self,
        key: &EncryptionKey,
        encrypted: &EncryptedData,
        aad: &[u8],
    ) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
        decrypt_with_aad(key, encrypted, aad)
    }
}

/// Default [`Kdf`] — Argon2id, delegating to [`EncryptionKey::derive_from_password`].
#[derive(Debug, Clone, Copy, Default)]
pub struct Argon2Kdf;

impl Kdf for Argon2Kdf {
    fn derive(&self, password: &str, salt: &[u8; 16]) -> Result<EncryptionKey, CryptoError> {
        EncryptionKey::derive_from_password(password, salt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let nonce1 = fresh_nonce();
        let nonce2 = fresh_nonce();
        let nonce3 = fresh_nonce();

        assert_ne!(nonce1.as_slice(), nonce2.as_slice());
        assert_ne!(nonce2.as_slice(), nonce3.as_slice());
        assert_ne!(nonce1.as_slice(), nonce3.as_slice());
    }

    #[test]
    fn fresh_nonce_has_nonzero_entropy() {
        let n1 = fresh_nonce();
        let n2 = fresh_nonce();
        let zero = [0u8; 12];
        assert!(n1.as_slice() != zero || n2.as_slice() != zero);
    }

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
    fn empty_key_id_is_rejected() {
        let key = EncryptionKey::from_bytes([0x42; 32]);
        let err = encrypt_with_key_id(&key, "", b"plaintext", b"aad").unwrap_err();
        assert!(matches!(err, CryptoError::InvalidKeyId));
    }

    #[test]
    fn decrypt_no_aad_data_with_aad_fails() {
        let key = EncryptionKey::from_bytes([0x42; 32]);
        let plaintext = b"hello world";

        let encrypted = encrypt_no_aad(&key, plaintext).unwrap();
        let result = decrypt_with_aad(&key, &encrypted, b"cred-1");
        assert!(result.is_err());
    }

    #[test]
    fn aes_gcm_cipher_trait_round_trips_via_key_id() {
        let cipher = AesGcmCipher;
        let key = EncryptionKey::from_bytes([0x11; 32]);
        let aad = b"credential-id-7";

        let enc = cipher
            .encrypt_with_key_id(&key, "rot-1", b"secret", aad)
            .unwrap();
        assert_eq!(enc.key_id, "rot-1");
        let dec = cipher.decrypt_with_aad(&key, &enc, aad).unwrap();
        assert_eq!(dec.as_slice(), b"secret");
    }

    #[test]
    fn argon2_kdf_trait_matches_inherent_derivation() {
        let kdf = Argon2Kdf;
        let salt = [7u8; 16];
        let via_trait = kdf.derive("hunter2", &salt).unwrap();
        let via_inherent = EncryptionKey::derive_from_password("hunter2", &salt).unwrap();
        assert_eq!(via_trait.as_bytes(), via_inherent.as_bytes());
    }
}
