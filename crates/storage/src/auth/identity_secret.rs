//! Versioned, user-bound envelopes for Plane-A identity secrets.
//!
//! Active TOTP seeds and pending enrollment candidates share the platform
//! credential [`KeyProvider`](crate::credential::KeyProvider) but use distinct,
//! versioned AAD domains. An
//! enrollment ciphertext therefore cannot be copied into the active-factor
//! column: confirmation must decrypt the exact candidate and re-seal it for
//! the active purpose.
//!
//! The codec also owns the explicit decrypt-only legacy-key seam used during
//! operator-controlled rotation. Opening a legacy envelope returns a current-
//! key replacement alongside zeroizing plaintext; the repository persists the
//! replacement with CAS before it exposes the backend as ready.

use std::{collections::HashMap, sync::Arc};

use nebula_crypto::{EncryptedData, EncryptionKey, decrypt_with_aad, encrypt_with_key_id};
use thiserror::Error;
use zeroize::Zeroizing;

use crate::credential::{KeyProvider, ProviderError};

const USER_ID_BYTES: usize = 16;
const TOTP_SEED_BASE32_BYTES: usize = 32;
const MAX_ENVELOPE_BYTES: usize = 4 * 1024;
const MAX_KEY_ID_BYTES: usize = 255;
const MAX_LEGACY_KEYS: usize = 16;
const ACTIVE_TOTP_AAD_DOMAIN: &[u8] = b"nebula:plane-a:totp:active:v1\0";
const CANDIDATE_TOTP_AAD_DOMAIN: &[u8] = b"nebula:plane-a:totp:enrollment-candidate:v1\0";

/// Lifecycle authority to which a TOTP-seed ciphertext is bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TotpSecretPurpose {
    /// The factor currently authorized for login verification.
    Active,
    /// A pending factor that must be verified before it can become active.
    EnrollmentCandidate,
}

impl TotpSecretPurpose {
    const fn aad_domain(self) -> &'static [u8] {
        match self {
            Self::Active => ACTIVE_TOTP_AAD_DOMAIN,
            Self::EnrollmentCandidate => CANDIDATE_TOTP_AAD_DOMAIN,
        }
    }
}

/// A successfully authenticated identity-secret envelope.
///
/// `plaintext` is scrubbed on drop. `replacement_envelope` is present only
/// when the original used an explicitly configured legacy key and must be
/// atomically replaced with a current-key envelope.
#[must_use = "legacy-key openings must persist their replacement envelope"]
pub struct OpenedIdentitySecret {
    /// Authenticated TOTP seed bytes, zeroized on drop.
    pub plaintext: Zeroizing<Vec<u8>>,
    /// Current-key envelope to persist when rotation was required.
    pub replacement_envelope: Option<Vec<u8>>,
}

impl std::fmt::Debug for OpenedIdentitySecret {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OpenedIdentitySecret")
            .field("plaintext", &"[redacted]")
            .field(
                "replacement_envelope",
                &self.replacement_envelope.as_ref().map(|_| "[redacted]"),
            )
            .finish()
    }
}

/// Fixed, secret-free failures from identity-envelope operations.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum IdentitySecretError {
    /// The current platform key could not be loaded.
    #[error("identity encryption key is unavailable")]
    KeyUnavailable(#[source] ProviderError),
    /// The current provider or legacy keyring violates key-id invariants.
    #[error("identity encryption keyring is invalid")]
    InvalidKeyring,
    /// Identity storage supplied a malformed user identifier.
    #[error("identity secret owner is invalid")]
    InvalidOwner,
    /// A TOTP seed was empty or exceeded the supported bound.
    #[error("identity TOTP seed has an invalid length")]
    InvalidPlaintext,
    /// Stored bytes were not a bounded, supported identity envelope.
    #[error("identity secret envelope is invalid")]
    InvalidEnvelope,
    /// No explicit decrypt-only key matched the stored key id.
    #[error("identity secret legacy key is unavailable")]
    LegacyKeyUnavailable,
    /// AES-256-GCM rejected the key, ciphertext, AAD, or authentication tag.
    #[error("identity secret authentication failed")]
    AuthenticationFailed,
    /// A current-key envelope could not be produced.
    #[error("identity secret encryption failed")]
    EncryptionFailed,
}

/// Rotation-aware AES-256-GCM codec for Plane-A TOTP seeds.
///
/// Construction snapshots the provider's current key and non-secret key id.
/// The built-in environment/file providers are immutable after construction;
/// rotating them means building a fresh codec at process startup with the old
/// key registered explicitly through [`Self::with_legacy_keys`].
pub struct IdentitySecretCodec {
    current_key: Arc<EncryptionKey>,
    current_key_id: String,
    legacy_keys: HashMap<String, Arc<EncryptionKey>>,
}

impl IdentitySecretCodec {
    /// Build a codec with one fail-closed current key and no legacy keys.
    ///
    /// # Errors
    ///
    /// Returns [`IdentitySecretError::KeyUnavailable`] when the provider
    /// cannot load key material, or [`IdentitySecretError::InvalidKeyring`]
    /// when its key id is empty or unreasonably large.
    pub fn new(key_provider: Arc<dyn KeyProvider>) -> Result<Self, IdentitySecretError> {
        Self::with_legacy_keys(key_provider, Vec::new())
    }

    /// Build a codec with explicit decrypt-only keys for bounded rotation.
    ///
    /// Legacy ids must be non-empty, unique, distinct from the current id,
    /// and the map is capped to prevent configuration-driven memory growth.
    ///
    /// # Errors
    ///
    /// Returns [`IdentitySecretError::InvalidKeyring`] for any violated
    /// invariant, or [`IdentitySecretError::KeyUnavailable`] when the current
    /// provider cannot load its key.
    pub fn with_legacy_keys(
        key_provider: Arc<dyn KeyProvider>,
        legacy_keys: Vec<(String, Arc<EncryptionKey>)>,
    ) -> Result<Self, IdentitySecretError> {
        let snapshot = key_provider
            .current()
            .map_err(IdentitySecretError::KeyUnavailable)?;
        let (current_key_id, current_key) = snapshot.into_parts();
        if !valid_key_id(&current_key_id) || legacy_keys.len() > MAX_LEGACY_KEYS {
            return Err(IdentitySecretError::InvalidKeyring);
        }
        let mut indexed_legacy_keys = HashMap::with_capacity(legacy_keys.len());
        for (key_id, key) in legacy_keys {
            if !valid_key_id(&key_id)
                || key_id.as_str() == current_key_id.as_ref()
                || indexed_legacy_keys.insert(key_id, key).is_some()
            {
                return Err(IdentitySecretError::InvalidKeyring);
            }
        }
        Ok(Self {
            current_key,
            current_key_id: current_key_id.to_string(),
            legacy_keys: indexed_legacy_keys,
        })
    }

    /// Encrypt one TOTP seed for its owner and lifecycle purpose.
    ///
    /// # Errors
    ///
    /// Rejects non-16-byte owners, empty/oversized seeds, and cryptographic or
    /// serialization failures. Error values never carry plaintext or envelope
    /// bytes.
    pub fn seal_totp_seed(
        &self,
        purpose: TotpSecretPurpose,
        user_id: &[u8],
        plaintext: &[u8],
    ) -> Result<Vec<u8>, IdentitySecretError> {
        validate_owner(user_id)?;
        if !is_canonical_totp_seed(plaintext) {
            return Err(IdentitySecretError::InvalidPlaintext);
        }
        let aad = totp_aad(purpose, user_id);
        let encrypted =
            encrypt_with_key_id(&self.current_key, &self.current_key_id, plaintext, &aad)
                .map_err(|_| IdentitySecretError::EncryptionFailed)?;
        serde_json::to_vec(&encrypted).map_err(|_| IdentitySecretError::EncryptionFailed)
    }

    /// Authenticate and decrypt one owner/purpose-bound TOTP envelope.
    ///
    /// # Errors
    ///
    /// Fails closed for malformed/oversized envelopes, an unsupported version,
    /// an unknown legacy key id, a different owner or purpose, tampering, or a
    /// wrong key. No error embeds stored bytes, key material, or plaintext.
    pub fn open_totp_seed(
        &self,
        purpose: TotpSecretPurpose,
        user_id: &[u8],
        envelope: &[u8],
    ) -> Result<OpenedIdentitySecret, IdentitySecretError> {
        validate_owner(user_id)?;
        if envelope.is_empty() || envelope.len() > MAX_ENVELOPE_BYTES {
            return Err(IdentitySecretError::InvalidEnvelope);
        }
        let encrypted: EncryptedData =
            serde_json::from_slice(envelope).map_err(|_| IdentitySecretError::InvalidEnvelope)?;
        if !encrypted.is_supported_version()
            || encrypted.key_id.is_empty()
            || encrypted.key_id.len() > MAX_KEY_ID_BYTES
            || encrypted.ciphertext.len() != TOTP_SEED_BASE32_BYTES
        {
            return Err(IdentitySecretError::InvalidEnvelope);
        }

        let (key, requires_rotation) = if encrypted.key_id == self.current_key_id {
            (Arc::clone(&self.current_key), false)
        } else {
            let key = self
                .legacy_keys
                .get(&encrypted.key_id)
                .cloned()
                .ok_or(IdentitySecretError::LegacyKeyUnavailable)?;
            (key, true)
        };
        let aad = totp_aad(purpose, user_id);
        let plaintext = decrypt_with_aad(&key, &encrypted, &aad)
            .map_err(|_| IdentitySecretError::AuthenticationFailed)?;
        if !is_canonical_totp_seed(&plaintext) {
            return Err(IdentitySecretError::InvalidPlaintext);
        }
        let replacement_envelope = if requires_rotation {
            Some(self.seal_totp_seed(purpose, user_id, &plaintext)?)
        } else {
            None
        };
        Ok(OpenedIdentitySecret {
            plaintext,
            replacement_envelope,
        })
    }

    /// Non-secret identifier of the key used for newly sealed envelopes.
    #[must_use]
    pub fn current_key_id(&self) -> &str {
        &self.current_key_id
    }
}

impl std::fmt::Debug for IdentitySecretCodec {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("IdentitySecretCodec")
            .field("current_key_id", &self.current_key_id)
            .field("current_key", &"[redacted]")
            .field("legacy_key_count", &self.legacy_keys.len())
            .finish()
    }
}

fn valid_key_id(key_id: &str) -> bool {
    !key_id.is_empty() && key_id.len() <= MAX_KEY_ID_BYTES && !key_id.chars().any(char::is_control)
}

fn validate_owner(user_id: &[u8]) -> Result<(), IdentitySecretError> {
    if user_id.len() == USER_ID_BYTES {
        Ok(())
    } else {
        Err(IdentitySecretError::InvalidOwner)
    }
}

fn totp_aad(purpose: TotpSecretPurpose, user_id: &[u8]) -> Vec<u8> {
    let domain = purpose.aad_domain();
    let mut aad = Vec::with_capacity(domain.len() + USER_ID_BYTES);
    aad.extend_from_slice(domain);
    aad.extend_from_slice(user_id);
    aad
}

/// Validate the historical/current v1 representation: canonical unpadded
/// RFC 4648 Base32 of exactly 20 decoded bytes (32 encoded bytes). Issuance,
/// not this structural validator, is responsible for CSPRNG entropy.
pub(crate) fn is_canonical_totp_seed(bytes: &[u8]) -> bool {
    if bytes.len() != TOTP_SEED_BASE32_BYTES {
        return false;
    }
    let Some(decoded) = decode_totp_base32(bytes) else {
        return false;
    };
    let decoded = Zeroizing::new(decoded);
    let reencoded = Zeroizing::new(encode_totp_base32(&decoded));
    reencoded.as_slice() == bytes
}

fn decode_totp_base32(bytes: &[u8]) -> Option<[u8; 20]> {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut output = [0_u8; 20];
    let mut output_index = 0_usize;
    let mut accumulator = 0_u32;
    let mut available_bits = 0_u8;
    for byte in bytes {
        let value = ALPHABET.iter().position(|candidate| candidate == byte)? as u32;
        accumulator = (accumulator << 5) | value;
        available_bits += 5;
        if available_bits >= 8 {
            available_bits -= 8;
            *output.get_mut(output_index)? = (accumulator >> available_bits) as u8;
            output_index += 1;
            accumulator &= (1_u32 << available_bits).wrapping_sub(1);
        }
    }
    (output_index == output.len() && available_bits == 0).then_some(output)
}

fn encode_totp_base32(bytes: &[u8; 20]) -> [u8; TOTP_SEED_BASE32_BYTES] {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut output = [0_u8; TOTP_SEED_BASE32_BYTES];
    let mut output_index = 0_usize;
    let mut accumulator = 0_u32;
    let mut available_bits = 0_u8;
    for byte in bytes {
        accumulator = (accumulator << 8) | u32::from(*byte);
        available_bits += 8;
        while available_bits >= 5 {
            available_bits -= 5;
            let index = ((accumulator >> available_bits) & 0x1f) as usize;
            output[output_index] = ALPHABET[index];
            output_index += 1;
            accumulator &= (1_u32 << available_bits).wrapping_sub(1);
        }
    }
    debug_assert_eq!(output_index, output.len());
    debug_assert_eq!(available_bits, 0);
    output
}
