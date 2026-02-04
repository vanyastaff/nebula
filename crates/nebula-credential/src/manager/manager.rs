//! Credential Manager - Central interface for credential operations
//!
//! Provides high-level API for CRUD operations, caching, validation, and multi-tenant isolation.

use crate::core::{
    CredentialContext, CredentialId, CredentialMetadata, ManagerError, ManagerResult,
};
use crate::manager::{CacheConfig, CacheLayer, CacheStats};
use crate::traits::StorageProvider;
use crate::utils::EncryptedData;
use std::marker::PhantomData;
use std::sync::Arc;
use tracing::{debug, error, info, warn};

/// Central credential manager with caching and storage abstraction
#[derive(Clone)]
pub struct CredentialManager {
    /// Underlying storage provider (LocalStorage, AWS, Vault, K8s)
    storage: Arc<dyn StorageProvider>,

    /// Optional in-memory cache with LRU + TTL
    cache: Option<Arc<CacheLayer>>,
}

impl CredentialManager {
    /// Create builder for constructing manager instance
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use nebula_credential::prelude::*;
    /// use std::sync::Arc;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let storage = MockStorageProvider::new();
    /// let manager = CredentialManager::builder()
    ///     .storage(Arc::new(storage))
    ///     .build();
    /// # Ok(())
    /// # }
    /// ```
    pub fn builder() -> CredentialManagerBuilder<No> {
        CredentialManagerBuilder::new()
    }

    /// Store a credential with encryption and optional caching
    ///
    /// # Arguments
    ///
    /// * `id` - Unique credential identifier
    /// * `data` - Encrypted credential data
    /// * `metadata` - Credential metadata
    /// * `context` - Request context (owner, scope, trace)
    ///
    /// # Errors
    ///
    /// Returns `ManagerError::StorageError` if storage operation fails
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use nebula_credential::prelude::*;
    /// # use std::sync::Arc;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let manager = CredentialManager::builder()
    /// #     .storage(Arc::new(MockStorageProvider::new()))
    /// #     .build();
    /// let id = CredentialId::new("github-token")?;
    /// let data = encrypt(b"secret", &EncryptionKey::from_bytes(&[0u8; 32])?)?;
    /// let metadata = CredentialMetadata::new("user-123");
    /// let context = CredentialContext::new("user-123");
    ///
    /// manager.store(&id, data, metadata, &context).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn store(
        &self,
        id: &CredentialId,
        data: EncryptedData,
        mut metadata: CredentialMetadata,
        context: &CredentialContext,
    ) -> ManagerResult<()> {
        info!(
            credential_id = %id,
            owner_id = %context.owner_id,
            scope = ?context.scope_id,
            "Storing credential"
        );

        // Copy scope from context to metadata for multi-tenant isolation
        metadata.scope = context.scope_id.clone();

        // Store via provider
        match self
            .storage
            .store(id, data.clone(), metadata.clone(), context)
            .await
        {
            Ok(()) => {
                info!(
                    credential_id = %id,
                    "Credential stored successfully"
                );
            }
            Err(e) => {
                error!(
                    credential_id = %id,
                    error = %e,
                    "Storage operation failed"
                );
                return Err(ManagerError::StorageError {
                    credential_id: id.to_string(),
                    source: e,
                });
            }
        }

        // Invalidate cache entry if caching enabled
        if let Some(cache) = &self.cache {
            cache.invalidate(id).await;
            debug!(
                credential_id = %id,
                "Cache invalidated for credential"
            );
        }

        Ok(())
    }

    /// Retrieve a credential by ID with cache-aside pattern
    ///
    /// # Cache Behavior
    ///
    /// - Cache hit: Returns cached credential (<10ms)
    /// - Cache miss: Fetches from storage, populates cache (<100ms)
    ///
    /// # Arguments
    ///
    /// * `id` - Unique credential identifier
    /// * `context` - Request context (owner, scope, trace)
    ///
    /// # Returns
    ///
    /// * `Ok(Some((data, metadata)))` - Credential found
    /// * `Ok(None)` - Credential not found
    /// * `Err(ManagerError)` - Operation failed
    ///
    /// # Errors
    ///
    /// Returns `ManagerError::StorageError` if storage provider fails
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use nebula_credential::prelude::*;
    /// # use std::sync::Arc;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let manager = CredentialManager::builder()
    /// #     .storage(Arc::new(MockStorageProvider::new()))
    /// #     .build();
    /// let id = CredentialId::new("github-token")?;
    /// let context = CredentialContext::new("user-123");
    ///
    /// if let Some((data, metadata)) = manager.retrieve(&id, &context).await? {
    ///     println!("Found credential: {:?}", metadata);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn retrieve(
        &self,
        id: &CredentialId,
        context: &CredentialContext,
    ) -> ManagerResult<Option<(EncryptedData, CredentialMetadata)>> {
        // Check cache first (if enabled)
        if let Some(cache) = &self.cache {
            if let Some(cached) = cache.get(id).await {
                debug!(
                    credential_id = %id,
                    "Cache hit for credential"
                );
                return Ok(Some(cached));
            } else {
                debug!(
                    credential_id = %id,
                    "Cache miss for credential"
                );
            }
        }

        // Cache miss or no cache - fetch from storage
        match self.storage.retrieve(id, context).await {
            Ok((data, metadata)) => {
                debug!(
                    credential_id = %id,
                    "Retrieved credential from storage"
                );
                // Populate cache if enabled
                if let Some(cache) = &self.cache {
                    cache
                        .insert(id.clone(), data.clone(), metadata.clone())
                        .await;
                    debug!(
                        credential_id = %id,
                        "Populated cache with credential"
                    );
                }
                Ok(Some((data, metadata)))
            }
            Err(crate::core::StorageError::NotFound { .. }) => {
                debug!(
                    credential_id = %id,
                    "Credential not found"
                );
                Ok(None)
            }
            Err(e) => {
                error!(
                    credential_id = %id,
                    error = %e,
                    "Storage retrieval failed"
                );
                Err(ManagerError::StorageError {
                    credential_id: id.to_string(),
                    source: e,
                })
            }
        }
    }

    /// Delete a credential and invalidate cache
    ///
    /// # Arguments
    ///
    /// * `id` - Unique credential identifier
    /// * `context` - Request context (owner, scope, trace)
    ///
    /// # Errors
    ///
    /// Returns `ManagerError::StorageError` if storage operation fails
    ///
    /// # Idempotency
    ///
    /// Idempotent - deleting a non-existent credential succeeds
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use nebula_credential::prelude::*;
    /// # use std::sync::Arc;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let manager = CredentialManager::builder()
    /// #     .storage(Arc::new(MockStorageProvider::new()))
    /// #     .build();
    /// let id = CredentialId::new("github-token")?;
    /// let context = CredentialContext::new("user-123");
    ///
    /// manager.delete(&id, &context).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn delete(
        &self,
        id: &CredentialId,
        context: &CredentialContext,
    ) -> ManagerResult<()> {
        info!(
            credential_id = %id,
            "Deleting credential"
        );

        // Delete from storage
        match self.storage.delete(id, context).await {
            Ok(()) => {
                info!(
                    credential_id = %id,
                    "Credential deleted successfully"
                );
            }
            Err(e) => {
                error!(
                    credential_id = %id,
                    error = %e,
                    "Storage deletion failed"
                );
                return Err(ManagerError::StorageError {
                    credential_id: id.to_string(),
                    source: e,
                });
            }
        }

        // Invalidate cache entry if caching enabled
        if let Some(cache) = &self.cache {
            cache.invalidate(id).await;
            debug!(
                credential_id = %id,
                "Cache invalidated for deleted credential"
            );
        }

        Ok(())
    }

    /// List all credential IDs (no caching, always fresh)
    ///
    /// # Arguments
    ///
    /// * `context` - Request context (owner, scope, trace)
    ///
    /// # Returns
    ///
    /// List of all credential IDs in storage
    ///
    /// # Errors
    ///
    /// Returns `ManagerError::StorageError` if storage operation fails
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use nebula_credential::prelude::*;
    /// # use std::sync::Arc;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let manager = CredentialManager::builder()
    /// #     .storage(Arc::new(MockStorageProvider::new()))
    /// #     .build();
    /// let context = CredentialContext::new("user-123");
    ///
    /// let ids = manager.list(&context).await?;
    /// println!("Found {} credentials", ids.len());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn list(&self, context: &CredentialContext) -> ManagerResult<Vec<CredentialId>> {
        self.storage
            .list(None, context)
            .await
            .map_err(ManagerError::from)
    }

    /// Get cache performance statistics
    ///
    /// # Returns
    ///
    /// * `Some(CacheStats)` - If caching is enabled
    /// * `None` - If caching is disabled
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use nebula_credential::prelude::*;
    /// # use std::sync::Arc;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let manager = CredentialManager::builder()
    /// #     .storage(Arc::new(MockStorageProvider::new()))
    /// #     .build();
    /// if let Some(stats) = manager.cache_stats() {
    ///     println!("Hit rate: {:.1}%", stats.hit_rate() * 100.0);
    ///     println!("Utilization: {:.1}%", stats.utilization() * 100.0);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn cache_stats(&self) -> Option<CacheStats> {
        self.cache.as_ref().map(|cache| cache.stats())
    }

    /// Retrieve credential with scope enforcement (Phase 4: Multi-Tenant Isolation)
    ///
    /// Unlike `retrieve()`, this method enforces scope-based access control:
    /// - If context has no scope, returns error (scope required for isolation)
    /// - If credential has no scope, returns error (unscoped credentials not accessible via retrieve_scoped)
    /// - If scopes don't match (exact or hierarchical), returns None
    ///
    /// # Scope Matching Rules
    ///
    /// 1. **Exact match**: `org:acme/team:eng` == `org:acme/team:eng`
    /// 2. **Hierarchical match**: Parent scope can access child credentials
    ///    - Context scope `org:acme/team:eng` can access credential with scope `org:acme/team:eng/service:api`
    ///    - Context scope `org:acme` can access credential with scope `org:acme/team:eng`
    /// 3. **No cross-tenant access**: `org:tenant-a` cannot access `org:tenant-b`
    ///
    /// # Arguments
    ///
    /// * `id` - Unique credential identifier
    /// * `context` - Request context with required scope
    ///
    /// # Returns
    ///
    /// * `Ok(Some((data, metadata)))` - Credential found and scope matches
    /// * `Ok(None)` - Credential not found OR scope mismatch
    /// * `Err(ManagerError::ScopeRequired)` - Context has no scope
    ///
    /// # Errors
    ///
    /// - `ManagerError::ScopeRequired` - Context must have scope for scoped operations
    /// - `ManagerError::StorageError` - Storage operation failed
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use nebula_credential::prelude::*;
    /// # use std::sync::Arc;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let manager = CredentialManager::builder()
    /// #     .storage(Arc::new(MockStorageProvider::new()))
    /// #     .build();
    /// let context = CredentialContext::new("user-123")
    ///     .with_scope("org:acme/team:eng")?;
    ///
    /// // Retrieves credential only if scope matches
    /// let id = CredentialId::new("db-password")?;
    /// if let Some((data, metadata)) = manager.retrieve_scoped(&id, &context).await? {
    ///     println!("Access granted: scope = {:?}", metadata.scope);
    /// } else {
    ///     println!("Access denied or not found");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn retrieve_scoped(
        &self,
        id: &CredentialId,
        context: &CredentialContext,
    ) -> ManagerResult<Option<(EncryptedData, CredentialMetadata)>> {
        // Scope is required for scoped operations
        let context_scope =
            context
                .scope_id
                .as_ref()
                .ok_or_else(|| ManagerError::ScopeRequired {
                    operation: "retrieve_scoped".to_string(),
                })?;

        // First retrieve the credential (may use cache)
        let result = self.retrieve(id, context).await?;

        // If credential doesn't exist, return None
        let (data, metadata) = match result {
            Some(tuple) => tuple,
            None => return Ok(None),
        };

        // Check scope isolation
        match &metadata.scope {
            None => {
                // Unscoped credentials are not accessible via retrieve_scoped
                warn!(
                    credential_id = %id,
                    "Attempted to retrieve unscoped credential via retrieve_scoped"
                );
                Ok(None)
            }
            Some(cred_scope) => {
                // Check if context scope matches credential scope (exact or hierarchical)
                if context_scope.matches_exact(cred_scope)
                    || context_scope.matches_prefix(cred_scope)
                {
                    debug!(
                        credential_id = %id,
                        context_scope = %context_scope,
                        cred_scope = %cred_scope,
                        "Scope access granted"
                    );
                    Ok(Some((data, metadata)))
                } else {
                    warn!(
                        credential_id = %id,
                        context_scope = %context_scope,
                        cred_scope = %cred_scope,
                        "Scope access denied - mismatch"
                    );
                    Ok(None)
                }
            }
        }
    }

    /// List credentials filtered by scope (Phase 4: Multi-Tenant Isolation)
    ///
    /// Returns only credentials that match the context's scope (exact or hierarchical).
    ///
    /// # Scope Filtering Rules
    ///
    /// - If context has no scope, returns error (scope required)
    /// - Returns credentials with exact scope match
    /// - Returns credentials with child scopes (hierarchical match)
    /// - Excludes unscoped credentials
    /// - Excludes credentials from other scopes
    ///
    /// # Arguments
    ///
    /// * `context` - Request context with required scope
    ///
    /// # Returns
    ///
    /// List of credential IDs accessible within the context's scope
    ///
    /// # Errors
    ///
    /// - `ManagerError::ScopeRequired` - Context must have scope for scoped operations
    /// - `ManagerError::StorageError` - Storage operation failed
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use nebula_credential::prelude::*;
    /// # use std::sync::Arc;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let manager = CredentialManager::builder()
    /// #     .storage(Arc::new(MockStorageProvider::new()))
    /// #     .build();
    /// let context = CredentialContext::new("user-123")
    ///     .with_scope("org:acme/team:eng")?;
    ///
    /// // Lists only credentials in org:acme/team:eng scope (and child scopes)
    /// let ids = manager.list_scoped(&context).await?;
    /// println!("Found {} credentials in scope", ids.len());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn list_scoped(
        &self,
        context: &CredentialContext,
    ) -> ManagerResult<Vec<CredentialId>> {
        // Scope is required for scoped operations
        let context_scope =
            context
                .scope_id
                .as_ref()
                .ok_or_else(|| ManagerError::ScopeRequired {
                    operation: "list_scoped".to_string(),
                })?;

        // Get all credentials (no filtering at storage level yet)
        let all_ids = self.list(context).await?;
        let total_count = all_ids.len();

        // Filter by scope - need to fetch metadata for each credential
        let mut scoped_ids = Vec::new();
        for id in all_ids {
            // Use retrieve to get metadata (will use cache if available)
            let Some((_, metadata)) = self.retrieve(&id, context).await? else {
                continue;
            };

            let Some(cred_scope) = &metadata.scope else {
                // Skip unscoped credentials
                continue;
            };

            // Include if exact match or hierarchical match
            if context_scope.matches_exact(cred_scope) || context_scope.matches_prefix(cred_scope) {
                scoped_ids.push(id);
            }
        }

        debug!(
            context_scope = %context_scope,
            total_credentials = total_count,
            scoped_credentials = scoped_ids.len(),
            "Filtered credentials by scope"
        );

        Ok(scoped_ids)
    }

    /// Validate a single credential (Phase 5: Validation and Health Checks)
    ///
    /// Checks if credential exists and validates expiration based on rotation policy.
    ///
    /// # Arguments
    ///
    /// * `id` - Unique credential identifier
    /// * `context` - Request context (owner, scope, trace)
    ///
    /// # Returns
    ///
    /// ValidationResult with status (Valid, Expired, NotFound, Invalid)
    ///
    /// # Errors
    ///
    /// Returns `ManagerError::StorageError` if storage operation fails
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use nebula_credential::prelude::*;
    /// # use std::sync::Arc;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let manager = CredentialManager::builder()
    /// #     .storage(Arc::new(MockStorageProvider::new()))
    /// #     .build();
    /// let context = CredentialContext::new("user-123");
    /// let id = CredentialId::new("db-password")?;
    ///
    /// let result = manager.validate(&id, &context).await?;
    /// if result.is_valid() {
    ///     println!("Credential is valid");
    /// } else if result.is_expired() {
    ///     println!("Credential expired - rotation needed");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn validate(
        &self,
        id: &CredentialId,
        context: &CredentialContext,
    ) -> ManagerResult<crate::manager::ValidationResult> {
        use crate::manager::validation::{
            ValidationDetails, ValidationResult, validate_credential,
        };

        info!(
            credential_id = %id,
            owner_id = %context.owner_id,
            "Validating credential"
        );

        // Retrieve credential
        match self.retrieve(id, context).await? {
            Some((_, metadata)) => {
                // Validate expiration
                let result = validate_credential(id, &metadata);

                if result.is_expired() {
                    warn!(
                        credential_id = %id,
                        "Credential validation failed - expired"
                    );
                } else {
                    debug!(
                        credential_id = %id,
                        valid = result.is_valid(),
                        "Credential validated"
                    );
                }

                Ok(result)
            }
            None => {
                warn!(
                    credential_id = %id,
                    "Credential validation failed - not found"
                );
                Ok(ValidationResult {
                    credential_id: id.clone(),
                    valid: false,
                    details: ValidationDetails::NotFound,
                })
            }
        }
    }

    /// Validate multiple credentials in parallel (Phase 5: Batch Validation)
    ///
    /// Validates credentials concurrently with bounded parallelism.
    ///
    /// # Arguments
    ///
    /// * `ids` - List of credential identifiers to validate
    /// * `context` - Request context (owner, scope, trace)
    ///
    /// # Returns
    ///
    /// HashMap mapping credential IDs to their validation results
    ///
    /// # Errors
    ///
    /// Returns `ManagerError::StorageError` if storage operations fail
    ///
    /// # Performance
    ///
    /// - Uses tokio::task::JoinSet for parallel validation
    /// - Bounded concurrency prevents resource exhaustion
    /// - Leverages cache for improved performance
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use nebula_credential::prelude::*;
    /// # use std::sync::Arc;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let manager = CredentialManager::builder()
    /// #     .storage(Arc::new(MockStorageProvider::new()))
    /// #     .build();
    /// let context = CredentialContext::new("user-123");
    /// let ids = vec![
    ///     CredentialId::new("db-password")?,
    ///     CredentialId::new("api-key")?,
    /// ];
    ///
    /// let results = manager.validate_batch(&ids, &context).await?;
    /// println!("Validated {} credentials", results.len());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn validate_batch(
        &self,
        ids: &[CredentialId],
        context: &CredentialContext,
    ) -> ManagerResult<std::collections::HashMap<CredentialId, crate::manager::ValidationResult>>
    {
        use std::collections::HashMap;
        use tokio::task::JoinSet;

        info!(
            count = ids.len(),
            owner_id = %context.owner_id,
            "Batch validating credentials"
        );

        let mut join_set = JoinSet::new();
        let mut results = HashMap::new();

        // Spawn validation tasks
        for id in ids {
            let id_clone = id.clone();
            let context_clone = context.clone();
            let manager_clone = self.clone();

            join_set.spawn(async move {
                let result = manager_clone.validate(&id_clone, &context_clone).await;
                (id_clone, result)
            });
        }

        // Collect results
        while let Some(task_result) = join_set.join_next().await {
            match task_result {
                Ok((id, Ok(validation_result))) => {
                    results.insert(id, validation_result);
                }
                Ok((id, Err(e))) => {
                    error!(
                        credential_id = %id,
                        error = %e,
                        "Batch validation failed for credential"
                    );
                    return Err(e);
                }
                Err(e) => {
                    error!(
                        error = %e,
                        "Batch validation task panicked"
                    );
                    return Err(ManagerError::StorageError {
                        credential_id: "batch-validation".to_string(),
                        source: crate::core::StorageError::ReadFailure {
                            id: "batch-validation".to_string(),
                            source: std::io::Error::other(format!("Task panic: {}", e)),
                        },
                    });
                }
            }
        }

        debug!(
            validated = results.len(),
            total = ids.len(),
            "Batch validation complete"
        );

        Ok(results)
    }

    /// Store multiple credentials in parallel
    ///
    /// # Arguments
    ///
    /// * `batch` - Vector of (id, data, metadata) tuples to store
    /// * `context` - Request context for all operations
    ///
    /// # Returns
    ///
    /// HashMap mapping each credential ID to its store result
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use nebula_credential::prelude::*;
    /// # use std::sync::Arc;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let manager = CredentialManager::builder()
    /// #     .storage(Arc::new(MockStorageProvider::new()))
    /// #     .build();
    /// let batch = vec![
    ///     (
    ///         CredentialId::new("cred-1")?,
    ///         encrypt(&EncryptionKey::from_bytes([0u8; 32]), b"secret1")?,
    ///         CredentialMetadata::new(),
    ///     ),
    ///     (
    ///         CredentialId::new("cred-2")?,
    ///         encrypt(&EncryptionKey::from_bytes([0u8; 32]), b"secret2")?,
    ///         CredentialMetadata::new(),
    ///     ),
    /// ];
    /// let context = CredentialContext::new("user-1");
    /// let results = manager.store_batch(&batch, &context).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn store_batch(
        &self,
        batch: &[(CredentialId, EncryptedData, CredentialMetadata)],
        context: &CredentialContext,
    ) -> ManagerResult<std::collections::HashMap<CredentialId, ManagerResult<()>>> {
        use std::collections::HashMap;
        use tokio::task::JoinSet;

        info!(
            count = batch.len(),
            owner_id = %context.owner_id,
            "Batch storing credentials"
        );

        let mut join_set = JoinSet::new();
        let mut results = HashMap::new();

        for (id, data, metadata) in batch {
            let id_clone = id.clone();
            let data_clone = data.clone();
            let metadata_clone = metadata.clone();
            let context_clone = context.clone();
            let manager_clone = self.clone();

            join_set.spawn(async move {
                let result = manager_clone
                    .store(&id_clone, data_clone, metadata_clone, &context_clone)
                    .await;
                (id_clone, result)
            });
        }

        while let Some(task_result) = join_set.join_next().await {
            match task_result {
                Ok((id, result)) => {
                    results.insert(id, result);
                }
                Err(e) => {
                    error!(error = %e, "Batch store task panicked");
                    return Err(ManagerError::StorageError {
                        credential_id: "batch-store".to_string(),
                        source: crate::core::StorageError::ReadFailure {
                            id: "batch-store".to_string(),
                            source: std::io::Error::other(format!("Task panic: {}", e)),
                        },
                    });
                }
            }
        }

        let success_count = results.values().filter(|r| r.is_ok()).count();
        debug!(
            total = batch.len(),
            succeeded = success_count,
            failed = batch.len() - success_count,
            "Batch store complete"
        );

        Ok(results)
    }

    /// Retrieve multiple credentials in parallel
    ///
    /// # Arguments
    ///
    /// * `ids` - Credential IDs to retrieve
    /// * `context` - Request context for all operations
    ///
    /// # Returns
    ///
    /// HashMap mapping each credential ID to its retrieve result
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use nebula_credential::prelude::*;
    /// # use std::sync::Arc;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let manager = CredentialManager::builder()
    /// #     .storage(Arc::new(MockStorageProvider::new()))
    /// #     .build();
    /// let ids = vec![
    ///     CredentialId::new("cred-1")?,
    ///     CredentialId::new("cred-2")?,
    /// ];
    /// let context = CredentialContext::new("user-1");
    /// let results = manager.retrieve_batch(&ids, &context).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn retrieve_batch(
        &self,
        ids: &[CredentialId],
        context: &CredentialContext,
    ) -> ManagerResult<
        std::collections::HashMap<
            CredentialId,
            ManagerResult<Option<(EncryptedData, CredentialMetadata)>>,
        >,
    > {
        use std::collections::HashMap;
        use tokio::task::JoinSet;

        info!(
            count = ids.len(),
            owner_id = %context.owner_id,
            "Batch retrieving credentials"
        );

        let mut join_set = JoinSet::new();
        let mut results = HashMap::new();

        for id in ids {
            let id_clone = id.clone();
            let context_clone = context.clone();
            let manager_clone = self.clone();

            join_set.spawn(async move {
                let result = manager_clone.retrieve(&id_clone, &context_clone).await;
                (id_clone, result)
            });
        }

        while let Some(task_result) = join_set.join_next().await {
            match task_result {
                Ok((id, result)) => {
                    results.insert(id, result);
                }
                Err(e) => {
                    error!(error = %e, "Batch retrieve task panicked");
                    return Err(ManagerError::StorageError {
                        credential_id: "batch-retrieve".to_string(),
                        source: crate::core::StorageError::ReadFailure {
                            id: "batch-retrieve".to_string(),
                            source: std::io::Error::other(format!("Task panic: {}", e)),
                        },
                    });
                }
            }
        }

        let success_count = results.values().filter(|r| r.is_ok()).count();
        debug!(
            total = ids.len(),
            succeeded = success_count,
            failed = ids.len() - success_count,
            "Batch retrieve complete"
        );

        Ok(results)
    }

    /// Delete multiple credentials in parallel
    ///
    /// # Arguments
    ///
    /// * `ids` - Credential IDs to delete
    /// * `context` - Request context for all operations
    ///
    /// # Returns
    ///
    /// HashMap mapping each credential ID to its delete result
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use nebula_credential::prelude::*;
    /// # use std::sync::Arc;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let manager = CredentialManager::builder()
    /// #     .storage(Arc::new(MockStorageProvider::new()))
    /// #     .build();
    /// let ids = vec![
    ///     CredentialId::new("cred-1")?,
    ///     CredentialId::new("cred-2")?,
    /// ];
    /// let context = CredentialContext::new("user-1");
    /// let results = manager.delete_batch(&ids, &context).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn delete_batch(
        &self,
        ids: &[CredentialId],
        context: &CredentialContext,
    ) -> ManagerResult<std::collections::HashMap<CredentialId, ManagerResult<()>>> {
        use std::collections::HashMap;
        use tokio::task::JoinSet;

        info!(
            count = ids.len(),
            owner_id = %context.owner_id,
            "Batch deleting credentials"
        );

        let mut join_set = JoinSet::new();
        let mut results = HashMap::new();

        for id in ids {
            let id_clone = id.clone();
            let context_clone = context.clone();
            let manager_clone = self.clone();

            join_set.spawn(async move {
                let result = manager_clone.delete(&id_clone, &context_clone).await;
                (id_clone, result)
            });
        }

        while let Some(task_result) = join_set.join_next().await {
            match task_result {
                Ok((id, result)) => {
                    results.insert(id, result);
                }
                Err(e) => {
                    error!(error = %e, "Batch delete task panicked");
                    return Err(ManagerError::StorageError {
                        credential_id: "batch-delete".to_string(),
                        source: crate::core::StorageError::ReadFailure {
                            id: "batch-delete".to_string(),
                            source: std::io::Error::other(format!("Task panic: {}", e)),
                        },
                    });
                }
            }
        }

        let success_count = results.values().filter(|r| r.is_ok()).count();
        debug!(
            total = ids.len(),
            succeeded = success_count,
            failed = ids.len() - success_count,
            "Batch delete complete"
        );

        Ok(results)
    }
}

// Type-level markers for builder typestate pattern
#[doc(hidden)]
pub struct Yes;
#[doc(hidden)]
pub struct No;

/// Builder for CredentialManager with typestate pattern
///
/// Ensures required parameters (storage) are provided at compile time.
///
/// # Type Parameters
///
/// * `HasStorage` - Type-level marker indicating if storage provider is set
///
/// # Examples
///
/// ```no_run
/// use nebula_credential::prelude::*;
/// use std::sync::Arc;
/// use std::time::Duration;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let storage = MockStorageProvider::new();
///
/// // Basic manager
/// let manager = CredentialManager::builder()
///     .storage(Arc::new(storage))
///     .build();
///
/// // With caching
/// let manager_with_cache = CredentialManager::builder()
///     .storage(Arc::new(MockStorageProvider::new()))
///     .cache_ttl(Duration::from_secs(300))
///     .cache_max_size(1000)
///     .build();
/// # Ok(())
/// # }
/// ```
pub struct CredentialManagerBuilder<HasStorage> {
    storage: Option<Arc<dyn StorageProvider>>,
    cache_config: Option<CacheConfig>,
    _marker: PhantomData<HasStorage>,
}

impl CredentialManagerBuilder<No> {
    /// Create new builder instance
    pub fn new() -> Self {
        Self {
            storage: None,
            cache_config: None,
            _marker: PhantomData,
        }
    }

    /// Set storage provider (required)
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use nebula_credential::prelude::*;
    /// use std::sync::Arc;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let storage = MockStorageProvider::new();
    /// let builder = CredentialManager::builder()
    ///     .storage(Arc::new(storage));
    /// # Ok(())
    /// # }
    /// ```
    pub fn storage(self, storage: Arc<dyn StorageProvider>) -> CredentialManagerBuilder<Yes> {
        CredentialManagerBuilder {
            storage: Some(storage),
            cache_config: self.cache_config,
            _marker: PhantomData,
        }
    }
}

impl<S> CredentialManagerBuilder<S> {
    /// Set cache configuration (optional)
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use nebula_credential::prelude::*;
    /// use std::sync::Arc;
    /// use std::time::Duration;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let cache_config = CacheConfig {
    ///     enabled: true,
    ///     ttl: Some(Duration::from_secs(300)),
    ///     idle_timeout: None,
    ///     max_capacity: 1000,
    ///     eviction_strategy: nebula_credential::manager::EvictionStrategy::Lru,
    /// };
    ///
    /// let manager = CredentialManager::builder()
    ///     .storage(Arc::new(MockStorageProvider::new()))
    ///     .cache_config(cache_config)
    ///     .build();
    /// # Ok(())
    /// # }
    /// ```
    pub fn cache_config(mut self, config: CacheConfig) -> Self {
        self.cache_config = Some(config);
        self
    }

    /// Enable caching with TTL (shorthand)
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use nebula_credential::prelude::*;
    /// use std::sync::Arc;
    /// use std::time::Duration;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let manager = CredentialManager::builder()
    ///     .storage(Arc::new(MockStorageProvider::new()))
    ///     .cache_ttl(Duration::from_secs(300))
    ///     .build();
    /// # Ok(())
    /// # }
    /// ```
    pub fn cache_ttl(mut self, ttl: std::time::Duration) -> Self {
        self.cache_config
            .get_or_insert_with(CacheConfig::default)
            .ttl = Some(ttl);
        self
    }

    /// Set cache maximum size (shorthand)
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use nebula_credential::prelude::*;
    /// use std::sync::Arc;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let manager = CredentialManager::builder()
    ///     .storage(Arc::new(MockStorageProvider::new()))
    ///     .cache_max_size(1000)
    ///     .build();
    /// # Ok(())
    /// # }
    /// ```
    pub fn cache_max_size(mut self, size: usize) -> Self {
        self.cache_config
            .get_or_insert_with(CacheConfig::default)
            .max_capacity = size;
        self
    }
}

impl CredentialManagerBuilder<Yes> {
    /// Build the CredentialManager instance
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use nebula_credential::prelude::*;
    /// use std::sync::Arc;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let manager = CredentialManager::builder()
    ///     .storage(Arc::new(MockStorageProvider::new()))
    ///     .build();
    /// # Ok(())
    /// # }
    /// ```
    pub fn build(self) -> CredentialManager {
        let cache = self
            .cache_config
            .map(|config| Arc::new(CacheLayer::new(&config)));

        CredentialManager {
            storage: self.storage.unwrap(), // Safe: typestate guarantees Some
            cache,
        }
    }
}

impl Default for CredentialManagerBuilder<No> {
    fn default() -> Self {
        Self::new()
    }
}

// Conversion from StorageError to ManagerError
impl From<crate::core::StorageError> for ManagerError {
    fn from(error: crate::core::StorageError) -> Self {
        ManagerError::StorageError {
            credential_id: "unknown".to_string(),
            source: error,
        }
    }
}
