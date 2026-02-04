// Storage Provider Trait Contract
// This file documents the StorageProvider trait interface that all provider implementations must follow.
// This is a CONTRACT, not implementation code.

use async_trait::async_trait;
use crate::core::{CredentialContext, CredentialFilter, CredentialId, CredentialMetadata, StorageError};
use crate::utils::EncryptedData;

/// Storage provider trait for credential persistence
///
/// This trait defines the contract that all storage backends MUST implement.
/// Implementations target different backends (local filesystem, AWS, Azure, Vault, K8s)
/// without changing application code.
///
/// # Thread Safety
/// All implementations MUST be `Send + Sync` to allow sharing across threads.
///
/// # Error Handling
/// All methods return `Result<T, StorageError>`. Provider-specific errors MUST be
/// mapped to `StorageError` variants with actionable context.
#[async_trait]
pub trait StorageProvider: Send + Sync {
    /// Store encrypted credential with metadata
    ///
    /// # Contract Requirements
    /// - MUST accept encrypted data (ciphertext + nonce + tag)
    /// - MUST store metadata separately from encrypted data
    /// - MUST support idempotent writes (overwrite if ID exists)
    /// - MUST validate credential size against provider limits
    /// - MUST convert metadata tags to provider-specific format
    /// - MUST return error if size exceeds provider limit
    ///
    /// # Provider-Specific Behavior
    /// - **Local**: Write to file with atomic rename, 0600 permissions
    /// - **AWS**: CreateSecret or UpdateSecret with KMS encryption
    /// - **Azure**: SetSecret with tags, automatic token refresh
    /// - **Vault**: kv2::set with versioning (creates version N+1)
    /// - **K8s**: Create or Update Secret in namespace
    ///
    /// # Errors
    /// - `StorageError::CredentialTooLarge` - Exceeds provider size limit
    /// - `StorageError::WriteFailure` - I/O error during write
    /// - `StorageError::PermissionDenied` - Insufficient permissions
    /// - `StorageError::Timeout` - Operation exceeded time limit
    async fn store(
        &self,
        id: &CredentialId,
        data: EncryptedData,
        metadata: CredentialMetadata,
        context: &CredentialContext,
    ) -> Result<(), StorageError>;

    /// Retrieve encrypted credential by ID
    ///
    /// # Contract Requirements
    /// - MUST return both encrypted data AND metadata
    /// - MUST decrypt using provider's encryption mechanism (if applicable)
    /// - MUST return NotFound if credential doesn't exist
    /// - MUST not expose decrypted credential values in errors/logs
    ///
    /// # Provider-Specific Behavior
    /// - **Local**: Read file, deserialize JSON, return CredentialFile contents
    /// - **AWS**: GetSecretValue, deserialize SecretString as JSON
    /// - **Azure**: GetSecret, deserialize value as JSON
    /// - **Vault**: kv2::read (latest version), deserialize data as JSON
    /// - **K8s**: Get Secret, base64 decode data field, deserialize as JSON
    ///
    /// # Errors
    /// - `StorageError::NotFound` - Credential does not exist
    /// - `StorageError::ReadFailure` - I/O error during read
    /// - `StorageError::PermissionDenied` - Insufficient permissions
    /// - `StorageError::Timeout` - Operation exceeded time limit
    /// - `StorageError::DecryptionFailed` - Provider-side decryption failed
    async fn retrieve(
        &self,
        id: &CredentialId,
        context: &CredentialContext,
    ) -> Result<(EncryptedData, CredentialMetadata), StorageError>;

    /// Delete credential by ID
    ///
    /// # Contract Requirements
    /// - MUST be idempotent (deleting non-existent credential succeeds)
    /// - SHOULD support soft-delete if provider supports it
    /// - MUST return Ok(()) even if credential doesn't exist
    ///
    /// # Provider-Specific Behavior
    /// - **Local**: Delete file (std::fs::remove_file, ignore NotFound)
    /// - **AWS**: DeleteSecret with recovery window (7-30 days configurable)
    /// - **Azure**: DeleteSecret (soft-delete, 7-90 day retention)
    /// - **Vault**: kv2::delete_metadata (permanent) or delete_latest (soft)
    /// - **K8s**: Delete Secret (permanent, no soft-delete)
    ///
    /// # Errors
    /// - `StorageError::WriteFailure` - I/O error during delete
    /// - `StorageError::PermissionDenied` - Insufficient permissions
    async fn delete(
        &self,
        id: &CredentialId,
        context: &CredentialContext,
    ) -> Result<(), StorageError>;

    /// List all credential IDs (optionally filtered)
    ///
    /// # Contract Requirements
    /// - MUST return IDs only, NOT credential values
    /// - MUST support filtering by metadata tags (if filter provided)
    /// - MUST support filtering by date ranges (if filter provided)
    /// - SHOULD paginate for large result sets (provider-dependent)
    ///
    /// # Provider-Specific Behavior
    /// - **Local**: Scan directory, parse filenames, filter by metadata
    /// - **AWS**: ListSecrets with tag filters, paginate with NextToken
    /// - **Azure**: ListSecrets, filter client-side by tags
    /// - **Vault**: kv2::list with path prefix, return keys only
    /// - **K8s**: List Secrets with label selector, filter by prefix
    ///
    /// # Errors
    /// - `StorageError::ReadFailure` - I/O error during list
    /// - `StorageError::PermissionDenied` - Insufficient permissions
    async fn list(
        &self,
        filter: Option<&CredentialFilter>,
        context: &CredentialContext,
    ) -> Result<Vec<CredentialId>, StorageError>;

    /// Check if credential exists
    ///
    /// # Contract Requirements
    /// - MUST return true if credential exists, false otherwise
    /// - SHOULD be lightweight (metadata-only check, no data retrieval)
    /// - MUST NOT throw error for non-existent credentials
    ///
    /// # Provider-Specific Behavior
    /// - **Local**: Check file existence (std::fs::metadata)
    /// - **AWS**: DescribeSecret (lighter than GetSecretValue)
    /// - **Azure**: GetSecret with metadata-only request
    /// - **Vault**: kv2::read metadata endpoint
    /// - **K8s**: Get Secret metadata (not data field)
    ///
    /// # Errors
    /// - `StorageError::ReadFailure` - I/O error during check
    /// - `StorageError::PermissionDenied` - Insufficient permissions
    async fn exists(
        &self,
        id: &CredentialId,
        context: &CredentialContext,
    ) -> Result<bool, StorageError>;
}

// ============================================================================
// Provider Implementation Requirements
// ============================================================================

/// All provider implementations MUST satisfy these requirements:
///
/// 1. **Concurrency Safety**
///    - MUST be thread-safe (Send + Sync)
///    - MUST handle concurrent operations on same credential ID
///    - Local: Use file locking (fs2::FileExt)
///    - Cloud: Rely on provider's native concurrency controls
///
/// 2. **Error Handling**
///    - MUST map provider-specific errors to StorageError
///    - MUST include actionable context (required permissions, fix suggestions)
///    - MUST log errors with context before returning
///    - MUST NOT expose sensitive data (credentials, tokens) in errors
///
/// 3. **Retry Logic**
///    - MUST implement exponential backoff for transient errors
///    - MUST respect RetryPolicy configuration
///    - MUST add jitter to prevent thundering herd
///    - MUST NOT retry non-retryable errors (4xx client errors)
///
/// 4. **Metrics**
///    - MUST record operation latency in StorageMetrics
///    - MUST record success/failure counts
///    - MUST record retry attempts
///    - MUST use Instant::now() for accurate timing
///
/// 5. **Timeouts**
///    - MUST apply timeout to all async operations
///    - Default: 5s for reads, 10s for writes
///    - MUST return StorageError::Timeout on expiration
///
/// 6. **Size Validation**
///    - MUST validate credential size before submission
///    - MUST return CredentialTooLarge with size and limit
///    - Limits:
///      * AWS: 64KB
///      * Azure: 25KB
///      * Vault: Configurable (default 1MB)
///      * K8s: 1MB
///      * Local: No hard limit (filesystem dependent)
///
/// 7. **Metadata Conversion**
///    - MUST convert CredentialMetadata to provider tags/labels
///    - AWS: Convert to Tags (max 50, key 1-128 chars, value 0-256 chars)
///    - Azure: Convert to Tags (max 15)
///    - Vault: Store in KV v2 metadata
///    - K8s: Convert to Labels (max 63 chars) and Annotations (max 256KB)
///
/// 8. **Logging and Observability**
///    - MUST use tracing::info! for successful operations
///    - MUST use tracing::error! for failures
///    - MUST include: credential_id, provider_name, operation, duration
///    - MUST redact credential values (use "[REDACTED]" in logs)
///
/// 9. **Testing**
///    - MUST have unit tests with MockStorageProvider
///    - SHOULD have integration tests with real provider (testcontainers)
///    - MUST test error conditions (NotFound, PermissionDenied, Timeout)
///    - MUST test retry logic with transient failures
///
/// 10. **Documentation**
///     - MUST document provider-specific configuration requirements
///     - MUST document IAM permissions / RBAC roles needed
///     - MUST provide usage examples in rustdoc
///     - MUST document error scenarios and remediation steps

// ============================================================================
// Example Usage Pattern
// ============================================================================

/// ```no_run
/// use nebula_credential::prelude::*;
///
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {
///     // Initialize provider (example: local storage)
///     let config = LocalStorageConfig {
///         base_path: PathBuf::from("/home/user/.nebula/credentials"),
///         create_dir: true,
///         ..Default::default()
///     };
///
///     let provider = LocalStorageProvider::new(config).await?;
///
///     // Store credential
///     let id = CredentialId::new("github_token")?;
///     let data = encrypt_credential(b"ghp_secret_token")?;
///     let metadata = CredentialMetadata {
///         created_at: Utc::now(),
///         tags: vec!["environment:production".into()],
///         ..Default::default()
///     };
///     let context = CredentialContext::new("user_123");
///
///     provider.store(&id, data.clone(), metadata.clone(), &context).await?;
///
///     // Retrieve credential
///     let (retrieved_data, retrieved_metadata) = provider.retrieve(&id, &context).await?;
///
///     // List credentials
///     let filter = CredentialFilter {
///         tags: Some(vec!["environment:production".into()]),
///         ..Default::default()
///     };
///     let ids = provider.list(Some(&filter), &context).await?;
///
///     // Check existence
///     let exists = provider.exists(&id, &context).await?;
///
///     // Delete credential
///     provider.delete(&id, &context).await?;
///
///     Ok(())
/// }
/// ```
