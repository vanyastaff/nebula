//! Validation utilities for credential operations
//!
//! Provides reusable validation functions for common checks across providers.

use crate::core::{CredentialId, StorageError};
use crate::utils::EncryptedData;

/// Validate encrypted data size against provider limits
///
/// # Arguments
///
/// * `id` - Credential ID for error reporting
/// * `data` - Encrypted data to validate
/// * `max_size` - Maximum allowed size in bytes
/// * `provider_name` - Provider name for error message
///
/// # Returns
///
/// * `Ok(())` - Size is within limit
/// * `Err(StorageError::WriteFailure)` - Size exceeds limit
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_credential::utils::validate_encrypted_size;
///
/// // AWS Secrets Manager has 64KB limit
/// validate_encrypted_size(&id, &data, 64 * 1024, "AWS Secrets Manager")?;
///
/// // Kubernetes Secrets has 1MB limit
/// validate_encrypted_size(&id, &data, 1_000_000, "Kubernetes")?;
/// ```
pub fn validate_encrypted_size(
    id: &CredentialId,
    data: &EncryptedData,
    max_size: usize,
    provider_name: &str,
) -> Result<(), StorageError> {
    // Calculate total size: ciphertext + nonce + authentication tag
    let size = data.ciphertext.len() + data.nonce.len() + data.tag.len();

    if size > max_size {
        return Err(StorageError::WriteFailure {
            id: id.as_str().to_string(),
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "Payload size {} bytes exceeds {} limit of {} bytes",
                    size, provider_name, max_size
                ),
            ),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_credential_id() -> CredentialId {
        CredentialId::new("test_cred").unwrap()
    }

    fn test_encrypted_data(size: usize) -> EncryptedData {
        EncryptedData::new([0u8; 12], vec![0u8; size], [0u8; 16])
    }

    #[test]
    fn test_validate_encrypted_size_within_limit() {
        let id = test_credential_id();
        let data = test_encrypted_data(100); // 100 + 12 + 16 = 128 bytes

        assert!(validate_encrypted_size(&id, &data, 200, "Test Provider").is_ok());
        assert!(validate_encrypted_size(&id, &data, 128, "Test Provider").is_ok()); // Exact match
    }

    #[test]
    fn test_validate_encrypted_size_exceeds_limit() {
        let id = test_credential_id();
        let data = test_encrypted_data(100); // 100 + 12 + 16 = 128 bytes

        let result = validate_encrypted_size(&id, &data, 100, "Test Provider");
        assert!(result.is_err());

        if let Err(StorageError::WriteFailure { id: err_id, source }) = result {
            assert_eq!(err_id, "test_cred");
            assert!(source.to_string().contains("128 bytes"));
            assert!(source.to_string().contains("100 bytes"));
            assert!(source.to_string().contains("Test Provider"));
        } else {
            panic!("Expected WriteFailure error");
        }
    }

    #[test]
    fn test_validate_encrypted_size_aws_limit() {
        let id = test_credential_id();

        // Within AWS limit (64KB)
        let small_data = test_encrypted_data(60 * 1024);
        assert!(validate_encrypted_size(&id, &small_data, 64 * 1024, "AWS").is_ok());

        // Exceeds AWS limit
        let large_data = test_encrypted_data(64 * 1024);
        assert!(validate_encrypted_size(&id, &large_data, 64 * 1024, "AWS").is_err());
    }

    #[test]
    fn test_validate_encrypted_size_kubernetes_limit() {
        let id = test_credential_id();

        // Within Kubernetes limit (1MB)
        let small_data = test_encrypted_data(900 * 1024);
        assert!(validate_encrypted_size(&id, &small_data, 1_000_000, "Kubernetes").is_ok());

        // Exceeds Kubernetes limit
        let large_data = test_encrypted_data(1_000_000);
        assert!(validate_encrypted_size(&id, &large_data, 1_000_000, "Kubernetes").is_err());
    }

    #[test]
    fn test_error_message_contains_details() {
        let id = test_credential_id();
        let data = test_encrypted_data(1000);

        let result = validate_encrypted_size(&id, &data, 500, "Custom Provider");
        assert!(result.is_err());

        let err = result.unwrap_err();
        let msg = err.to_string();

        assert!(msg.contains("test_cred"));
        assert!(msg.contains("Custom Provider"));
    }
}
