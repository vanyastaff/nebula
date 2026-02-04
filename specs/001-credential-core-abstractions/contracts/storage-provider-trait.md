# API Contract: StorageProvider Trait

**Feature**: 001-credential-core-abstractions  
**Date**: 2026-02-03  
**Phase**: 1 (Architecture & Contracts)

## Overview

The `StorageProvider` trait defines the contract for credential persistence backends. This trait abstracts storage operations (store, retrieve, delete, list) so implementations can target different backends (local filesystem, AWS, Azure, Vault, K8s) without changing application code.

## Trait Definition

```rust
use async_trait::async_trait;
use crate::{
    CredentialId, EncryptedData, CredentialMetadata, CredentialContext,
    CredentialFilter, StorageError,
};

/// Storage provider trait for credential persistence
#[async_trait]
pub trait StorageProvider: Send + Sync {
    /// Store encrypted credential with metadata
    ///
    /// # Arguments
    /// * `id` - Unique credential identifier
    /// * `data` - Encrypted credential data (ciphertext + nonce + tag)
    /// * `metadata` - Non-sensitive metadata (timestamps, tags)
    /// * `context` - Request context (owner, scope, trace)
    ///
    /// # Returns
    /// * `Ok(())` - Credential stored successfully
    /// * `Err(StorageError)` - Storage operation failed
    ///
    /// # Errors
    /// * `StorageError::WriteFailure` - I/O error during write
    /// * `StorageError::PermissionDenied` - Insufficient permissions
    /// * `StorageError::Timeout` - Operation exceeded time limit
    async fn store(
        &self,
        id: &CredentialId,
        data: EncryptedData,
        metadata: CredentialMetadata,
        context: &CredentialContext,
    ) -> Result<(), StorageError>;
    
    /// Retrieve encrypted credential by ID
    ///
    /// # Arguments
    /// * `id` - Unique credential identifier
    /// * `context` - Request context (owner, scope, trace)
    ///
    /// # Returns
    /// * `Ok((data, metadata))` - Encrypted data and metadata
    /// * `Err(StorageError)` - Retrieval failed
    ///
    /// # Errors
    /// * `StorageError::NotFound` - Credential does not exist
    /// * `StorageError::ReadFailure` - I/O error during read
    /// * `StorageError::PermissionDenied` - Insufficient permissions
    /// * `StorageError::Timeout` - Operation exceeded time limit
    async fn retrieve(
        &self,
        id: &CredentialId,
        context: &CredentialContext,
    ) -> Result<(EncryptedData, CredentialMetadata), StorageError>;
    
    /// Delete credential by ID
    ///
    /// # Arguments
    /// * `id` - Unique credential identifier
    /// * `context` - Request context (owner, scope, trace)
    ///
    /// # Returns
    /// * `Ok(())` - Credential deleted successfully
    /// * `Err(StorageError)` - Deletion failed
    ///
    /// # Errors
    /// * `StorageError::NotFound` - Credential does not exist (idempotent - not an error)
    /// * `StorageError::WriteFailure` - I/O error during delete
    /// * `StorageError::PermissionDenied` - Insufficient permissions
    async fn delete(
        &self,
        id: &CredentialId,
        context: &CredentialContext,
    ) -> Result<(), StorageError>;
    
    /// List all credential IDs (optionally filtered)
    ///
    /// # Arguments
    /// * `filter` - Optional filter criteria (tags, date ranges)
    /// * `context` - Request context (owner, scope, trace)
    ///
    /// # Returns
    /// * `Ok(Vec<CredentialId>)` - List of credential IDs matching filter
    /// * `Err(StorageError)` - List operation failed
    ///
    /// # Errors
    /// * `StorageError::ReadFailure` - I/O error during list
    /// * `StorageError::PermissionDenied` - Insufficient permissions
    async fn list(
        &self,
        filter: Option<&CredentialFilter>,
        context: &CredentialContext,
    ) -> Result<Vec<CredentialId>, StorageError>;
    
    /// Check if credential exists
    ///
    /// # Arguments
    /// * `id` - Unique credential identifier
    /// * `context` - Request context (owner, scope, trace)
    ///
    /// # Returns
    /// * `Ok(true)` - Credential exists
    /// * `Ok(false)` - Credential does not exist
    /// * `Err(StorageError)` - Check operation failed
    ///
    /// # Errors
    /// * `StorageError::ReadFailure` - I/O error during check
    /// * `StorageError::PermissionDenied` - Insufficient permissions
    async fn exists(
        &self,
        id: &CredentialId,
        context: &CredentialContext,
    ) -> Result<bool, StorageError>;
}
```

## Operation Semantics

### store()

**Preconditions**:
- `id` is a valid CredentialId (alphanumeric + hyphens + underscores)
- `data` contains valid EncryptedData with supported version
- `context` contains valid owner_id

**Postconditions**:
- Credential is persisted to storage backend
- Subsequent `retrieve(id)` returns the same encrypted data
- Subsequent `exists(id)` returns `true`

**Idempotency**: Not idempotent - overwrites existing credential if ID matches

**Atomicity**: Must be atomic - either fully succeeds or fully fails (no partial writes)

### retrieve()

**Preconditions**:
- `id` is a valid CredentialId
- `context` contains valid owner_id

**Postconditions**:
- Returns encrypted data exactly as stored via `store()`
- Returns metadata with correct timestamps
- Does NOT modify last_accessed timestamp (read-only operation for Phase 1)

**Idempotency**: Idempotent - multiple calls return same result

**Cache behavior**: Phase 1 has no caching - always reads from storage

### delete()

**Preconditions**:
- `id` is a valid CredentialId
- `context` contains valid owner_id

**Postconditions**:
- Credential no longer exists in storage
- Subsequent `retrieve(id)` returns `StorageError::NotFound`
- Subsequent `exists(id)` returns `false`

**Idempotency**: Idempotent - deleting non-existent credential succeeds (returns `Ok(())`)

**Security**: Credential data should be securely erased (Phase 1: file deletion, Phase 2: cloud provider deletion APIs)

### list()

**Preconditions**:
- `context` contains valid owner_id
- `filter` (if provided) contains valid filter criteria

**Postconditions**:
- Returns all credential IDs matching filter
- Does NOT return encrypted data or secrets
- Order is implementation-defined (Phase 1: alphabetical)

**Idempotency**: Idempotent - multiple calls return same result (assuming no concurrent modifications)

**Performance**: Phase 1 reads all metadata files - O(n) where n = credential count

### exists()

**Preconditions**:
- `id` is a valid CredentialId
- `context` contains valid owner_id

**Postconditions**:
- Returns `true` if credential exists, `false` otherwise
- Does NOT read or decrypt credential data

**Idempotency**: Idempotent - multiple calls return same result

**Performance**: Fast check without full read (Phase 1: file existence check)

## Error Handling

### Error Propagation

All methods return `Result<T, StorageError>`. Implementations MUST:
1. Convert backend-specific errors to StorageError variants
2. Include credential ID in error context
3. Preserve underlying error cause with `#[source]` attribute
4. Redact secrets from error messages

### Error Recovery

| Error | Retry? | Action |
|-------|--------|--------|
| NotFound | No | Return error to caller |
| ReadFailure (I/O) | Maybe | Retry with backoff (Phase 2) |
| WriteFailure (I/O) | Maybe | Retry with backoff (Phase 2) |
| PermissionDenied | No | Return error to caller |
| Timeout | Maybe | Retry with backoff (Phase 2) |

Phase 1 does NOT implement automatic retry - that's Phase 2 for cloud providers.

## Thread Safety

All `StorageProvider` implementations MUST be `Send + Sync`:
- **Send**: Can be transferred between threads
- **Sync**: Can be shared between threads via `Arc<dyn StorageProvider>`

Phase 1 LocalStorageProvider uses:
- File locks for atomic writes (prevents corruption)
- No shared mutable state (each operation is independent)

## Performance Expectations

### Phase 1 (Local Storage)
- **store()**: <10ms p95 (write + fsync)
- **retrieve()**: <5ms p95 (read from disk)
- **delete()**: <5ms p95 (file deletion)
- **list()**: O(n) where n = credential count
- **exists()**: <1ms p95 (file stat)

### Phase 2 (Cloud Providers)
- AWS/Azure/Vault: 50-500ms p95 (network + API latency)
- Automatic retry with exponential backoff
- Caching layer (Phase 3) reduces latency to <1ms for cached reads

## Testing Contract

All `StorageProvider` implementations MUST pass the following test suite:

```rust
#[tokio::test]
async fn test_store_and_retrieve() {
    // Store credential → retrieve → verify data matches
}

#[tokio::test]
async fn test_retrieve_nonexistent() {
    // Retrieve non-existent ID → verify NotFound error
}

#[tokio::test]
async fn test_delete_idempotent() {
    // Delete credential twice → both succeed
}

#[tokio::test]
async fn test_list_empty() {
    // List with no credentials → verify empty vec
}

#[tokio::test]
async fn test_list_filtered() {
    // Store multiple → list with filter → verify matches only
}

#[tokio::test]
async fn test_exists() {
    // exists() before store → false, after store → true, after delete → false
}

#[tokio::test]
async fn test_concurrent_writes() {
    // Concurrent store() to same ID → last write wins, no corruption
}
```

## Implementation Checklist

When implementing `StorageProvider`:

- [ ] Implement all 5 trait methods (store, retrieve, delete, list, exists)
- [ ] Convert backend errors to StorageError with context
- [ ] Ensure atomic operations (no partial writes)
- [ ] Add tracing spans for all operations
- [ ] Include credential_id in all log events
- [ ] Redact secrets from error messages and logs
- [ ] Pass all contract tests
- [ ] Document provider-specific configuration
- [ ] Handle provider-specific size limits
- [ ] Implement proper resource cleanup in Drop

## Examples

### Using StorageProvider

```rust
use nebula_credential::{
    StorageProvider, CredentialId, EncryptedData,
    CredentialMetadata, CredentialContext,
};

async fn example(provider: &dyn StorageProvider) -> Result<(), StorageError> {
    let id = CredentialId::new("github_token")?;
    let context = CredentialContext::new("user_123");
    
    // Store credential
    provider.store(&id, encrypted_data, metadata, &context).await?;
    
    // Retrieve credential
    let (data, metadata) = provider.retrieve(&id, &context).await?;
    
    // Check existence
    let exists = provider.exists(&id, &context).await?;
    assert!(exists);
    
    // List all credentials
    let ids = provider.list(None, &context).await?;
    
    // Delete credential
    provider.delete(&id, &context).await?;
    
    Ok(())
}
```

### Mock Provider for Testing

```rust
use std::collections::HashMap;
use tokio::sync::RwLock;

pub struct MockStorageProvider {
    data: RwLock<HashMap<CredentialId, (EncryptedData, CredentialMetadata)>>,
}

#[async_trait]
impl StorageProvider for MockStorageProvider {
    async fn store(/* ... */) -> Result<(), StorageError> {
        let mut data = self.data.write().await;
        data.insert(id.clone(), (data, metadata));
        Ok(())
    }
    
    // ... implement other methods
}
```

## References

- Feature Specification: [../spec.md](../spec.md)
- Data Model: [../data-model.md](../data-model.md)
- Implementation Plan: [../plan.md](../plan.md)
- Constitution: `.specify/memory/constitution.md` (Principle IV: Async Discipline)
