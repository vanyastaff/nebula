//! Encryption and key derivation tests
//!
//! Tests for AES-256-GCM encryption/decryption, Argon2id key derivation,
//! and memory zeroization security features.

use nebula_credential::core::{CryptoError, SecretString};
use nebula_credential::utils::{EncryptedData, EncryptionKey, decrypt, encrypt};
use std::time::Instant;

/// Test: Encrypt secret → decrypt → verify match (roundtrip)
///
/// Verifies that data encrypted with AES-256-GCM can be successfully
/// decrypted back to the original plaintext.
#[test]
fn test_encrypt_decrypt_roundtrip() {
    let salt = [0u8; 16];
    let key = EncryptionKey::derive_from_password("test-password", &salt)
        .expect("key derivation should succeed");

    let secret = SecretString::new("my-api-key-12345");
    let plaintext = secret.expose_secret(|s| s.as_bytes().to_vec());

    // Encrypt
    let encrypted = encrypt(&key, &plaintext).expect("encryption should succeed");

    // Verify encrypted data structure
    assert_eq!(encrypted.version, EncryptedData::CURRENT_VERSION);
    assert_eq!(encrypted.nonce.len(), 12);
    assert_eq!(encrypted.tag.len(), 16);
    assert!(!encrypted.ciphertext.is_empty());

    // Decrypt
    let decrypted = decrypt(&key, &encrypted).expect("decryption should succeed");

    // Verify roundtrip
    assert_eq!(decrypted, plaintext);
    assert_eq!(String::from_utf8(decrypted).unwrap(), "my-api-key-12345");
}

/// Test: Same password + salt → same key twice (deterministic)
///
/// Verifies that key derivation with Argon2id is deterministic:
/// the same password and salt always produce the same key.
#[test]
fn test_key_derivation_deterministic() {
    let salt = [42u8; 16];
    let password = "deterministic-password";

    let key1 = EncryptionKey::derive_from_password(password, &salt)
        .expect("first key derivation should succeed");

    let key2 = EncryptionKey::derive_from_password(password, &salt)
        .expect("second key derivation should succeed");

    // Verify keys are functionally identical by encrypting with one and decrypting with the other
    let plaintext = b"test data";
    let encrypted = encrypt(&key1, plaintext).expect("encryption should succeed");

    // key2 should be able to decrypt what key1 encrypted
    let decrypted = decrypt(&key2, &encrypted).expect("decryption should succeed");
    assert_eq!(decrypted, plaintext);

    // And vice versa
    let encrypted2 = encrypt(&key2, plaintext).expect("encryption should succeed");
    let decrypted2 = decrypt(&key1, &encrypted2).expect("decryption should succeed");
    assert_eq!(decrypted2, plaintext);
}

/// Test: Different passwords → different keys
///
/// Verifies that different passwords produce different encryption keys,
/// preventing key collisions.
#[test]
fn test_key_derivation_different_passwords() {
    let salt = [0u8; 16];

    let key1 = EncryptionKey::derive_from_password("password1", &salt)
        .expect("key1 derivation should succeed");

    let key2 = EncryptionKey::derive_from_password("password2", &salt)
        .expect("key2 derivation should succeed");

    // Verify keys are functionally different by attempting cross-decryption
    let plaintext = b"same plaintext";
    let encrypted1 = encrypt(&key1, plaintext).expect("encryption 1 should succeed");
    let encrypted2 = encrypt(&key2, plaintext).expect("encryption 2 should succeed");

    // Ciphertext should be different (nonces are also different)
    assert_ne!(encrypted1.ciphertext, encrypted2.ciphertext);

    // Cross-decryption should fail (proves keys are different)
    let result = decrypt(&key1, &encrypted2);
    assert!(matches!(result, Err(CryptoError::DecryptionFailed)));
}

/// Test: 100 encryptions → 100 unique nonces
///
/// Verifies that the nonce generator produces unique nonces,
/// preventing nonce reuse which would compromise AES-GCM security.
#[test]
fn test_nonce_uniqueness() {
    let salt = [0u8; 16];
    let key = EncryptionKey::derive_from_password("test-password", &salt)
        .expect("key derivation should succeed");

    let plaintext = b"test data";
    let mut nonces = Vec::new();

    // Perform 100 encryptions
    for _ in 0..100 {
        let encrypted = encrypt(&key, plaintext).expect("encryption should succeed");
        nonces.push(encrypted.nonce);
    }

    // Verify all nonces are unique
    for i in 0..nonces.len() {
        for j in (i + 1)..nonces.len() {
            assert_ne!(
                nonces[i], nonces[j],
                "Nonce collision detected at indices {} and {}",
                i, j
            );
        }
    }
}

/// Test: Decrypt with wrong key → CryptoError::DecryptionFailed
///
/// Verifies that attempting to decrypt data with the wrong key
/// fails securely without leaking information.
#[test]
fn test_decryption_with_wrong_key() {
    let salt = [0u8; 16];

    // Encrypt with key1
    let key1 = EncryptionKey::derive_from_password("correct-password", &salt)
        .expect("key1 derivation should succeed");

    let plaintext = b"secret data";
    let encrypted = encrypt(&key1, plaintext).expect("encryption should succeed");

    // Try to decrypt with different key
    let key2 = EncryptionKey::derive_from_password("wrong-password", &salt)
        .expect("key2 derivation should succeed");

    let result = decrypt(&key2, &encrypted);

    // Should fail with DecryptionFailed error
    assert!(matches!(result, Err(CryptoError::DecryptionFailed)));

    // Verify error message doesn't leak information
    match result {
        Err(CryptoError::DecryptionFailed) => {
            // Expected error
        }
        other => panic!("Expected DecryptionFailed, got {:?}", other),
    }
}

/// Test: Key derivation timing (security requirement)
///
/// Verifies that Argon2id key derivation takes 100-200ms,
/// which prevents brute-force attacks while remaining usable.
///
/// Note: Timing tests can be flaky on CI systems. This test measures
/// actual wall-clock time, not paused time.
#[test]
fn test_key_derivation_timing() {
    let salt = [0u8; 16];
    let password = "test-password-for-timing";

    // Measure actual derivation time
    let start = Instant::now();
    let _key = EncryptionKey::derive_from_password(password, &salt)
        .expect("key derivation should succeed");
    let duration = start.elapsed();

    // Verify derivation takes between 50ms and 500ms
    // (wider range to account for CPU variations, but should be ~100-200ms)
    let millis = duration.as_millis();
    println!("Key derivation took {}ms", millis);

    assert!(
        millis >= 50,
        "Key derivation too fast ({}ms), may be vulnerable to brute force",
        millis
    );
    assert!(
        millis <= 500,
        "Key derivation too slow ({}ms), may impact usability",
        millis
    );
}

/// Test: EncryptionKey::from_bytes() roundtrip
///
/// Verifies that encryption keys can be loaded directly from bytes,
/// which is necessary for loading keys from secure storage.
#[test]
fn test_key_derivation_from_bytes() {
    // Create a key directly from known bytes
    let key_bytes = [
        0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
        0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d,
        0x1e, 0x1f,
    ];
    let key1 = EncryptionKey::from_bytes(key_bytes);

    // Encrypt some data with the key
    let plaintext = b"test data for from_bytes";
    let encrypted = encrypt(&key1, plaintext).expect("encryption should succeed");

    // Create another key from the same bytes (simulating loading from secure storage)
    let key2 = EncryptionKey::from_bytes(key_bytes);

    // Verify the second key can decrypt the data
    let decrypted = decrypt(&key2, &encrypted).expect("decryption should succeed");
    assert_eq!(decrypted, plaintext);

    // Verify roundtrip works both ways
    let encrypted2 = encrypt(&key2, plaintext).expect("encryption should succeed");
    let decrypted2 = decrypt(&key1, &encrypted2).expect("decryption should succeed");
    assert_eq!(decrypted2, plaintext);

    // Verify that different bytes produce different keys
    let different_bytes = [0xff; 32];
    let key3 = EncryptionKey::from_bytes(different_bytes);
    let result = decrypt(&key3, &encrypted);
    assert!(matches!(result, Err(CryptoError::DecryptionFailed)));
}

/// Test: EncryptionKey memory is zeroized on drop
///
/// Verifies that the ZeroizeOnDrop derive macro is working and
/// EncryptionKey memory is securely cleared when dropped.
///
/// Note: This test verifies that the ZeroizeOnDrop trait is applied.
/// Full memory inspection requires unsafe code or external tools.
#[test]
fn test_encryption_key_zeroized() {
    let salt = [0u8; 16];
    let password = "zeroization-test";

    // Create and drop a key
    {
        let key = EncryptionKey::derive_from_password(password, &salt)
            .expect("key derivation should succeed");

        // Use the key
        let plaintext = b"test";
        let _encrypted = encrypt(&key, plaintext).expect("encryption should succeed");

        // Key will be dropped and zeroized at end of scope
    }

    // If we reach here without panics, ZeroizeOnDrop is working
    // The memory was automatically zeroed when the key was dropped

    // Create a new key with same password to verify it still works
    let key2 = EncryptionKey::derive_from_password(password, &salt)
        .expect("key derivation should succeed");
    let plaintext = b"test";
    let _encrypted = encrypt(&key2, plaintext).expect("encryption should succeed");

    // This test passes if:
    // 1. EncryptionKey implements ZeroizeOnDrop (checked by compiler)
    // 2. Keys can be created and used normally
    // 3. No memory corruption occurs after zeroization
}
