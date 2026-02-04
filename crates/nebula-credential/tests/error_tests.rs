//! Error handling tests
//!
//! Tests for error messages, context propagation, and actionable error information.

use nebula_credential::core::{CredentialError, CryptoError, StorageError, ValidationError};
use std::io;

/// Test: StorageError::NotFound includes credential ID
///
/// Verifies that NotFound errors contain the credential ID for debugging.
#[test]
fn test_storage_error_not_found() {
    let err = StorageError::NotFound {
        id: "github_token".to_string(),
    };

    let msg = err.to_string();

    // Verify error message includes credential ID
    assert!(
        msg.contains("github_token"),
        "Error should include credential ID"
    );
    assert!(msg.contains("not found"), "Error should indicate not found");

    // Verify exact format
    assert_eq!(msg, "Credential 'github_token' not found");
}

/// Test: Storage error messages are actionable
///
/// Verifies that storage error messages provide enough context
/// for users to understand and fix the issue.
#[test]
fn test_storage_error_display() {
    // Test NotFound
    let not_found = StorageError::NotFound {
        id: "missing_cred".to_string(),
    };
    assert!(not_found.to_string().contains("missing_cred"));
    assert!(not_found.to_string().contains("not found"));

    // Test ReadFailure
    let io_err = io::Error::new(io::ErrorKind::PermissionDenied, "access denied");
    let read_fail = StorageError::ReadFailure {
        id: "protected_cred".to_string(),
        source: io_err,
    };
    let msg = read_fail.to_string();
    assert!(
        msg.contains("protected_cred"),
        "Should include credential ID"
    );
    assert!(
        msg.contains("Failed to read"),
        "Should indicate read failure"
    );
    assert!(
        msg.contains("access denied"),
        "Should include underlying error"
    );

    // Test WriteFailure
    let io_err = io::Error::new(io::ErrorKind::NotFound, "directory not found");
    let write_fail = StorageError::WriteFailure {
        id: "new_cred".to_string(),
        source: io_err,
    };
    let msg = write_fail.to_string();
    assert!(msg.contains("new_cred"));
    assert!(msg.contains("Failed to write"));
    assert!(msg.contains("directory not found"));

    // Test PermissionDenied
    let perm_denied = StorageError::PermissionDenied {
        id: "secure_cred".to_string(),
    };
    assert!(perm_denied.to_string().contains("secure_cred"));
    assert!(perm_denied.to_string().contains("Permission denied"));

    // Test Timeout
    let timeout = StorageError::Timeout {
        duration: std::time::Duration::from_secs(30),
    };
    assert!(timeout.to_string().contains("30s"));
    assert!(timeout.to_string().contains("timed out"));
}

/// Test: Crypto error messages don't leak secrets
///
/// Verifies that cryptographic errors never include sensitive data
/// like passwords, keys, or plaintext in error messages.
#[test]
fn test_crypto_error_display() {
    // DecryptionFailed - should not leak actual key values or plaintext
    let decrypt_err = CryptoError::DecryptionFailed;
    let msg = decrypt_err.to_string();
    // Should not leak actual key bytes/values (mentioning "key" generically is OK)
    assert!(!msg.contains("0x"), "Should not leak key bytes");
    assert!(!msg.contains("password"), "Should not leak password");
    assert!(!msg.contains("plaintext"), "Should not leak plaintext");
    assert!(!msg.contains("secret"), "Should not leak secret values");
    assert!(
        msg.contains("Decryption failed"),
        "Should indicate what failed"
    );
    assert!(
        msg.contains("invalid key") || msg.contains("corrupted data"),
        "Should suggest possible causes"
    );

    // EncryptionFailed - generic message only
    let encrypt_err = CryptoError::EncryptionFailed("cipher initialization failed".to_string());
    let msg = encrypt_err.to_string();
    assert!(msg.contains("Encryption failed"));
    assert!(msg.contains("cipher initialization"));
    assert!(!msg.contains("secret"), "Should not leak secrets");

    // KeyDerivation - should not leak password
    let kd_err = CryptoError::KeyDerivation("invalid parameters".to_string());
    let msg = kd_err.to_string();
    assert!(msg.contains("Key derivation failed"));
    assert!(msg.contains("invalid parameters"));
    assert!(!msg.contains("password"), "Should not leak password");

    // NonceGeneration
    let nonce_err = CryptoError::NonceGeneration;
    assert_eq!(nonce_err.to_string(), "Nonce generation failed");

    // UnsupportedVersion - safe to show version number
    let version_err = CryptoError::UnsupportedVersion(99);
    let msg = version_err.to_string();
    assert!(msg.contains("99"), "Can safely show version number");
    assert!(msg.contains("Unsupported"));
}

/// Test: Validation error messages include helpful reason
///
/// Verifies that validation errors explain why validation failed
/// and what the user should do to fix it.
#[test]
fn test_validation_error_display() {
    // EmptyCredentialId
    let empty_err = ValidationError::EmptyCredentialId;
    let msg = empty_err.to_string();
    assert!(msg.contains("empty"), "Should indicate empty ID");
    assert!(
        msg.contains("Credential ID"),
        "Should mention credential ID"
    );

    // InvalidCredentialId with reason
    let invalid_err = ValidationError::InvalidCredentialId {
        id: "../etc/passwd".to_string(),
        reason: "contains invalid characters (only alphanumeric, hyphens, underscores allowed)"
            .to_string(),
    };
    let msg = invalid_err.to_string();
    assert!(msg.contains("../etc/passwd"), "Should show the invalid ID");
    assert!(
        msg.contains("contains invalid characters"),
        "Should explain why"
    );
    assert!(
        msg.contains("only alphanumeric"),
        "Should explain what's allowed"
    );

    // InvalidFormat
    let format_err =
        ValidationError::InvalidFormat("missing required field 'credential_type'".to_string());
    let msg = format_err.to_string();
    assert!(msg.contains("Invalid credential format"));
    assert!(msg.contains("missing required field"));
}

/// Test: Error source chain works correctly
///
/// Verifies that error source chaining (via #[source] attribute)
/// allows traversing the full error context.
#[test]
fn test_error_source_chain() {
    use std::error::Error;

    // Create nested error: I/O -> Storage -> Credential
    let io_err = io::Error::new(io::ErrorKind::PermissionDenied, "access denied");
    let storage_err = StorageError::ReadFailure {
        id: "protected".to_string(),
        source: io_err,
    };
    let cred_err = CredentialError::Storage {
        id: "protected".to_string(),
        source: storage_err,
    };

    // Verify top-level error
    let msg = cred_err.to_string();
    assert!(
        msg.contains("protected"),
        "Top-level should have credential ID"
    );
    assert!(
        msg.contains("Storage error"),
        "Should indicate storage error"
    );

    // Verify source chain Level 1: StorageError
    let source1 = cred_err.source();
    assert!(source1.is_some(), "Should have source");
    let storage_source = source1.unwrap();
    assert!(storage_source.to_string().contains("Failed to read"));

    // Verify source chain Level 2: io::Error
    let source2 = storage_source.source();
    assert!(source2.is_some(), "Should have nested source");
    let io_source = source2.unwrap();
    assert!(io_source.to_string().contains("access denied"));

    // No further sources
    assert!(io_source.source().is_none(), "I/O error is the root cause");
}

/// Test: Crypto error conversion doesn't leak context
///
/// Verifies that when converting crypto errors to credential errors,
/// no additional secret context is added.
#[test]
fn test_crypto_error_conversion() {
    let crypto_err = CryptoError::DecryptionFailed;
    let cred_err: CredentialError = crypto_err.into();

    let msg = cred_err.to_string();
    assert!(msg.contains("Cryptographic error"));
    assert!(msg.contains("Decryption failed"));
    // Should not leak actual key/password values (generic mentions are OK)
    assert!(!msg.contains("0x"), "Should not add key bytes");
    assert!(!msg.contains("password:"), "Should not add password values");
    assert!(!msg.contains("secret:"), "Should not add secret values");
}

/// Test: Storage error includes ID in all variants
///
/// Verifies that all storage error variants that involve a credential
/// include the credential ID for debugging.
#[test]
fn test_storage_error_always_includes_id() {
    // NotFound
    let err1 = StorageError::NotFound {
        id: "id1".to_string(),
    };
    assert!(err1.to_string().contains("id1"));

    // ReadFailure
    let err2 = StorageError::ReadFailure {
        id: "id2".to_string(),
        source: io::Error::new(io::ErrorKind::NotFound, "test"),
    };
    assert!(err2.to_string().contains("id2"));

    // WriteFailure
    let err3 = StorageError::WriteFailure {
        id: "id3".to_string(),
        source: io::Error::new(io::ErrorKind::PermissionDenied, "test"),
    };
    assert!(err3.to_string().contains("id3"));

    // PermissionDenied
    let err4 = StorageError::PermissionDenied {
        id: "id4".to_string(),
    };
    assert!(err4.to_string().contains("id4"));

    // Timeout doesn't have credential ID (operation-level error)
    let err5 = StorageError::Timeout {
        duration: std::time::Duration::from_secs(5),
    };
    // This is OK - timeout is a general operation error
    assert!(!err5.to_string().contains("credential"));
}

/// Test: Validation errors are user-friendly
///
/// Verifies that validation errors use clear, non-technical language
/// that helps users understand what went wrong.
#[test]
fn test_validation_errors_user_friendly() {
    // Empty ID
    let err1 = ValidationError::EmptyCredentialId;
    let msg1 = err1.to_string();
    assert!(!msg1.contains("null"), "Avoid technical jargon");
    assert!(!msg1.contains("undefined"), "Avoid technical jargon");
    assert!(msg1.contains("empty"), "Use simple language");

    // Invalid ID with helpful reason
    let err2 = ValidationError::InvalidCredentialId {
        id: "bad/id".to_string(),
        reason: "contains invalid characters (only alphanumeric, hyphens, underscores allowed)"
            .to_string(),
    };
    let msg2 = err2.to_string();
    assert!(msg2.contains("bad/id"), "Show what was invalid");
    assert!(msg2.contains("only alphanumeric"), "Explain what's allowed");
    assert!(!msg2.contains("regex"), "Avoid implementation details");
    assert!(!msg2.contains("pattern"), "Avoid implementation details");
}

/// Test: Error Display implementation is stable
///
/// Verifies that error messages have stable formats that can be
/// relied upon for logging and monitoring.
#[test]
fn test_error_display_format_stable() {
    // StorageError::NotFound format
    let err1 = StorageError::NotFound {
        id: "test".to_string(),
    };
    assert_eq!(err1.to_string(), "Credential 'test' not found");

    // CryptoError::DecryptionFailed format
    let err2 = CryptoError::DecryptionFailed;
    assert_eq!(
        err2.to_string(),
        "Decryption failed - invalid key or corrupted data"
    );

    // ValidationError::EmptyCredentialId format
    let err3 = ValidationError::EmptyCredentialId;
    assert_eq!(err3.to_string(), "Credential ID cannot be empty");
}
