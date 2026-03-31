//! Error handling tests
//!
//! Tests for error messages, context propagation, and actionable error information.

use nebula_credential::{CredentialError, CryptoError, ValidationError};

/// Test: Crypto error messages don't leak secrets
#[test]
fn test_crypto_error_display() {
    let decrypt_err = CryptoError::DecryptionFailed;
    let msg = decrypt_err.to_string();
    assert!(!msg.contains("0x"), "Should not leak key bytes");
    assert!(!msg.contains("password"), "Should not leak password");
    assert!(
        msg.contains("Decryption failed"),
        "Should indicate what failed"
    );

    let encrypt_err = CryptoError::EncryptionFailed("cipher initialization failed".to_string());
    let msg = encrypt_err.to_string();
    assert!(msg.contains("Encryption failed"));
    assert!(msg.contains("cipher initialization"));

    let kd_err = CryptoError::KeyDerivation("invalid parameters".to_string());
    let msg = kd_err.to_string();
    assert!(msg.contains("Key derivation failed"));

    let nonce_err = CryptoError::NonceGeneration;
    assert_eq!(nonce_err.to_string(), "Nonce generation failed");

    let version_err = CryptoError::UnsupportedVersion(99);
    assert!(version_err.to_string().contains("99"));
}

/// Test: Validation error messages include helpful reason
#[test]
fn test_validation_error_display() {
    let empty_err = ValidationError::EmptyCredentialId;
    let msg = empty_err.to_string();
    assert!(msg.contains("empty"));
    assert!(msg.contains("Credential ID"));

    let invalid_err = ValidationError::InvalidCredentialId {
        id: "../etc/passwd".to_string(),
        reason: "contains invalid characters".to_string(),
    };
    let msg = invalid_err.to_string();
    assert!(msg.contains("../etc/passwd"));
    assert!(msg.contains("contains invalid characters"));

    let format_err = ValidationError::InvalidFormat("missing required field".to_string());
    assert!(format_err.to_string().contains("Invalid credential format"));
}

/// Test: Crypto error conversion to CredentialError
#[test]
fn test_crypto_error_conversion() {
    let crypto_err = CryptoError::DecryptionFailed;
    let cred_err: CredentialError = crypto_err.into();
    let msg = cred_err.to_string();
    assert!(msg.contains("Cryptographic error"));
    assert!(msg.contains("Decryption failed"));
}

/// Test: Validation error conversion to CredentialError
#[test]
fn test_validation_error_conversion() {
    let val_err = ValidationError::EmptyCredentialId;
    let cred_err: CredentialError = val_err.into();
    assert!(matches!(cred_err, CredentialError::Validation { .. }));
}

/// Test: Error Display format is stable
#[test]
fn test_error_display_format_stable() {
    let err = CryptoError::DecryptionFailed;
    assert_eq!(
        err.to_string(),
        "Decryption failed - invalid key or corrupted data"
    );

    let err = ValidationError::EmptyCredentialId;
    assert_eq!(err.to_string(), "Credential ID cannot be empty");
}

/// Test: Classify integration for error types
#[test]
fn test_classify_integration() {
    use nebula_error::Classify;

    let err = CredentialError::InvalidInput("bad".into());
    assert_eq!(err.category(), nebula_error::ErrorCategory::Validation);
    assert!(!err.is_retryable());

    let err = CredentialError::refresh(
        nebula_credential::RefreshErrorKind::TransientNetwork,
        nebula_credential::RetryAdvice::Immediate,
        "connection reset",
    );
    assert!(err.is_retryable());

    let err = CryptoError::DecryptionFailed;
    assert_eq!(err.category(), nebula_error::ErrorCategory::Internal);
}
