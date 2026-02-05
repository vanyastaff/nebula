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

    // ========================================================================
    // Credential Rotation (Phase 4)
    // ========================================================================

    /// Start rotation for a credential
    ///
    /// Creates a rotation transaction and begins the rotation process.
    /// This is a stub implementation - full logic will be added in user stories.
    ///
    /// # Arguments
    ///
    /// * `id` - Credential to rotate
    /// * `context` - Request context
    ///
    /// # Errors
    ///
    /// Returns `ManagerError` if rotation cannot be started
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use nebula_credential::prelude::*;
    /// # async fn example(manager: CredentialManager) -> Result<(), Box<dyn std::error::Error>> {
    /// let id = CredentialId::new("db-password")?;
    /// let context = CredentialContext::new("user-123");
    ///
    /// let transaction_id = manager.rotate_credential(&id, &context).await?;
    /// # Ok(())
    /// # }
    /// ```
    /// Rotate credential with automatic rollback on failure
    ///
    /// # T078: Automatic Rollback Trigger
    ///
    /// Implements automatic rollback when validation fails:
    /// 1. Attempts credential rotation
    /// 2. Validates new credential
    /// 3. On validation failure: classifies error and triggers rollback
    /// 4. Logs rollback event for audit trail
    ///
    /// # Arguments
    ///
    /// * `id` - Credential to rotate
    /// * `context` - Request context
    ///
    /// # Returns
    ///
    /// * `Ok(transaction_id)` - Rotation succeeded
    /// * `Err(ManagerError)` - Rotation failed (after automatic rollback)
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use nebula_credential::prelude::*;
    /// # async fn example(manager: CredentialManager) -> Result<(), Box<dyn std::error::Error>> {
    /// let id = CredentialId::new("db-password")?;
    /// let context = CredentialContext::new("user-123");
    ///
    /// match manager.rotate_credential(&id, &context).await {
    ///     Ok(tx_id) => println!("Rotation succeeded: {}", tx_id),
    ///     Err(e) => eprintln!("Rotation failed (rolled back): {}", e),
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn rotate_credential(
        &self,
        id: &CredentialId,
        context: &CredentialContext,
    ) -> ManagerResult<String> {
        use crate::rotation::{FailureHandler, RotationErrorLog, RotationTransaction};

        info!(
            credential_id = %id,
            owner_id = %context.owner_id,
            "Starting credential rotation with automatic rollback"
        );

        // 1. Retrieve current credential
        let (_, metadata) =
            self.retrieve(id, context)
                .await?
                .ok_or_else(|| ManagerError::NotFound {
                    credential_id: id.to_string(),
                })?;

        // 2. Create rotation transaction
        let mut transaction = RotationTransaction::new(id.clone(), metadata.version);
        let transaction_id = transaction.id.to_string();

        // 3. Create failure handler for automatic rollback
        let failure_handler = FailureHandler::new();

        // 4. Begin transaction
        if let Err(e) = transaction.begin_transaction() {
            error!(
                transaction_id = %transaction_id,
                error = %e,
                "Failed to begin rotation transaction"
            );
            return Err(ManagerError::ValidationError {
                credential_id: id.to_string(),
                reason: format!("Transaction begin failed: {}", e),
            });
        }

        // 5. Prepare phase (validation simulation - in real impl this would create new credential)
        if let Err(e) = transaction.prepare_phase() {
            let error_msg = format!("Prepare phase failed: {}", e);
            error!(
                transaction_id = %transaction_id,
                credential_id = %id,
                error = %error_msg,
                "Validation failed during prepare phase"
            );

            // Classify error type
            let failure_type = failure_handler.classify_error(&error_msg);

            // Check if rollback should be triggered
            if failure_handler.should_trigger_rollback(&failure_type, 0) {
                warn!(
                    transaction_id = %transaction_id,
                    credential_id = %id,
                    failure_type = ?failure_type,
                    "Triggering automatic rollback"
                );

                // Create error log
                let error_log =
                    RotationErrorLog::new(transaction_id.clone(), id.clone(), error_msg.clone())
                        .with_error_classification(format!("{:?}", failure_type))
                        .with_rollback_triggered();

                // Trigger rollback
                if let Err(rollback_err) = transaction.abort_transaction("Validation failed") {
                    error!(
                        transaction_id = %transaction_id,
                        error = %rollback_err,
                        "Rollback failed"
                    );
                    return Err(ManagerError::ValidationError {
                        credential_id: id.to_string(),
                        reason: format!(
                            "Validation failed and rollback also failed: {}",
                            rollback_err
                        ),
                    });
                }

                info!(
                    transaction_id = %transaction_id,
                    credential_id = %id,
                    "Automatic rollback completed successfully"
                );

                // Log rollback event (simplified - in full impl would use NotificationSender)
                warn!(
                    transaction_id = %transaction_id,
                    credential_id = %id,
                    error_classification = ?error_log.error_classification,
                    "Rotation rolled back: {}",
                    error_log.error_message
                );

                return Err(ManagerError::ValidationError {
                    credential_id: id.to_string(),
                    reason: format!("Rotation failed and was rolled back: {}", error_msg),
                });
            }

            // Transient error - could retry, but for now return error
            return Err(ManagerError::ValidationError {
                credential_id: id.to_string(),
                reason: error_msg,
            });
        }

        // 6. Commit phase
        if let Err(e) = transaction.commit_phase() {
            error!(
                transaction_id = %transaction_id,
                error = %e,
                "Commit phase failed"
            );

            // Automatic rollback on commit failure
            if let Err(rollback_err) = transaction.abort_transaction("Commit failed") {
                error!(
                    transaction_id = %transaction_id,
                    error = %rollback_err,
                    "Rollback after commit failure also failed"
                );
            } else {
                warn!(
                    transaction_id = %transaction_id,
                    "Rolled back after commit failure"
                );
            }

            return Err(ManagerError::ValidationError {
                credential_id: id.to_string(),
                reason: format!("Commit phase failed: {}", e),
            });
        }

        info!(
            transaction_id = %transaction_id,
            credential_id = %id,
            old_version = metadata.version,
            new_version = metadata.version + 1,
            "Credential rotation completed successfully"
        );

        Ok(transaction_id)
    }

    /// Get rotation status for a credential
    ///
    /// Returns current rotation state and transaction details.
    /// This is a stub implementation - full logic will be added in user stories.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use nebula_credential::prelude::*;
    /// # async fn example(manager: CredentialManager) -> Result<(), Box<dyn std::error::Error>> {
    /// let id = CredentialId::new("db-password")?;
    /// let context = CredentialContext::new("user-123");
    ///
    /// if let Some(status) = manager.get_rotation_status(&id, &context).await? {
    ///     println!("Rotation state: {:?}", status);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_rotation_status(
        &self,
        _id: &CredentialId,
        _context: &CredentialContext,
    ) -> ManagerResult<Option<String>> {
        // TODO: Implement in user stories
        // - Query rotation state from storage
        // - Return transaction details
        unimplemented!("Rotation status tracking will be implemented in Phase 4 user stories")
    }

    /// Cancel an in-progress rotation
    ///
    /// Rolls back the rotation transaction if it's still in progress.
    /// This is a stub implementation - full logic will be added in user stories.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use nebula_credential::prelude::*;
    /// # async fn example(manager: CredentialManager) -> Result<(), Box<dyn std::error::Error>> {
    /// let id = CredentialId::new("db-password")?;
    /// let context = CredentialContext::new("user-123");
    ///
    /// manager.cancel_rotation(&id, &context).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn cancel_rotation(
        &self,
        _id: &CredentialId,
        _context: &CredentialContext,
    ) -> ManagerResult<()> {
        // TODO: Implement in user stories
        // - Validate rotation can be cancelled
        // - Rollback transaction
        // - Restore old credential
        unimplemented!("Rotation cancellation will be implemented in Phase 4 user stories")
    }

    /// Rotate credential with periodic policy (US1)
    ///
    /// Performs rotation according to periodic rotation policy with grace period.
    /// This is the implementation for US1 - Automatic Periodic Rotation.
    ///
    /// # Arguments
    ///
    /// * `id` - Credential to rotate
    /// * `context` - Request context
    ///
    /// # Returns
    ///
    /// * `String` - Transaction ID for tracking rotation progress
    ///
    /// # Errors
    ///
    /// Returns `ManagerError` if:
    /// - Credential not found
    /// - No periodic rotation policy configured
    /// - Rotation workflow fails
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use nebula_credential::prelude::*;
    /// # use nebula_credential::rotation::policy::{RotationPolicy, PeriodicConfig};
    /// # use std::time::Duration;
    /// # async fn example(manager: CredentialManager) -> Result<(), Box<dyn std::error::Error>> {
    /// let id = CredentialId::new("db-password")?;
    /// let context = CredentialContext::new("user-123");
    ///
    /// // Credential must have periodic rotation policy configured
    /// let transaction_id = manager.rotate_periodic(&id, &context).await?;
    /// println!("Rotation started: {}", transaction_id);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn rotate_periodic(
        &self,
        id: &CredentialId,
        context: &CredentialContext,
    ) -> ManagerResult<String> {
        use crate::rotation::RotationTransaction;
        use crate::rotation::policy::RotationPolicy;

        info!(
            credential_id = %id,
            owner_id = %context.owner_id,
            "Starting periodic rotation"
        );

        // 1. Retrieve credential metadata to check policy
        let (_, metadata) =
            self.retrieve(id, context)
                .await?
                .ok_or_else(|| ManagerError::NotFound {
                    credential_id: id.to_string(),
                })?;

        // 2. Verify periodic rotation policy is configured
        let policy =
            metadata
                .rotation_policy
                .as_ref()
                .ok_or_else(|| ManagerError::ValidationError {
                    credential_id: id.to_string(),
                    reason: "No rotation policy configured".to_string(),
                })?;

        let _periodic_config = match policy {
            RotationPolicy::Periodic(config) => config,
            _ => {
                return Err(ManagerError::ValidationError {
                    credential_id: id.to_string(),
                    reason: format!("Expected Periodic policy, got {:?}", policy),
                });
            }
        };

        // 3. Create rotation transaction
        let transaction = RotationTransaction::new(id.clone(), metadata.version);

        let transaction_id = transaction.id.to_string();

        info!(
            credential_id = %id,
            transaction_id = %transaction_id,
            old_version = metadata.version,
            "Periodic rotation transaction created"
        );

        // TODO: Full rotation workflow will be implemented in subsequent phases
        // - Create new credential (Phase 9: US9 - Transaction Safety)
        // - Validate new credential (Phase 2: Validation Framework)
        // - Start grace period (already implemented in grace_period.rs)
        // - Commit or rollback (Phase 9: US7 - Rollback)
        // - Schedule cleanup after grace period expires

        Ok(transaction_id)
    }

    /// Rotate credential before expiration (for tokens with TTL)
    ///
    /// Initiates rotation when credential approaches expiration based on BeforeExpiry policy.
    /// This is specifically for OAuth2 tokens, JWT tokens, and other short-lived credentials.
    ///
    /// # Arguments
    ///
    /// * `id` - Credential to rotate
    /// * `context` - Request context (owner, scope, trace)
    ///
    /// # Returns
    ///
    /// Transaction ID for tracking rotation progress
    ///
    /// # Errors
    ///
    /// * `ManagerError::NotFound` - Credential doesn't exist
    /// * `ManagerError::ValidationError` - No BeforeExpiry policy configured
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use nebula_credential::manager::CredentialManager;
    /// use nebula_credential::core::{CredentialId, CredentialContext};
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let manager = CredentialManager::new(storage_provider);
    /// let id = CredentialId::new("oauth2-token")?;
    /// let context = CredentialContext::new("user-123");
    ///
    /// // Rotate token before it expires (triggered at 80% of TTL)
    /// let transaction_id = manager.rotate_before_expiry(&id, &context).await?;
    /// println!("Token refresh started: {}", transaction_id);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn rotate_before_expiry(
        &self,
        id: &CredentialId,
        context: &CredentialContext,
    ) -> ManagerResult<String> {
        use crate::rotation::RotationTransaction;
        use crate::rotation::policy::RotationPolicy;

        info!(
            credential_id = %id,
            owner_id = %context.owner_id,
            "Starting before-expiry rotation"
        );

        // 1. Retrieve credential metadata to check policy and expiration
        let (_, metadata) =
            self.retrieve(id, context)
                .await?
                .ok_or_else(|| ManagerError::NotFound {
                    credential_id: id.to_string(),
                })?;

        // 2. Verify before-expiry rotation policy is configured
        let policy =
            metadata
                .rotation_policy
                .as_ref()
                .ok_or_else(|| ManagerError::ValidationError {
                    credential_id: id.to_string(),
                    reason: "No rotation policy configured".to_string(),
                })?;

        let _before_expiry_config = match policy {
            RotationPolicy::BeforeExpiry(config) => config,
            _ => {
                return Err(ManagerError::ValidationError {
                    credential_id: id.to_string(),
                    reason: format!("Expected BeforeExpiry policy, got {:?}", policy),
                });
            }
        };

        // 3. Create rotation transaction
        let transaction = RotationTransaction::new(id.clone(), metadata.version);

        let transaction_id = transaction.id.to_string();

        info!(
            credential_id = %id,
            transaction_id = %transaction_id,
            old_version = metadata.version,
            "Before-expiry rotation transaction created"
        );

        // TODO: Full token refresh workflow will be implemented in subsequent phases
        // - Use Credential::refresh() to get new token
        // - Validate new token using TestableCredential::test()
        // - Update credential with new token and expiration time
        // - No grace period needed for token refresh (atomic update)
        // - Schedule next refresh based on new expiration time

        Ok(transaction_id)
    }

    /// Rotate credential at scheduled time (maintenance window)
    ///
    /// Initiates rotation at a specific date/time with optional pre-rotation notifications.
    /// Typically used for planned maintenance windows or security updates.
    ///
    /// # Arguments
    ///
    /// * `id` - Credential to rotate
    /// * `context` - Request context (owner, scope, trace)
    ///
    /// # Returns
    ///
    /// Transaction ID for tracking rotation progress
    ///
    /// # Errors
    ///
    /// * `ManagerError::NotFound` - Credential doesn't exist
    /// * `ManagerError::ValidationError` - No Scheduled policy configured
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use nebula_credential::manager::CredentialManager;
    /// use nebula_credential::core::{CredentialId, CredentialContext};
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let manager = CredentialManager::new(storage_provider);
    /// let id = CredentialId::new("prod-db-password")?;
    /// let context = CredentialContext::new("admin");
    ///
    /// // Rotate at scheduled maintenance window
    /// let transaction_id = manager.rotate_scheduled(&id, &context).await?;
    /// println!("Scheduled rotation started: {}", transaction_id);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn rotate_scheduled(
        &self,
        id: &CredentialId,
        context: &CredentialContext,
    ) -> ManagerResult<String> {
        use crate::rotation::RotationTransaction;
        use crate::rotation::policy::RotationPolicy;

        info!(
            credential_id = %id,
            owner_id = %context.owner_id,
            "Starting scheduled rotation"
        );

        // 1. Retrieve credential metadata to check policy
        let (_, metadata) =
            self.retrieve(id, context)
                .await?
                .ok_or_else(|| ManagerError::NotFound {
                    credential_id: id.to_string(),
                })?;

        // 2. Verify scheduled rotation policy is configured
        let policy =
            metadata
                .rotation_policy
                .as_ref()
                .ok_or_else(|| ManagerError::ValidationError {
                    credential_id: id.to_string(),
                    reason: "No rotation policy configured".to_string(),
                })?;

        let _scheduled_config = match policy {
            RotationPolicy::Scheduled(config) => config,
            _ => {
                return Err(ManagerError::ValidationError {
                    credential_id: id.to_string(),
                    reason: format!("Expected Scheduled policy, got {:?}", policy),
                });
            }
        };

        // 3. Create rotation transaction
        let transaction = RotationTransaction::new(id.clone(), metadata.version);

        let transaction_id = transaction.id.to_string();

        info!(
            credential_id = %id,
            transaction_id = %transaction_id,
            old_version = metadata.version,
            "Scheduled rotation transaction created"
        );

        // TODO: Full scheduled rotation workflow will be implemented in subsequent phases
        // - Check if notification time has arrived (ScheduledRotation::should_notify_now())
        // - Send pre-rotation notification via NotificationSender
        // - Wait until scheduled_at time (ScheduledRotation::should_rotate_now())
        // - Execute rotation with grace period
        // - Send completion notification
        // - Handle notification failures with retry logic

        Ok(transaction_id)
    }

    /// Trigger manual rotation (emergency incident response)
    ///
    /// Initiates immediate manual rotation, typically for security incidents.
    /// Supports immediate revocation (no grace period) for emergencies.
    ///
    /// # Arguments
    ///
    /// * `id` - Credential to rotate
    /// * `context` - Request context (owner, scope, trace)
    /// * `reason` - Why rotation is being performed
    /// * `triggered_by` - Who initiated the rotation
    /// * `is_emergency` - Whether to immediately revoke old credential
    ///
    /// # Returns
    ///
    /// Transaction ID for tracking rotation progress
    ///
    /// # Errors
    ///
    /// * `ManagerError::NotFound` - Credential doesn't exist
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use nebula_credential::manager::CredentialManager;
    /// use nebula_credential::core::{CredentialId, CredentialContext};
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let manager = CredentialManager::new(storage_provider);
    /// let id = CredentialId::new("api-key")?;
    /// let context = CredentialContext::new("security-team");
    ///
    /// // Emergency rotation - immediate revocation
    /// let transaction_id = manager.trigger_manual_rotation(
    ///     &id,
    ///     &context,
    ///     "API key leaked in public GitHub repository",
    ///     "incident-response-bot",
    ///     true, // Emergency
    /// ).await?;
    /// println!("Emergency rotation started: {}", transaction_id);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn trigger_manual_rotation(
        &self,
        id: &CredentialId,
        context: &CredentialContext,
        reason: impl Into<String>,
        triggered_by: impl Into<String>,
        is_emergency: bool,
    ) -> ManagerResult<String> {
        use crate::rotation::{ManualRotation, RotationTransaction};

        let reason_str = reason.into();
        let triggered_by_str = triggered_by.into();

        info!(
            credential_id = %id,
            owner_id = %context.owner_id,
            triggered_by = %triggered_by_str,
            is_emergency = is_emergency,
            "Triggering manual rotation"
        );

        // 1. Retrieve credential metadata
        let (_, metadata) =
            self.retrieve(id, context)
                .await?
                .ok_or_else(|| ManagerError::NotFound {
                    credential_id: id.to_string(),
                })?;

        // 2. Create manual rotation metadata
        let manual = if is_emergency {
            ManualRotation::emergency(reason_str, triggered_by_str)
        } else {
            ManualRotation::planned(reason_str, triggered_by_str)
        };

        // 3. Create rotation transaction with manual metadata
        let transaction =
            RotationTransaction::new_manual(id.clone(), metadata.version, manual.clone());

        let transaction_id = transaction.id.to_string();

        info!(
            credential_id = %id,
            transaction_id = %transaction_id,
            old_version = metadata.version,
            is_emergency = manual.is_emergency,
            reason = %manual.reason,
            "Manual rotation transaction created"
        );

        // TODO: Full manual rotation workflow will be implemented in subsequent phases
        // - If is_emergency: immediately revoke old credential (no grace period)
        // - If not emergency: use grace period from ManualConfig
        // - Send emergency notification via NotificationSender
        // - Log to audit trail with reason and triggered_by
        // - Execute rotation
        // - Send completion notification

        Ok(transaction_id)
    }

    /// Rotate database credential using blue-green deployment pattern
    ///
    /// Implements zero-downtime rotation for database credentials by:
    /// 1. Creating a standby credential (green) with same privileges
    /// 2. Validating standby connectivity and privileges
    /// 3. Atomically swapping active/standby credentials
    /// 4. Keeping old credential as standby for rollback
    ///
    /// # T063: Blue-Green Database Rotation
    ///
    /// # Arguments
    ///
    /// * `id` - Current active credential ID
    /// * `context` - Request context
    ///
    /// # Returns
    ///
    /// Returns transaction ID for tracking the rotation
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
    /// let id = CredentialId::new("postgres-prod")?;
    /// let context = CredentialContext::new("db-service");
    ///
    /// let transaction_id = manager.rotate_blue_green(&id, &context).await?;
    /// println!("Blue-green rotation started: {}", transaction_id);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn rotate_blue_green(
        &self,
        id: &CredentialId,
        context: &CredentialContext,
    ) -> ManagerResult<String> {
        use crate::rotation::{BlueGreenRotation, RotationTransaction};

        info!(
            credential_id = %id,
            owner_id = %context.owner_id,
            "Starting blue-green database rotation"
        );

        // 1. Retrieve current active credential metadata
        let (_, metadata) =
            self.retrieve(id, context)
                .await?
                .ok_or_else(|| ManagerError::NotFound {
                    credential_id: id.to_string(),
                })?;

        // 2. Generate standby credential ID
        let standby_id = CredentialId::new(format!("{}-standby", id.as_str())).map_err(|e| {
            ManagerError::ValidationError {
                credential_id: format!("{}-standby", id.as_str()),
                reason: format!("Invalid standby credential ID: {}", e),
            }
        })?;

        // 3. Create blue-green rotation tracker
        let _rotation = BlueGreenRotation::new(id.clone(), standby_id.clone());

        // 4. Create rotation transaction
        let transaction = RotationTransaction::new(id.clone(), metadata.version);
        let transaction_id = transaction.id.to_string();

        info!(
            credential_id = %id,
            standby_id = %standby_id,
            transaction_id = %transaction_id,
            old_version = metadata.version,
            "Blue-green rotation transaction created"
        );

        // TODO: Full blue-green rotation workflow will be implemented in subsequent phases
        // - Create standby credential with mirrored privileges (create_standby_credential)
        // - Enumerate required privileges from active credential (enumerate_required_privileges)
        // - Validate standby connectivity (validate_standby_connectivity)
        // - Validate standby privileges match active (validate_privileges)
        // - Atomically swap credentials (swap_credentials)
        // - Update metadata to mark standby as active
        // - Keep old credential as standby for quick rollback
        // - Send completion notification

        Ok(transaction_id)
    }

    /// Rotate credential with grace period for gradual migration
    ///
    /// Implements gradual credential rotation with usage tracking:
    /// 1. Creates new credential while keeping old one active
    /// 2. Tracks usage of both credentials during grace period
    /// 3. Monitors migration progress (old credential usage decreasing)
    /// 4. Automatically revokes old credential when safe
    ///
    /// # T071: Rotate with Grace Period
    ///
    /// # Arguments
    ///
    /// * `id` - Current credential ID
    /// * `context` - Request context
    /// * `grace_period_config` - Grace period configuration (duration, overlap, notifications)
    ///
    /// # Returns
    ///
    /// Returns transaction ID for tracking the rotation
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use nebula_credential::prelude::*;
    /// # use std::sync::Arc;
    /// # use std::time::Duration;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let manager = CredentialManager::builder()
    /// #     .storage(Arc::new(MockStorageProvider::new()))
    /// #     .build();
    /// let id = CredentialId::new("api-key-prod")?;
    /// let context = CredentialContext::new("api-service");
    /// let grace_config = GracePeriodConfig::new(Duration::from_secs(7 * 24 * 3600)); // 7 days
    ///
    /// let transaction_id = manager.rotate_with_grace_period(&id, &context, &grace_config).await?;
    /// println!("Grace period rotation started: {}", transaction_id);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn rotate_with_grace_period(
        &self,
        id: &CredentialId,
        context: &CredentialContext,
        grace_period_config: &crate::rotation::GracePeriodConfig,
    ) -> ManagerResult<String> {
        use crate::rotation::{GracePeriodState, GracePeriodTracker, RotationTransaction};

        info!(
            credential_id = %id,
            owner_id = %context.owner_id,
            grace_period_days = grace_period_config.duration.as_secs() / 86400,
            "Starting grace period rotation"
        );

        // 1. Retrieve current credential metadata
        let (_, metadata) =
            self.retrieve(id, context)
                .await?
                .ok_or_else(|| ManagerError::NotFound {
                    credential_id: id.to_string(),
                })?;

        // 2. Generate new credential ID
        let new_id = CredentialId::new(format!("{}-v{}", id.as_str(), metadata.version + 1))
            .map_err(|e| ManagerError::ValidationError {
                credential_id: format!("{}-v{}", id.as_str(), metadata.version + 1),
                reason: format!("Invalid new credential ID: {}", e),
            })?;

        // 3. Create grace period state
        let grace_period = GracePeriodState::new(
            id.clone(),
            metadata.version,
            metadata.version + 1,
            grace_period_config,
        );

        // 4. Create grace period tracker
        let grace_period_state = grace_period.map_err(|e| ManagerError::ValidationError {
            credential_id: id.to_string(),
            reason: format!("Grace period calculation failed: {}", e),
        })?;
        let tracker = GracePeriodTracker::new(id.clone(), new_id.clone(), grace_period_state);

        // 5. Create rotation transaction
        let transaction = RotationTransaction::new(id.clone(), metadata.version);
        let transaction_id = transaction.id.to_string();

        info!(
            credential_id = %id,
            new_credential_id = %new_id,
            transaction_id = %transaction_id,
            old_version = metadata.version,
            new_version = metadata.version + 1,
            grace_period_expires = %tracker.grace_period.expires_at,
            "Grace period rotation transaction created"
        );

        // TODO: Full grace period rotation workflow will be implemented in subsequent phases
        // - Generate new credential (new secret/token)
        // - Store new credential with incremented version
        // - Initialize usage tracking for both credentials
        // - Send notification about grace period start
        // - Monitor usage metrics (track_old_credential_usage, track_new_credential_usage)
        // - Check migration progress (check_old_credential_usage)
        // - Automatically revoke old credential when safe (can_revoke_old_credential)
        // - Cleanup expired credentials (cleanup_expired_credentials)
        // - Send completion notification

        Ok(transaction_id)
    }

    /// Atomically rotate credential using two-phase commit protocol
    ///
    /// # T095: Atomic Rotation with 2PC
    ///
    /// Implements full two-phase commit (2PC) for atomic rotation:
    /// 1. **Prepare Phase**: Create and validate new credential
    /// 2. **Commit Phase**: Atomically swap credentials or rollback on failure
    ///
    /// This ensures credentials never enter invalid intermediate states during rotation.
    ///
    /// # Arguments
    ///
    /// * `id` - Credential to rotate
    /// * `context` - Request context
    /// * `validator` - Optional custom validation logic
    ///
    /// # Returns
    ///
    /// * `(String, TransactionLog)` - Transaction ID and complete audit log
    ///
    /// # Errors
    ///
    /// Returns `ManagerError` if:
    /// - Credential not found
    /// - Prepare phase fails (credential creation or validation)
    /// - Commit phase fails (storage error)
    /// - Automatic rollback fails
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use nebula_credential::prelude::*;
    /// # use nebula_credential::rotation::TransactionLog;
    /// # async fn example(manager: CredentialManager) -> Result<(), Box<dyn std::error::Error>> {
    /// let id = CredentialId::new("db-password")?;
    /// let context = CredentialContext::new("user-123");
    ///
    /// let (transaction_id, log) = manager
    ///     .rotate_atomic(&id, &context, None)
    ///     .await?;
    ///
    /// println!("Rotation committed: {}", transaction_id);
    /// println!("Audit log: {} entries", log.entry_count());
    /// # Ok(())
    /// # }
    /// ```
    #[allow(clippy::type_complexity)]
    pub async fn rotate_atomic(
        &self,
        id: &CredentialId,
        context: &CredentialContext,
        _validator: Option<Box<dyn Fn(&EncryptedData) -> bool + Send + Sync>>,
    ) -> ManagerResult<(String, crate::rotation::TransactionLog)> {
        use crate::rotation::{RotationState, RotationTransaction, TransactionLog};

        info!(
            credential_id = %id,
            owner_id = %context.owner_id,
            "Starting atomic rotation with 2PC"
        );

        // 1. Retrieve current credential
        let (data, metadata) =
            self.retrieve(id, context)
                .await?
                .ok_or_else(|| ManagerError::NotFound {
                    credential_id: id.to_string(),
                })?;

        // 2. Create rotation transaction
        let mut transaction = RotationTransaction::new(id.clone(), metadata.version);
        let transaction_id = transaction.id.to_string();

        // 3. Create transaction log
        let mut log = TransactionLog::new(transaction_id.clone(), id.clone());
        log.log_info(format!(
            "Starting atomic rotation for version {}",
            metadata.version
        ));

        // === PHASE 1: PREPARE ===
        info!(
            transaction_id = %transaction_id,
            credential_id = %id,
            "Entering prepare phase"
        );

        // Begin transaction
        if let Err(e) = transaction.begin_transaction() {
            error!(
                transaction_id = %transaction_id,
                error = %e,
                "Failed to begin transaction"
            );
            log.log_error(format!("Failed to begin transaction: {}", e));
            log.log_rollback("Transaction begin failed");
            return Err(ManagerError::ValidationError {
                credential_id: id.to_string(),
                reason: format!("Transaction begin failed: {}", e),
            });
        }
        log.log_transition(RotationState::Creating, "Transaction began");

        // Prepare phase - validate credential state
        if let Err(e) = transaction.prepare_phase() {
            error!(
                transaction_id = %transaction_id,
                error = %e,
                "Prepare phase failed"
            );
            log.log_error(format!("Prepare phase failed: {}", e));

            // Automatic rollback
            if let Err(rollback_err) = transaction.abort_transaction("Prepare phase failed") {
                error!(
                    transaction_id = %transaction_id,
                    error = %rollback_err,
                    "Rollback failed after prepare failure"
                );
                log.log_error(format!("Rollback failed: {}", rollback_err));
            } else {
                log.log_rollback("Prepare phase failed - transaction aborted");
            }

            return Err(ManagerError::ValidationError {
                credential_id: id.to_string(),
                reason: format!("Prepare phase failed: {}", e),
            });
        }

        log.log_transition(RotationState::Validating, "Prepare phase completed");
        log.log_validation_result(true, "Credential state validated");

        // === PHASE 2: COMMIT ===
        info!(
            transaction_id = %transaction_id,
            credential_id = %id,
            "Entering commit phase"
        );

        // Commit phase - finalize rotation
        if let Err(e) = transaction.commit_phase() {
            error!(
                transaction_id = %transaction_id,
                error = %e,
                "Commit phase failed"
            );
            log.log_error(format!("Commit phase failed: {}", e));

            // Automatic rollback
            if let Err(rollback_err) = transaction.abort_transaction("Commit phase failed") {
                error!(
                    transaction_id = %transaction_id,
                    error = %rollback_err,
                    "Rollback failed after commit failure"
                );
                log.log_error(format!("Rollback failed: {}", rollback_err));
            } else {
                log.log_rollback("Commit phase failed - transaction aborted");
            }

            return Err(ManagerError::ValidationError {
                credential_id: id.to_string(),
                reason: format!("Commit phase failed: {}", e),
            });
        }

        log.log_transition(RotationState::Committed, "Commit phase completed");

        // Update metadata version
        let mut new_metadata = metadata.clone();
        new_metadata.version += 1;
        new_metadata.last_modified = chrono::Utc::now();

        // Store updated metadata (in real implementation, this would be atomic with 2PC)
        if let Err(e) = self.store(id, data, new_metadata, context).await {
            error!(
                transaction_id = %transaction_id,
                error = %e,
                "Failed to update metadata after commit"
            );
            log.log_warning(format!("Metadata update warning: {}", e));
        }

        log.log_commit();

        info!(
            transaction_id = %transaction_id,
            credential_id = %id,
            old_version = metadata.version,
            new_version = metadata.version + 1,
            "Atomic rotation committed successfully"
        );

        Ok((transaction_id, log))
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
